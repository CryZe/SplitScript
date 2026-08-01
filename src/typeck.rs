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
mod statements;

use context::{
    CallableContext, DebugContext, ExpressionMode, FailureContext, LoopContext, NonePolicy,
};
use declarations::{Binding, DeclarationEnvironment};

use crate::{
    Diagnostic,
    ast::{EnumDecl, ExprId, FunctionId, Program, Span, TypeRef},
    inference::{InferenceContext, Requirements, Type},
    resolution::ProgramResolutions,
    semantic::{
        ResolvedEnumVariantId, ResolvedMember, ResolvedValue, SemanticBuilder, SemanticModel,
        ValueConversionKind,
    },
    stdlib::{
        DeclaredTypeRef, StandardLibrary, StdlibStateProviderId, StdlibTypeConstructorId,
        StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    types::{
        EnumTypeId, ResolvedArrayType, ResolvedOptionType, ResolvedResultType, ResolvedTypeRef,
        TypeKind, TypeStore,
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
}

struct MethodReceiver {
    ty: Type,
    value: ResolvedValue,
    members: Vec<ResolvedMember>,
}

pub struct CheckOutput {
    pub semantics: SemanticModel,
    pub enum_types: Vec<EnumDecl>,
    pub array_types: Vec<ResolvedArrayType>,
    pub option_types: Vec<ResolvedOptionType>,
    pub result_types: Vec<ResolvedResultType>,
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
    scopes: Vec<HashMap<String, Binding>>,
    return_ty: Type,
    callable: CallableContext,
    expression_mode: ExpressionMode,
    debug_context: DebugContext,
    loops: LoopContext,
    failure: FailureContext,
    inferred_process_reads: Vec<(Type, Span)>,
    deferred_member_paths: Vec<DeferredMemberPath>,
    none_policy: NonePolicy,
    semantics: SemanticBuilder,
    active_function_component: HashSet<FunctionId>,
}

impl Checker {
    fn is_provider_value_name(&self, name: &str) -> bool {
        self.provider_value.is_some_and(|(provider, _)| {
            self.standard_library.state_provider(provider).value_name == name
        })
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
        self.loops.enter();
        let output = operation(self);
        self.loops.exit();
        output
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
        let output = operation(self);
        self.callable = previous_callable;
        self.return_ty = previous_return;
        self.failure = previous_failure;
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
        let (kind, value) = match (expected, actual_shallow) {
            (Type::Option(_), Type::Option(_)) | (Type::Result(_), Type::Result(_)) => {
                return self.unify(actual, expected, span);
            }
            (Type::Variable(_), _) => return self.unify(actual, expected, span),
            (_, Type::Result(result)) => {
                let value = self.inference.result_value(result);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify(value, expected, span)?;
                }
                let actual = self.type_name(actual_shallow);
                let expected = self.type_name(expected);
                self.error(
                    format!(
                        "cannot use fallible `{actual}` where `{expected}` is required; unwrap it with `else`, propagate it with `?`, or use `retry` in `onAttach`"
                    ),
                    span,
                );
                return None;
            }
            (_, Type::Option(option)) => {
                let value = self.inference.option_value(option);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify(value, expected, span)?;
                }
                let actual = self.type_name(actual_shallow);
                let expected = self.type_name(expected);
                self.error(
                    format!(
                        "cannot use optional `{actual}` where `{expected}` is required; unwrap it with `else` or handle it with `match`"
                    ),
                    span,
                );
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
            _ => return self.unify(actual, expected, span),
        };
        self.unify(actual, value, span)?;
        self.semantics
            .resolve_value_conversion(expression, kind, actual, expected);
        Some(expected)
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
        .unwrap_or(ResolvedTypeRef::Core(crate::stdlib::CoreTypeId::Void));
    resolved_type_ref(ty, types)
}

fn resolved_type_ref(ty: ResolvedTypeRef, types: &TypeStore) -> Type {
    match ty {
        ResolvedTypeRef::Core(core) => Type::Known(types.id_for_core(core)),
        ResolvedTypeRef::Standard(standard) => Type::Known(types.id_for_standard(standard)),
        ResolvedTypeRef::Record(record) => Type::Known(types.id_for_record(record)),
        ResolvedTypeRef::Enum(enumeration) => Type::Known(types.id_for_enum(enumeration)),
        ResolvedTypeRef::GenericParameter(parameter) => Type::Known(parameter),
        ResolvedTypeRef::Array(id) => Type::Array(id),
        ResolvedTypeRef::Option(id) => Type::Option(id),
        ResolvedTypeRef::Result(id) => Type::Result(id),
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
                if (0 == current.level && (1 + current.level) == 2 && bytes.get(0) == 0x48) {
                    print("inferred")
                }
            }
            "#,
        )
        .unwrap();
    }
}
