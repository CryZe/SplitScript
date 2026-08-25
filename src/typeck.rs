use std::collections::{HashMap, HashSet};

mod body_pass;
mod call_resolution;
mod context;
mod control_flow;
mod declaration_pass;
mod declarations;
mod driver;
mod expressions;
mod finalization;
mod function_graph;
mod layout_refinement;
mod statements;

use context::{
    CallableContext, DebugContext, ExpressionMode, FailureContext, LoopContext, NonePolicy,
};
use declarations::{Binding, DeclarationEnvironment};

use crate::{
    Diagnostic,
    ast::{EnumDecl, ExprId, FunctionId, Program, Span, TypeRef, ValueId},
    inference::{InferenceContext, Requirements, Type},
    resolution::ProgramResolutions,
    semantic::{
        ResolvedEnumVariantId, ResolvedMember, ResolvedReceiver, ResolvedValue, SemanticBuilder,
        SemanticModel, ValueConversionKind,
    },
    stdlib::{
        DeclaredTypeRef, StandardLibrary, StdlibStateProviderId, StdlibTypeConstructorId,
        StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    types::{
        EnumTypeId, ResolvedArrayType, ResolvedAsyncType, ResolvedCallableType, ResolvedOptionType,
        ResolvedResultType, ResolvedSetType, ResolvedTypeRef, TypeKind, TypeStore,
    },
};

struct PathResolution {
    ty: Type,
    value: Option<ResolvedValue>,
    members: Option<Vec<ResolvedMember>>,
}

#[derive(Clone)]
struct DeferredMemberPath {
    expression: ExprId,
    receiver: Type,
    fields: Vec<String>,
    result: Type,
    span: Span,
    library_item: Option<crate::stdlib::StdlibItemId>,
}

struct MethodReceiver {
    ty: Type,
    value: ResolvedReceiver,
}

struct CallSyntax<'a> {
    callee: &'a [String],
    name_span: Span,
    postfix_receiver: Option<&'a crate::ast::Expr>,
    type_arguments: &'a [TypeRef],
}

#[derive(Clone)]
struct ExpectedTypeSource {
    span: Span,
    label: String,
}

pub struct CheckOutput {
    pub semantics: SemanticModel,
    pub enum_types: Vec<EnumDecl>,
    pub array_types: Vec<ResolvedArrayType>,
    pub option_types: Vec<ResolvedOptionType>,
    pub result_types: Vec<ResolvedResultType>,
    pub async_types: Vec<ResolvedAsyncType>,
    pub callable_types: Vec<ResolvedCallableType>,
    pub range_types: Vec<crate::types::ResolvedRangeType>,
    pub set_types: Vec<ResolvedSetType>,
    pub application_types: Vec<crate::types::ResolvedApplicationType>,
}

pub struct RecoveringCheckOutput {
    pub output: CheckOutput,
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg(test)]
pub fn check(program: &Program) -> Result<CheckOutput, Vec<Diagnostic>> {
    let recovered = check_recovering(program);
    if recovered.diagnostics.is_empty() {
        Ok(recovered.output)
    } else {
        Err(recovered.diagnostics)
    }
}

#[cfg(test)]
pub fn check_recovering(program: &Program) -> RecoveringCheckOutput {
    let standard_library = StandardLibrary::new();
    let mut resolutions = crate::resolution::ProgramResolutions::default();
    let mut resolution_diagnostics =
        crate::resolution::validate_declarations(program, &standard_library);
    resolution_diagnostics.extend(crate::resolution::resolve_program(
        program,
        &standard_library,
        &mut resolutions,
    ));
    let mut recovered = check_recovering_with_library(program, &resolutions, standard_library);
    resolution_diagnostics.append(&mut recovered.diagnostics);
    recovered.diagnostics = resolution_diagnostics;
    recovered
}

pub(crate) fn check_with_library(
    program: &Program,
    resolutions: &crate::resolution::ProgramResolutions,
    standard_library: StandardLibrary,
) -> Result<CheckOutput, Vec<Diagnostic>> {
    let recovered = check_recovering_with_library(program, resolutions, standard_library);
    if recovered.diagnostics.is_empty() {
        Ok(recovered.output)
    } else {
        Err(recovered.diagnostics)
    }
}

pub(crate) fn check_recovering_with_library(
    program: &Program,
    resolutions: &crate::resolution::ProgramResolutions,
    standard_library: StandardLibrary,
) -> RecoveringCheckOutput {
    driver::check_recovering(program, resolutions, standard_library)
}

