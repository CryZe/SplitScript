//! Source declaration and signature collection before body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{Program, SettingDecl, SettingKind, Span, StateMemoryDecoder, StateSource},
    inference::{Requirements, Type},
    intrinsic_registry::{MAX_NATIVE_STRING_BYTES, MAX_NATIVE_UTF16_UNITS},
    stdlib::{CoreTypeId, StdlibCapabilityId, StdlibTypeId},
    stdlib_semantic::StandardLibrarySemanticExt,
    types::EnumTypeId,
};

use super::{
    Checker,
    control_flow::contains_value_return,
    declarations::{Binding, FunctionSignature, RuntimeSettingDeclaration, RuntimeSettingKind},
};

pub(super) fn collect(checker: &mut Checker, program: &Program) {
    collect_state_fields(checker, program);
    collect_settings(checker, program);
    collect_named_type_members(checker, program);
    collect_function_signatures(checker, program);
}

fn collect_state_fields(checker: &mut Checker, program: &Program) {
    let Some(state) = program.state.as_ref() else {
        return;
    };
    let provider = checker
        .provider_value
        .map(|(provider, _)| checker.standard_library.state_provider(provider));
    if state.layouts.is_empty() {
        for field in &state.fields {
            let ty = collect_state_field_type(checker, field, provider);
            checker.semantics.resolve_value_type(field.id, ty);
            checker.declarations.state_fields_by_id.insert(field.id, ty);
            checker
                .declarations
                .state_storage_fields
                .insert(field.id, field.id);
            if checker
                .declarations
                .state_fields
                .insert(field.name.clone(), (field.id, ty))
                .is_some()
            {
                checker.error(
                    format!("duplicate state field `{}`", field.name),
                    field.span,
                );
            }
        }
    } else {
        // First collect every physical declaration independently. Layouts are
        // allowed to omit names or use the same name with a different type.
        for layout in &state.layouts {
            let mut fields = HashMap::new();
            for field in &layout.fields {
                let ty = collect_state_field_type(checker, field, provider);
                checker.semantics.resolve_value_type(field.id, ty);
                checker.declarations.state_fields_by_id.insert(field.id, ty);
                if fields.insert(field.name.clone(), (field.id, ty)).is_some() {
                    checker.error(
                        format!("duplicate state field `{}` in this layout", field.name),
                        field.span,
                    );
                }
            }
            checker
                .declarations
                .layout_state_fields
                .insert(layout.variant, fields);
        }

        // A name becomes part of StateSnapshot's common interface only when
        // every layout declares it and explicit annotations do not conflict.
        // Unannotated declarations still participate in bidirectional
        // inference by unifying with the canonical declaration.
        let first = &state.layouts[0];
        for field in &first.fields {
            let declarations = state
                .layouts
                .iter()
                .map(|layout| layout.fields.iter().find(|item| item.name == field.name))
                .collect::<Option<Vec<_>>>();
            let is_common = state.is_common_field(&field.name);
            if is_common {
                let canonical_ty = checker.declarations.state_fields_by_id[&field.id];
                checker
                    .declarations
                    .state_fields
                    .insert(field.name.clone(), (field.id, canonical_ty));
                for declaration in declarations.unwrap() {
                    let ty = checker.declarations.state_fields_by_id[&declaration.id];
                    if declaration.id != field.id {
                        let canonical_name = checker.type_name(canonical_ty);
                        checker.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: field.span,
                                label: format!(
                                    "the first layout declares `{}` as `{canonical_name}`",
                                    field.name
                                ),
                            },
                            |checker| {
                                checker.unify_expected(ty, canonical_ty, declaration.span);
                            },
                        );
                    }
                    checker
                        .declarations
                        .state_storage_fields
                        .insert(declaration.id, field.id);
                }
            }
        }

        for layout in &state.layouts {
            for field in &layout.fields {
                checker
                    .declarations
                    .state_storage_fields
                    .entry(field.id)
                    .or_insert(field.id);
            }
        }
    }

    if let (Some(layout_enum), Some(layout_value)) = (&state.layout_enum, state.layout_value) {
        let ty = checker.enum_type(EnumTypeId::Source(layout_enum.id));
        checker.semantics.resolve_value_type(layout_value, ty);
        checker.declarations.globals.insert(
            "layout".to_owned(),
            Binding {
                id: Some(layout_value),
                ty,
                mutable: false,
                debug_only: false,
                declaration_span: None,
            },
        );
    }

    let storage_fields = if state.layouts.is_empty() {
        state.fields.iter().map(|field| field.id).collect()
    } else {
        let mut fields = state.layouts[0]
            .fields
            .iter()
            .filter(|field| {
                checker
                    .declarations
                    .state_fields
                    .get(&field.name)
                    .is_some_and(|(canonical, _)| *canonical == field.id)
            })
            .map(|field| field.id)
            .collect::<Vec<_>>();
        let common = fields.iter().copied().collect::<HashSet<_>>();
        fields.extend(state.all_fields().filter_map(|field| {
            (checker.declarations.state_storage_fields[&field.id] == field.id
                && !common.contains(&field.id))
            .then_some(field.id)
        }));
        fields
    };
    let layout_fields = state
        .layouts
        .iter()
        .map(|layout| {
            (
                layout.variant,
                layout.fields.iter().map(|field| field.id).collect(),
            )
        })
        .collect();
    checker.semantics.resolve_state_layout(
        storage_fields,
        checker.declarations.state_storage_fields.clone(),
        layout_fields,
    );
}

