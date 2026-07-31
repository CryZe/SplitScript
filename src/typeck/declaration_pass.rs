//! Source declaration and signature collection before body checking.

use std::collections::HashSet;

use crate::{
    ast::{Program, SettingKind, StateSource},
    inference::Requirements,
    stdlib::StdlibCapabilityId,
    stdlib_semantic::StandardLibrarySemanticExt,
};

use super::{Checker, control_flow::contains_value_return, declarations::FunctionSignature};

pub(super) fn collect(checker: &mut Checker, program: &Program) {
    collect_state_fields(checker, program);
    collect_settings(checker, program);
    collect_named_type_members(checker, program);
    collect_function_signatures(checker, program);
}

fn collect_state_fields(checker: &mut Checker, program: &Program) {
    let state = program.state.as_ref().unwrap();
    let provider = state
        .provider
        .as_ref()
        .and_then(|provider| provider.resolved)
        .map(|provider| checker.standard_library.state_provider(provider));
    for field in &state.fields {
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
        checker.semantics.resolve_value_type(field.id, ty);
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
        if let StateSource::Pointer(path) = &field.source {
            if path.offsets.is_empty() {
                checker.error("a pointer path needs at least one offset", field.span);
            }
            checker.require(
                ty,
                Requirements::capability(StdlibCapabilityId::MemoryReadable),
                field.span,
            );
            if let Some(provider) = provider {
                if path.module.is_some() {
                    checker.error(
                        format!(
                            "`state {}` direct reads use hardware addresses and cannot name a module",
                            provider.name
                        ),
                        field.span,
                    );
                }
                if path.offsets.len() != 1 {
                    checker.error(
                        format!(
                            "`state {}` direct reads currently require exactly one address",
                            provider.name
                        ),
                        field.span,
                    );
                }
                if path
                    .offsets
                    .first()
                    .is_some_and(|address| *address > u32::MAX.into())
                {
                    checker.error(
                        format!("`state {}` addresses must fit in `u32`", provider.name),
                        field.span,
                    );
                }
            }
        }
    }
}

fn collect_settings(checker: &mut Checker, program: &Program) {
    for setting in &program.settings {
        if let Some(ty) = setting.value_type() {
            let ty = checker.syntax_type(ty);
            checker.semantics.resolve_value_type(setting.id, ty);
            if checker
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
            enumeration,
            default_variant,
            options,
        } = &setting.kind
        else {
            continue;
        };
        let Some(enumeration) = enumeration.source() else {
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
        let result = if let Some(annotation) = function.return_annotation {
            checker.syntax_type(annotation)
        } else if contains_value_return(&function.body) {
            checker.fresh_inference(Requirements::none(), None)
        } else {
            checker.core_type(crate::stdlib::CoreTypeId::Void)
        };
        checker
            .semantics
            .resolve_function_result(function.id, result);
        let signature = FunctionSignature {
            id: function.id,
            params,
            result,
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