#[derive(Clone)]
struct EnumVariantInfo {
    id: ResolvedEnumVariantId,
    name: String,
    payload: Option<Type>,
    source_span: Option<Span>,
}

#[derive(Clone)]
struct EnumInfo {
    name: String,
    variants: Vec<EnumVariantInfo>,
}

struct Checker {
    standard_library: StandardLibrary,
    resolutions: ProgramResolutions,
    errors: Vec<Diagnostic>,
    declarations: DeclarationEnvironment,
    inference: InferenceContext,
    provider_value: Option<(StdlibStateProviderId, Type)>,
    layout_value: Option<ValueId>,
    active_state_layout: Option<crate::ast::EnumVariantId>,
    active_managed_layouts: HashMap<crate::ast::ManagedClassId, crate::ast::EnumVariantId>,
    active_layout_constraints: HashMap<crate::ast::RecordFieldId, crate::ast::EnumVariantId>,
    scopes: Vec<HashMap<String, Binding>>,
    return_ty: Type,
    callable: CallableContext,
    expression_mode: ExpressionMode,
    debug_context: DebugContext,
    loops: LoopContext,
    failure: FailureContext,
    inferred_process_reads: Vec<(Type, Span)>,
    inferred_empty_collections: Vec<(Type, Span, &'static str)>,
    deferred_member_paths: Vec<DeferredMemberPath>,
    none_policy: NonePolicy,
    semantics: SemanticBuilder,
    standard_field_types: HashMap<crate::stdlib::StdlibFieldId, Type>,
    active_function_component: HashSet<FunctionId>,
    expected_type_source: Option<ExpectedTypeSource>,
    return_type_source: Option<ExpectedTypeSource>,
}

impl Checker {
    fn is_library_function(&self) -> bool {
        matches!(
            self.callable,
            CallableContext::LibraryFunction(_) | CallableContext::CompilerGenerated
        )
    }

    fn is_provider_value_name(&self, name: &str) -> bool {
        self.provider_value.is_some_and(|(provider, _)| {
            self.standard_library.state_provider(provider).value_name == name
        })
    }

    fn with_state_layout<T>(
        &mut self,
        layout: Option<crate::ast::EnumVariantId>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.active_state_layout, layout);
        let output = operation(self);
        self.active_state_layout = previous;
        output
    }

    fn with_managed_layout<T>(
        &mut self,
        class: Option<crate::ast::ManagedClassId>,
        layout: Option<crate::ast::EnumVariantId>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = class.and_then(|class| {
            layout
                .map(|layout| self.active_managed_layouts.insert(class, layout))
                .unwrap_or_else(|| self.active_managed_layouts.remove(&class))
        });
        let output = operation(self);
        if let Some(class) = class {
            if let Some(previous) = previous {
                self.active_managed_layouts.insert(class, previous);
            } else {
                self.active_managed_layouts.remove(&class);
            }
        }
        output
    }