fn collect_state_field_type(
    checker: &mut Checker,
    field: &crate::ast::StateField,
    provider: Option<&crate::stdlib::StdlibStateProvider>,
) -> Type {
    let ty = if let Some(annotation) = field.annotation {
        checker.syntax_type(annotation)
    } else {
        checker.fresh_inference(Requirements::none(), None)
    };
    if let Some(standard) = checker.standard_type_id(ty) {
        let declaration = checker.standard_library.type_decl(standard);
        if !declaration.value_usage.state_field {
            checker.error(
                format!("{} cannot be stored in a state field", declaration.name),
                field.span,
            );
        }
    }
    if let StateSource::Pointer(path) = &field.source {
        // An explicitly optional pointer field observes read failure as
        // `None`. The pointer still reads the contained representation; the
        // outer Option belongs to snapshot semantics rather than process
        // memory layout.
        let memory_ty = if matches!(field.annotation, Some(crate::ast::TypeRef::Option(_))) {
            match ty {
                Type::Option(option) => checker.inference.option_value(option),
                Type::Known(ty) => match checker.inference.type_store().kind(ty) {
                    crate::types::TypeKind::Option { value, .. } => Type::Known(*value),
                    _ => Type::Known(ty),
                },
                _ => ty,
            }
        } else {
            ty
        };
        if let Some(decoder) = path.decoder {
            let string = checker.standard_type(StdlibTypeId::String);
            checker.unify(memory_ty, string, field.span);
            match decoder {
                StateMemoryDecoder::Utf8 { max_bytes, span } => {
                    if max_bytes == 0 {
                        checker.error("a UTF-8 state read must allow at least one byte", span);
                    } else if max_bytes > MAX_NATIVE_STRING_BYTES {
                        checker.error(
                            format!(
                                "a UTF-8 state read is limited to {MAX_NATIVE_STRING_BYTES} bytes"
                            ),
                            span,
                        );
                    }
                }
                StateMemoryDecoder::Utf16Le { max_units, span } => {
                    if max_units == 0 {
                        checker.error(
                            "a UTF-16LE state read must allow at least one code unit",
                            span,
                        );
                    } else if max_units > MAX_NATIVE_UTF16_UNITS {
                        checker.error(
                            format!(
                                "a UTF-16LE state read is limited to {MAX_NATIVE_UTF16_UNITS} code units"
                            ),
                            span,
                        );
                    }
                }
            }
        } else {
            checker.require(
                memory_ty,
                Requirements::capability(StdlibCapabilityId::MemoryReadable),
                field.span,
            );
        }
        if let Some(provider) = provider
            && checker
                .standard_library
                .item(provider.direct_read)
                .signature
                .parameters[0]
                .ty
                == crate::stdlib::TypeRef::Core(CoreTypeId::U32)
        {
            if path.decoder.is_some() {
                checker.error(
                    format!(
                        "`state {}` does not yet support decoded string fields",
                        provider.name
                    ),
                    field.span,
                );
            }
            if matches!(path.base, crate::ast::PointerPathBase::Module { .. }) {
                checker.error(
                    format!(
                        "`state {}` direct reads use hardware addresses and cannot name a module",
                        provider.name
                    ),
                    field.span,
                );
            }
            if !path.offsets.is_empty() {
                checker.error(
                    format!(
                        "`state {}` direct reads currently require exactly one address",
                        provider.name
                    ),
                    field.span,
                );
            }
            if !matches!(path.base, crate::ast::PointerPathBase::Absolute(address) if address <= u32::MAX.into())
            {
                checker.error(
                    format!("`state {}` addresses must fit in `u32`", provider.name),
                    field.span,
                );
            }
        }
    }
    ty
}