    fn with_debug_context<T>(
        &mut self,
        context: DebugContext,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.debug_context, context);
        let output = operation(self);
        self.debug_context = previous;
        output
    }

    fn with_loop<T>(&mut self, operation: impl FnOnce(&mut Self) -> T) -> T {
        self.loops.enter_statement();
        let output = operation(self);
        self.loops.exit();
        output
    }

    fn with_value_loop<T>(
        &mut self,
        result: Type,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> (T, bool) {
        self.loops.enter_value(result);
        let output = operation(self);
        let has_break = self.loops.exit();
        (output, has_break)
    }

    fn with_expression_mode<T>(
        &mut self,
        mode: ExpressionMode,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.expression_mode, mode);
        let output = operation(self);
        self.expression_mode = previous;
        output
    }

    fn with_none_policy<T>(
        &mut self,
        policy: NonePolicy,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.none_policy, policy);
        let output = operation(self);
        self.none_policy = previous;
        output
    }

    fn with_expected_type_source<T>(
        &mut self,
        source: ExpectedTypeSource,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.expected_type_source.replace(source);
        let output = operation(self);
        self.expected_type_source = previous;
        output
    }

    fn with_return_type_source<T>(
        &mut self,
        source: Option<ExpectedTypeSource>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.return_type_source, source);
        let output = operation(self);
        self.return_type_source = previous;
        output
    }

    fn with_failure_context<T>(
        &mut self,
        context: FailureContext,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> (T, FailureContext) {
        let previous = std::mem::replace(&mut self.failure, context);
        let output = operation(self);
        let completed = std::mem::replace(&mut self.failure, previous);
        (output, completed)
    }

    fn with_callable_context<T>(
        &mut self,
        callable: CallableContext,
        return_ty: Type,
        failure: FailureContext,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous_callable = std::mem::replace(&mut self.callable, callable);
        let previous_return = std::mem::replace(&mut self.return_ty, return_ty);
        let previous_failure = std::mem::replace(&mut self.failure, failure);
        // A callable body is a lexical control-flow boundary. In particular,
        // a closure written inside a loop cannot target that loop with
        // `break` or `continue` when it is invoked later. Give the nested body
        // its own loop stack, just as it already receives independent return
        // and failure boundaries.
        let previous_loops = std::mem::take(&mut self.loops);
        let output = operation(self);
        self.callable = previous_callable;
        self.return_ty = previous_return;
        self.failure = previous_failure;
        self.loops = previous_loops;
        output
    }

    fn syntax_type(&self, ty: TypeRef) -> Type {
        syntax_type(ty, self.inference.type_store(), &self.resolutions)
    }

    fn resolved_type_ref(&self, ty: ResolvedTypeRef) -> Type {
        resolved_type_ref(ty, self.inference.type_store())
    }

    fn standard_type(&self, standard: StdlibTypeId) -> Type {
        self.inference.known_standard(standard)
    }

    fn core_type(&self, core: crate::stdlib::CoreTypeId) -> Type {
        self.inference.known_core(core)
    }

    fn standard_type_id(&self, ty: Type) -> Option<StdlibTypeId> {
        self.inference.standard_type(ty)
    }

    fn record_type(&self, record: crate::ast::RecordId) -> Type {
        Type::Known(self.inference.type_store().id_for_record(record))
    }

    fn source_record_id(&self, ty: Type) -> Option<crate::ast::RecordId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.inference.type_store().kind(id) {
            TypeKind::Record(record) => Some(*record),
            _ => None,
        }
    }

    fn source_enum_id(&self, ty: Type) -> Option<crate::ast::EnumId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.inference.type_store().kind(id) {
            TypeKind::Enum(enumeration) => Some(*enumeration),
            _ => None,
        }
    }

    fn declared_type(&self, ty: DeclaredTypeRef) -> Type {
        inference_type_for_declared(self.inference.type_store(), ty)
    }

    fn standard_field_type(&self, field: crate::stdlib::StdlibFieldId) -> Type {
        self.standard_field_types[&field]
    }

    fn expected_value_type(&mut self, ty: Type) -> Type {
        match self.shallow_type(ty) {
            Type::Option(option) => self.inference.option_value(option),
            Type::Result(result) => self.inference.result_value(result),
            ty => ty,
        }
    }

    fn expect_expression(
        &mut self,
        expression: ExprId,
        actual: Type,
        expected: Option<Type>,
        span: Span,
    ) -> Option<Type> {
        let Some(expected) = expected else {
            return Some(actual);
        };
        let expected = self.shallow_type(expected);
        let actual_shallow = self.shallow_type(actual);
        self.failure.observe_result(expected, actual_shallow);
        if self.is_error_type(actual_shallow) || self.is_error_type(expected) {
            return Some(self.error_type());
        }
        // `Never` is the bottom type: an expression which cannot complete can
        // appear wherever a value is expected. Keep its semantic type intact
        // so control-flow and code generation still know that this edge has no
        // runtime value. This conversion is deliberately directional; normal
        // values must not unify with an explicitly required `Never` type.
        if self.is_never_type(actual_shallow) {
            return Some(actual_shallow);
        }
        let none = self.core_type(crate::stdlib::CoreTypeId::None);
        let nested_wrapper_lift = match (expected, actual_shallow) {
            (Type::Option(option), Type::Result(_)) => matches!(
                self.shallow_type(self.inference.option_value(option)),
                Type::Result(_) | Type::Variable(_)
            ),
            (Type::Result(result), Type::Option(_)) => matches!(
                self.shallow_type(self.inference.result_value(result)),
                Type::Option(_) | Type::Variable(_)
            ),
            _ => false,
        };
        let (kind, value) = match (expected, actual_shallow) {
            (expected @ Type::Option(_), actual) if actual == none => {
                self.semantics.resolve_value_conversion(
                    expression,
                    ValueConversionKind::NoneToOptional,
                    actual,
                    expected,
                );
                return Some(expected);
            }
            (Type::Option(_), Type::Option(_)) | (Type::Result(_), Type::Result(_)) => {
                return self.unify_expected(actual, expected, span);
            }
            (Type::Variable(_), _) => return self.unify_expected(actual, expected, span),
            (Type::Option(option), Type::Result(_)) if nested_wrapper_lift => (
                ValueConversionKind::LiftOption,
                self.inference.option_value(option),
            ),
            (Type::Result(result), Type::Option(_)) if nested_wrapper_lift => (
                ValueConversionKind::LiftResult,
                self.inference.result_value(result),
            ),
            (_, Type::Result(result)) => {
                let value = self.inference.result_value(result);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify_expected(value, expected, span)?;
                }
                let expected = self.type_name(expected);
                self.diagnose_unhandled_result(
                    actual_shallow,
                    format!("using it where `{expected}` is required"),
                    span,
                );
                return None;
            }
            (_, Type::Option(option)) => {
                let value = self.inference.option_value(option);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify_expected(value, expected, span)?;
                }
                let actual = self.type_name(actual_shallow);
                let expected = self.type_name(expected);
                let diagnostic = Diagnostic::type_error(
                    format!(
                        "cannot use optional `{actual}` where `{expected}` is required; unwrap it with `else` or handle it with `match`"
                    ),
                    span,
                );
                let diagnostic = self.with_expected_source_label(diagnostic);
                self.errors.push(diagnostic);
                return None;
            }
            (Type::Option(option), _) => (
                ValueConversionKind::LiftOption,
                self.inference.option_value(option),
            ),
            (Type::Result(result), _) => (
                ValueConversionKind::LiftResult,
                self.inference.result_value(result),
            ),
            _ => return self.unify_expected(actual, expected, span),
        };
        self.unify_expected(actual, value, span)?;
        self.semantics
            .resolve_value_conversion(expression, kind, actual, expected);
        Some(expected)
    }

    fn diagnose_unhandled_result(
        &mut self,
        actual: Type,
        use_context: impl Into<String>,
        span: Span,
    ) -> bool {
        let actual = self.shallow_type(actual);
        if !matches!(actual, Type::Result(_)) {
            return false;
        }

        let actual = self.type_name(actual);
        let mut diagnostic = Diagnostic::type_error(
            format!(
                "fallible value `{actual}` must be handled before {}",
                use_context.into()
            ),
            span,
        )
        .with_primary_label("this expression still has a fallible `T!` type")
        .with_note(
            "use `value else fallback` when an ordinary fallback value should replace the error",
        )
        .with_note(
            "use `match value { Ok(value) => ..., Err(error) => ... }` when both outcomes matter",
        );
        if self.failure.result().is_some() {
            diagnostic = diagnostic.with_note(
                "postfix `?` is available here and returns the error from the current fallible boundary",
            );
        }
        diagnostic = self.with_expected_source_label(diagnostic);
        self.errors.push(diagnostic);
        true
    }

    fn fresh_inference(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
    ) -> Type {
        self.inference.fresh(requirements, largest_literal)
    }

    fn shallow_type(&mut self, ty: Type) -> Type {
        self.inference.shallow(ty)
    }

    fn error_type(&mut self) -> Type {
        self.inference.error_type()
    }

    fn is_error_type(&mut self, ty: Type) -> bool {
        self.inference.is_error_type(ty)
    }

    fn is_never_type(&mut self, ty: Type) -> bool {
        self.inference.is_never_type(ty)
    }

    fn unify(&mut self, left: Type, right: Type, span: Span) -> Option<Type> {
        match self.inference.unify(left, right) {
            Ok(ty) => Some(ty),
            Err(error) => {
                let message = self.inference_error_message(error);
                self.error(message, span);
                None
            }
        }
    }

    fn unify_expected(&mut self, actual: Type, expected: Type, span: Span) -> Option<Type> {
        let actual_before = self.shallow_type(actual);
        let expected_before = self.shallow_type(expected);
        match self.inference.unify(actual, expected) {
            Ok(ty) => Some(ty),
            Err(error) => {
                let diagnostic =
                    self.expected_type_diagnostic(actual_before, expected_before, &error, span);
                self.errors.push(diagnostic);
                None
            }
        }
    }

    fn expected_type_diagnostic(
        &mut self,
        actual: Type,
        expected: Type,
        error: &crate::inference::InferenceError,
        span: Span,
    ) -> Diagnostic {
        let expected_name = self.type_name(expected);
        let is_source_mismatch = matches!(
            error,
            crate::inference::InferenceError::TypeMismatch { .. }
                | crate::inference::InferenceError::UnsupportedOperation { .. }
                | crate::inference::InferenceError::UnsatisfiedConstraints { .. }
        );
        let diagnostic = if !matches!(expected, Type::Variable(_)) && is_source_mismatch {
            let (found, primary_label) = match self.inference.literal_kind(actual) {
                Some(crate::inference::LiteralKind::Integer) => (
                    "an integer literal".to_owned(),
                    "this value is an integer literal".to_owned(),
                ),
                Some(crate::inference::LiteralKind::Float) => (
                    "a floating-point literal".to_owned(),
                    "this value is a floating-point literal".to_owned(),
                ),
                None => {
                    let actual_name = self.type_name(actual);
                    (
                        format!("`{actual_name}`"),
                        format!("this value has type `{actual_name}`"),
                    )
                }
            };
            Diagnostic::type_error(format!("expected `{expected_name}`, found {found}"), span)
                .with_primary_label(primary_label)
        } else if matches!(
            error,
            crate::inference::InferenceError::IntegerLiteralOutOfRange(_)
        ) {
            Diagnostic::type_error(self.inference_error_message(error.clone()), span)
                .with_primary_label("this integer literal does not fit in the declared type")
        } else {
            Diagnostic::type_error(self.inference_error_message(error.clone()), span)
                .with_primary_label("this value does not meet the declared type requirements")
        };
        self.with_expected_source_label(diagnostic)
    }

    fn with_expected_source_label(&self, diagnostic: Diagnostic) -> Diagnostic {
        if let Some(source) = self.expected_type_source.as_ref() {
            diagnostic.with_secondary_label(source.span, source.label.clone())
        } else {
            diagnostic
        }
    }

    fn require(&mut self, ty: Type, requirements: Requirements, span: Span) -> Option<()> {
        match self.inference.require(ty, requirements) {
            Ok(()) => Some(()),
            Err(error) => {
                let message = self.inference_error_message(error);
                self.error(message, span);
                None
            }
        }
    }

    fn enum_type(&self, enumeration: EnumTypeId) -> Type {
        match enumeration {
            EnumTypeId::Source(id) => Type::Known(self.inference.type_store().id_for_enum(id)),
            EnumTypeId::Standard(id) => self.standard_type(id),
        }
    }

    fn enum_info(&self, enumeration: EnumTypeId) -> Option<EnumInfo> {
        match enumeration {
            EnumTypeId::Source(id) => self
                .declarations
                .enums
                .iter()
                .find(|declaration| declaration.id == id)
                .map(|declaration| EnumInfo {
                    name: declaration.name.clone(),
                    variants: declaration
                        .variants
                        .iter()
                        .map(|variant| EnumVariantInfo {
                            id: ResolvedEnumVariantId::Source(variant.id),
                            name: variant.name.clone(),
                            payload: variant.payload.map(|ty| self.syntax_type(ty)),
                            source_span: Some(variant.span),
                        })
                        .collect(),
                }),
            EnumTypeId::Standard(id) => {
                let library = &self.standard_library;
                let declaration = library.type_decl(id);
                let variants = library.variants_of(id).collect::<Vec<_>>();
                (!variants.is_empty()).then(|| EnumInfo {
                    name: declaration.name.to_owned(),
                    variants: variants
                        .into_iter()
                        .map(|variant| EnumVariantInfo {
                            id: ResolvedEnumVariantId::Standard(variant.id),
                            name: variant.name.to_owned(),
                            payload: None,
                            source_span: None,
                        })
                        .collect(),
                })
            }
        }
    }

    fn enum_info_for_type(&self, ty: Type) -> Option<(EnumTypeId, EnumInfo)> {
        let enumeration = match (ty, self.source_enum_id(ty)) {
            (Type::Known(_), Some(id)) => EnumTypeId::Source(id),
            (Type::Known(_), None) => EnumTypeId::Standard(self.standard_type_id(ty)?),
            _ => return None,
        };
        self.enum_info(enumeration)
            .map(|declaration| (enumeration, declaration))
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(Diagnostic::type_error(message, span));
    }
}

fn inference_type_for_declared(types: &TypeStore, ty: DeclaredTypeRef) -> Type {
    match ty {
        DeclaredTypeRef::Core(core) => Type::Known(types.id_for_core(core)),
        DeclaredTypeRef::Standard(standard) => Type::Known(types.id_for_standard(standard)),
    }
}

fn catalog_type_argument(
    ty: CatalogTypeRef,
    expected_constructor: StdlibTypeConstructorId,
) -> Option<CatalogTypeRef> {
    let CatalogTypeRef::Application {
        constructor,
        arguments: [argument],
    } = ty
    else {
        return None;
    };
    (constructor == expected_constructor).then_some(*argument)
}

fn syntax_type(ty: TypeRef, types: &TypeStore, resolutions: &ProgramResolutions) -> Type {
    let ty = resolutions
        .type_ref(ty)
        .unwrap_or(ResolvedTypeRef::Core(crate::stdlib::CoreTypeId::None));
    resolved_type_ref(ty, types)
}

fn resolved_type_ref(ty: ResolvedTypeRef, types: &TypeStore) -> Type {
    match ty {
        ResolvedTypeRef::Error => Type::Known(
            types
                .existing_error()
                .expect("recovery interned its semantic error type"),
        ),
        ResolvedTypeRef::Core(core) => Type::Known(types.id_for_core(core)),
        ResolvedTypeRef::Standard(standard) => Type::Known(types.id_for_standard(standard)),
        ResolvedTypeRef::StateSnapshot => Type::Known(types.id_for_state_snapshot()),
        ResolvedTypeRef::SettingsView => Type::Known(types.id_for_settings_view()),
        ResolvedTypeRef::Record(record) => Type::Known(types.id_for_record(record)),
        ResolvedTypeRef::Enum(enumeration) => Type::Known(types.id_for_enum(enumeration)),
        ResolvedTypeRef::ManagedClass(class) => Type::Known(types.id_for_managed_class(class)),
        ResolvedTypeRef::ManagedReference(class) => {
            Type::Known(types.id_for_managed_reference(class))
        }
        ResolvedTypeRef::GenericParameter(parameter) => Type::Known(parameter),
        ResolvedTypeRef::Array(id) => Type::Array(id),
        ResolvedTypeRef::Option(id) => Type::Option(id),
        ResolvedTypeRef::Result(id) => Type::Result(id),
        ResolvedTypeRef::Async(id) => Type::Async(id),
        ResolvedTypeRef::Callable(id) => Type::Callable(id),
        ResolvedTypeRef::Range(id) => Type::Range(id),
        ResolvedTypeRef::Set(id) => Type::Set(id),
        ResolvedTypeRef::Application(id) => Type::Application(id),
    }
}