fn collect_settings(checker: &mut Checker, program: &Program) {
    let mut runtime_keys = HashMap::<String, Span>::new();
    for family in &program.setting_families {
        checker
            .semantics
            .resolve_value_type(family.binding_id, checker.core_type(CoreTypeId::U32));
    }
    for setting in &program.settings {
        let runtime_key = setting.runtime_key();
        let key_span = setting
            .external_key
            .as_ref()
            .map_or(setting.span, crate::ast::SettingExternalKey::span);
        if runtime_key.is_empty() {
            checker.error("a setting key cannot be empty", key_span);
        } else if let Some(first_span) = runtime_keys.insert(runtime_key.to_owned(), key_span) {
            checker.errors.push(
                crate::Diagnostic::type_error(
                    format!("duplicate runtime setting key `{runtime_key}`"),
                    key_span,
                )
                .with_primary_label("this key is declared again here")
                .with_secondary_label(first_span, "the first declaration is here"),
            );
        } else {
            let kind = match setting.kind {
                SettingKind::Bool { .. } => RuntimeSettingKind::Bool,
                SettingKind::Choice { .. } => RuntimeSettingKind::Choice,
                SettingKind::File { .. } => RuntimeSettingKind::File,
                SettingKind::Title { .. } => RuntimeSettingKind::Title,
            };
            checker.declarations.settings_by_runtime_key.insert(
                runtime_key.to_owned(),
                RuntimeSettingDeclaration {
                    source_name: (setting.source_visible && kind != RuntimeSettingKind::Title)
                        .then(|| setting.name.clone()),
                    kind,
                    span: key_span,
                },
            );
        }
        if let Some(ty) = setting_value_type(checker, setting) {
            checker.semantics.resolve_value_type(setting.id, ty);
            if setting.source_visible
                && checker
                    .declarations
                    .settings
                    .insert(setting.name.clone(), (setting.id, ty))
                    .is_some()
            {
                checker.error(
                    format!("duplicate setting `{}`", setting.name),
                    setting.span,
                );
            }
        }
        let SettingKind::Choice {
            default_variant,
            options,
            ..
        } = &setting.kind
        else {
            continue;
        };
        let Some(EnumTypeId::Source(enumeration)) = checker.resolutions.setting_enum(setting.id)
        else {
            checker.error("unresolved enum used by choice setting", setting.span);
            continue;
        };
        let declaration = checker
            .declarations
            .enums
            .iter()
            .find(|item| item.id == enumeration)
            .cloned();
        let Some(declaration) = declaration else {
            checker.error("unknown enum used by choice setting", setting.span);
            continue;
        };
        let mut seen = HashSet::new();
        for option in options {
            let Some(variant) = declaration
                .variants
                .iter()
                .find(|variant| variant.name == option.variant)
            else {
                checker.error(
                    format!(
                        "enum `{}` has no variant `{}`",
                        declaration.name, option.variant
                    ),
                    option.span,
                );
                continue;
            };
            if variant.payload.is_some() {
                checker.error("choice variants cannot have payloads", option.span);
            }
            checker
                .semantics
                .resolve_setting_choice_option(option.id, variant.id);
            if !seen.insert(option.variant.clone()) {
                checker.error(
                    format!("duplicate choice option `{}`", option.variant),
                    option.span,
                );
            }
        }
        if !seen.contains(default_variant) {
            checker.error(
                "the default choice must be one of its options",
                setting.span,
            );
        } else if let Some(variant) = declaration
            .variants
            .iter()
            .find(|variant| variant.name == *default_variant)
        {
            checker
                .semantics
                .resolve_setting_choice_default(setting.id, variant.id);
        }
    }
}

fn setting_value_type(checker: &Checker, setting: &SettingDecl) -> Option<Type> {
    match &setting.kind {
        SettingKind::Bool { .. } => Some(checker.core_type(CoreTypeId::Bool)),
        SettingKind::Choice { .. } => {
            Some(checker.enum_type(checker.resolutions.setting_enum(setting.id)?))
        }
        SettingKind::File { .. } => Some(checker.standard_type(StdlibTypeId::String)),
        SettingKind::Title { .. } => None,
    }
}