fn closest_name<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let normalized_name = normalize_name(name);
    let maximum_distance = match normalized_name.chars().count() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    let mut seen = HashSet::new();
    let mut best: Option<(usize, String)> = None;
    let mut tied = false;
    for candidate in candidates {
        if candidate == name || !seen.insert(candidate) {
            continue;
        }
        let distance = edit_distance(&normalized_name, &normalize_name(candidate));
        if distance > maximum_distance {
            continue;
        }
        match &best {
            None => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance < *best_distance => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance == *best_distance => tied = true,
            Some(_) => {}
        }
    }
    (!tied)
        .then(|| best.map(|(_, candidate)| candidate))
        .flatten()
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_character != *right_character));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    use super::*;

    fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
        let program = parser::parse(source, lexer::lex(source).unwrap()).unwrap();
        check(&program).map(|_| ())
    }

    #[test]
    fn infers_local_from_precisely_typed_state() {
        check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            split {
                let next = current.level + 1;
                return next != old.level;
            }
            "#,
        )
        .unwrap();
    }

    #[test]
    fn rejects_mixed_integer_widths() {
        let errors = check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            split { return current.level == 1u32; }
            "#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("types do not match"))
        );
    }

    #[test]
    fn infers_integer_literals_bidirectionally_and_from_array_elements() {
        check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            whileAttached {
                let byte: u8 = 0x8b
                let bytes = [0x48, byte]
                if (0 == current.level && (1 + current.level) == 2 && bytes[0] == 0x48) {
                    print("inferred")
                }
            }
            "#,
        )
        .unwrap();
    }

    #[test]
    fn user_code_cannot_access_standard_library_private_fields() {
        let errors = check_source(
            r#"
            state "game" {}
            whileAttached {
                let seconds = Duration.zero().seconds
            }
            "#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("Duration has no field `seconds`")),
            "{errors:#?}"
        );
    }
}