fn collect_named_type_members(checker: &mut Checker, program: &Program) {
    let mut record_names = HashSet::new();
    for record in &program.records {
        if !record_names.insert(record.name.clone()) {
            checker.error(format!("duplicate record `{}`", record.name), record.span);
        }
        let mut fields = HashSet::new();
        for field in &record.fields {
            let field_ty = checker.syntax_type(field.ty);
            checker
                .semantics
                .resolve_record_field_type(field.id, field_ty);
            if !fields.insert(field.name.clone()) {
                checker.error(
                    format!(
                        "duplicate field `{}` in record `{}`",
                        field.name, record.name
                    ),
                    field.span,
                );
            }
            if let Some(standard) = checker.standard_type_id(field_ty) {
                let declaration = checker.standard_library.type_decl(standard);
                if !declaration.value_usage.record_field {
                    checker.error(
                        format!("{} cannot be stored in a record field", declaration.name),
                        field.span,
                    );
                }
            }
        }
    }

    let mut enum_names = HashSet::new();
    let enum_declarations = checker.declarations.enums.clone();
    for enumeration in &enum_declarations {
        if !enum_names.insert(enumeration.name.clone()) || record_names.contains(&enumeration.name)
        {
            checker.error(
                format!("duplicate named type `{}`", enumeration.name),
                enumeration.span,
            );
        }
        let mut variants = HashSet::new();
        for variant in &enumeration.variants {
            let payload = variant.payload.map(|ty| checker.syntax_type(ty));
            checker
                .semantics
                .resolve_enum_variant_payload(variant.id, payload);
            if !variants.insert(variant.name.clone()) {
                checker.error(
                    format!(
                        "duplicate variant `{}` in enum `{}`",
                        variant.name, enumeration.name
                    ),
                    variant.span,
                );
            }
            if let Some(standard) = payload.and_then(|ty| checker.standard_type_id(ty))
                && !checker
                    .standard_library
                    .type_decl(standard)
                    .value_usage
                    .enum_payload
            {
                checker.error(
                    "enum payloads cannot store this standard-library type",
                    variant.span,
                );
            }
        }
        if enumeration.variants.is_empty() {
            checker.error("an enum needs at least one variant", enumeration.span);
        }
    }
}

fn collect_function_signatures(checker: &mut Checker, program: &Program) {
    for function in &program.functions {
        let params = function
            .params
            .iter()
            .map(|parameter| {
                let ty = if let Some(annotation) = parameter.annotation {
                    checker.syntax_type(annotation)
                } else {
                    checker.fresh_inference(Requirements::none(), None)
                };
                checker.semantics.resolve_value_type(parameter.id, ty);
                ty
            })
            .collect::<Vec<_>>();
        let annotated = function
            .return_annotation
            .map(|annotation| checker.syntax_type(annotation));
        let completion = if let Some(Type::Async(future)) = annotated {
            checker.inference.async_value(future)
        } else if let Some(annotation) = annotated {
            annotation
        } else if contains_value_return(&function.body) {
            checker.fresh_inference(Requirements::none(), None)
        } else {
            checker.core_type(crate::stdlib::CoreTypeId::None)
        };
        let is_async = function.return_is_async
            || crate::typeck::control_flow::contains_suspension(&function.body);
        let result = if is_async {
            match annotated {
                Some(result @ Type::Async(_)) => result,
                _ => Type::Async(checker.inference.async_type(completion)),
            }
        } else if let Some(annotation) = annotated {
            annotation
        } else {
            completion
        };
        checker
            .semantics
            .resolve_function_result(function.id, result);
        checker
            .semantics
            .resolve_function_completion(function.id, completion);
        let signature = FunctionSignature {
            id: function.id,
            params,
            parameter_declarations: function
                .params
                .iter()
                .map(
                    |parameter| super::declarations::FunctionParameterDeclaration {
                        name: parameter.name.clone(),
                        span: parameter.span,
                    },
                )
                .collect(),
            result,
            completion,
            generalized: Vec::new(),
        };
        checker
            .declarations
            .function_signatures
            .insert(function.id, signature.clone());
        if let Some(receiver) = function.method_of {
            let key = (checker.syntax_type(receiver), function.name.clone());
            if checker
                .declarations
                .methods
                .insert(key, signature)
                .is_some()
            {
                checker.error(
                    format!("duplicate method `{}` for `{receiver}`", function.name),
                    function.span,
                );
            }
            continue;
        }
        if function.name == "Err"
            || !checker
                .standard_library
                .function_candidates(std::slice::from_ref(&function.name))
                .is_empty()
            || checker.declarations.functions.contains_key(&function.name)
        {
            checker.error(
                format!("duplicate or reserved function name `{}`", function.name),
                function.span,
            );
            continue;
        }
        checker
            .declarations
            .functions
            .insert(function.name.clone(), signature);
    }
}
