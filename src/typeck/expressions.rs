//! Expression constraints, wrapper flow, pattern checking, and operators.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic, DiagnosticFix, FixApplicability, TextEdit,
    ast::{
        BinaryOp, Expr, ExprId, ExprKind, InterpolatedPart, MatchPattern, Span, SuspensionMode,
        UnaryOp,
    },
    inference::{Requirements, Type},
    migration::{ASL_SETTINGS_LOOKUP_DIAGNOSTIC, migration_diagnostic},
    semantic::{
        PendingFunctionValue, ResolvedEnumVariantId, ResolvedStructFieldId, ResolvedStructId,
        ResolvedWrapperPattern,
    },
    signature::parse_signature,
    stdlib::{RuntimeRepresentation, StdlibCapabilityId, StdlibTypeId},
    types::{EnumTypeId, TypeKind},
};

use super::{Checker, context::NonePolicy, declarations::Binding};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PatternCoverage {
    Irrefutable,
    Enum {
        variant: ResolvedEnumVariantId,
        name: String,
        payload: Box<Self>,
    },
    Struct {
        structure: crate::ast::StructId,
        name: String,
        fields: Vec<(crate::ast::StructFieldId, String, Self)>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    FileVersion([u16; 4]),
    OptionNone,
    OptionSome(Box<Self>),
    IteratorEnd,
    IteratorItem(Box<Self>),
    ResultSuccess(Box<Self>),
    ResultError(Box<Self>),
    Array(Vec<Self>),
    Alternation(Vec<Self>),
    Invalid(crate::ast::PatternId),
}

impl PatternCoverage {
    fn is_irrefutable(&self) -> bool {
        matches!(self, Self::Irrefutable)
    }

    fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Irrefutable, _) => true,
            (Self::Alternation(alternatives), _) => {
                alternatives.iter().any(|pattern| pattern.covers(other))
            }
            (_, Self::Alternation(alternatives)) => {
                alternatives.iter().all(|pattern| self.covers(pattern))
            }
            (Self::Array(left), Self::Array(right)) if left.len() == right.len() => left
                .iter()
                .zip(right)
                .all(|(left, right)| left.covers(right)),
            (
                Self::Enum {
                    variant: left_variant,
                    payload: left_payload,
                    ..
                },
                Self::Enum {
                    variant: right_variant,
                    payload: right_payload,
                    ..
                },
            ) if left_variant == right_variant => left_payload.covers(right_payload),
            (
                Self::Struct {
                    structure: left_structure,
                    fields: left_fields,
                    ..
                },
                Self::Struct {
                    structure: right_structure,
                    fields: right_fields,
                    ..
                },
            ) if left_structure == right_structure => {
                left_fields.iter().all(|(left_field, _, left_pattern)| {
                    right_fields
                        .iter()
                        .find(|(right_field, _, _)| right_field == left_field)
                        .is_some_and(|(_, _, right_pattern)| left_pattern.covers(right_pattern))
                })
            }
            (Self::OptionSome(left), Self::OptionSome(right))
            | (Self::IteratorItem(left), Self::IteratorItem(right))
            | (Self::ResultSuccess(left), Self::ResultSuccess(right))
            | (Self::ResultError(left), Self::ResultError(right)) => left.covers(right),
            _ => self == other,
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Irrefutable => "_".to_owned(),
            Self::Enum { name, payload, .. } if payload.is_irrefutable() => name.clone(),
            Self::Enum { name, payload, .. } => format!("{name}({})", payload.display()),
            Self::Struct { name, fields, .. } => format!(
                "{name} {{ {} }}",
                fields
                    .iter()
                    .map(|(_, field, pattern)| format!("{field}: {}", pattern.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Bool(value) => value.to_string(),
            Self::Char(value) => format!("{value:?}"),
            Self::String(value) => format!("{value:?}"),
            Self::Int { value, negative } => {
                format!("{}{value}", if *negative { "-" } else { "" })
            }
            Self::FileVersion(parts) => {
                format!("v\"{}.{}.{}.{}\"", parts[0], parts[1], parts[2], parts[3])
            }
            Self::OptionNone => "None".to_owned(),
            Self::OptionSome(payload) => format!("Some({})", payload.display()),
            Self::IteratorEnd => "End".to_owned(),
            Self::IteratorItem(payload) => format!("Item({})", payload.display()),
            Self::ResultSuccess(payload) => format!("Ok({})", payload.display()),
            Self::ResultError(payload) => format!("Err({})", payload.display()),
            Self::Array(elements) => format!(
                "[{}]",
                elements
                    .iter()
                    .map(Self::display)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Alternation(alternatives) => alternatives
                .iter()
                .map(Self::display)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Invalid(_) => "<invalid pattern>".to_owned(),
        }
    }
}

fn patterns_cover(patterns: &[PatternCoverage], target: &PatternCoverage) -> bool {
    match target {
        PatternCoverage::Alternation(alternatives) => alternatives
            .iter()
            .all(|alternative| patterns_cover(patterns, alternative)),
        _ => patterns.iter().any(|pattern| pattern.covers(target)),
    }
}

struct CheckedPattern {
    coverage: PatternCoverage,
    layout_variants: Option<HashSet<crate::ast::EnumVariantId>>,
}

impl CheckedPattern {
    fn new(coverage: PatternCoverage) -> Self {
        Self {
            coverage,
            layout_variants: None,
        }
    }
}

impl Checker {
    pub(super) fn expr(&mut self, expr: &Expr, expected: Option<Type>) -> Option<Type> {
        let ty = match &expr.kind {
            ExprKind::Error => {
                self.error("cannot type-check a recovered expression", expr.span);
                return None;
            }
            ExprKind::None => {
                let none = self.core_type(crate::stdlib::CoreTypeId::None);
                match expected.map(|ty| self.shallow_type(ty)) {
                    Some(expected) if self.expected_accepts_none(expected) => {
                        self.semantics.resolve_value_conversion(
                            expr.id,
                            crate::semantic::ValueConversionKind::NoneToOptional,
                            none,
                            expected,
                        );
                        expected
                    }
                    Some(expected) if self.none_policy == NonePolicy::DomainNullable => {
                        self.semantics.resolve_value_conversion(
                            expr.id,
                            crate::semantic::ValueConversionKind::NoneToDomainNullable,
                            none,
                            expected,
                        );
                        expected
                    }
                    Some(Type::Result(result))
                        if self.shallow_type(self.inference.result_value(result)) != none =>
                    {
                        let value = self.inference.result_value(result);
                        self.expect_expression(expr.id, none, Some(value), expr.span)?
                    }
                    Some(expected) => {
                        self.expect_expression(expr.id, none, Some(expected), expr.span)?
                    }
                    None => none,
                }
            }
            ExprKind::IteratorEnd => {
                let item = self.fresh_inference(Requirements::none(), None);
                let step = Type::Application(self.inference.application_type(
                    crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                    vec![item],
                ));
                self.expect_expression(expr.id, step, expected, expr.span)?
            }
            ExprKind::Bool(_) => self.expect_expression(
                expr.id,
                self.core_type(crate::stdlib::CoreTypeId::Bool),
                expected,
                expr.span,
            )?,
            ExprKind::Int {
                value,
                negative,
                suffix,
            } => {
                let ty = if let Some(suffix) = suffix {
                    let suffix = self.syntax_type(*suffix);
                    let fits = if *negative {
                        self.inference.fits_negative_literal(*value, suffix)
                    } else {
                        self.inference.fits_unsigned_literal(*value, suffix)
                    };
                    if !fits {
                        self.error(
                            format!("integer literal does not fit in `{suffix}`"),
                            expr.span,
                        );
                        return None;
                    }
                    suffix
                } else {
                    if *negative {
                        self.inference.fresh_negative_integer_literal(
                            Requirements::capability(StdlibCapabilityId::Numeric),
                            *value,
                        )
                    } else {
                        self.fresh_inference(
                            Requirements::capability(StdlibCapabilityId::Numeric),
                            Some(*value),
                        )
                    }
                };
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Float(_) => {
                let ty = self.inference.fresh_float_literal();
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Char(_) => self.expect_expression(
                expr.id,
                self.core_type(crate::stdlib::CoreTypeId::Char),
                expected,
                expr.span,
            )?,
            ExprKind::String(_) => self.expect_expression(
                expr.id,
                self.standard_type(StdlibTypeId::String),
                expected,
                expr.span,
            )?,
            ExprKind::InterpolatedString(parts) => {
                self.array_type_id(self.standard_type(StdlibTypeId::String));
                for part in parts {
                    if let InterpolatedPart::Expr(value) = part {
                        let value_type = self.expr(value, None)?;
                        self.require(
                            value_type,
                            Requirements::capability(StdlibCapabilityId::Display),
                            value.span,
                        );
                    }
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::String),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Signature(signature) => {
                if let Err(message) = parse_signature(signature) {
                    self.error(message, expr.span);
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::Signature),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Array(elements) => {
                let value_expected = expected.map(|ty| self.expected_value_type(ty));
                let hinted = value_expected.and_then(|ty| match ty {
                    Type::Array(id) => self
                        .inference
                        .arrays()
                        .iter()
                        .find(|array| array.id == id)
                        .map(|array| (id, array.element)),
                    _ => None,
                });
                let (id, element_type) = if let Some((id, element)) = hinted {
                    (id, element)
                } else {
                    let element = self.fresh_inference(Requirements::none(), None);
                    let id = self.array_type_id(element);
                    (id, element)
                };
                if elements.is_empty() {
                    self.inferred_empty_collections
                        .push((element_type, expr.span, "array"));
                }
                if let Some(expected_length) = self.inference.array_length(id)
                    && elements.len() != expected_length as usize
                {
                    self.error(
                        format!(
                            "expected {expected_length} array elements, found {}",
                            elements.len()
                        ),
                        expr.span,
                    );
                }
                for element in elements {
                    self.expr(element, Some(element_type));
                }
                self.expect_expression(expr.id, Type::Array(id), expected, expr.span)?
            }
            ExprKind::Range {
                start, end, kind, ..
            } => {
                let hinted = expected
                    .map(|ty| self.expected_value_type(ty))
                    .and_then(|ty| match self.shallow_type(ty) {
                        Type::Range(range) if self.inference.range_kind(range) == *kind => {
                            Some((range, self.inference.range_bound(range)))
                        }
                        _ => None,
                    });
                let (range, bound) = hinted.unwrap_or_else(|| {
                    let bound = self.fresh_inference(
                        Requirements::capability(StdlibCapabilityId::Integer),
                        None,
                    );
                    (self.inference.range_type(bound, *kind), bound)
                });
                self.require(
                    bound,
                    Requirements::capability(StdlibCapabilityId::Integer),
                    expr.span,
                );
                self.expr(start, Some(bound));
                self.expr(end, Some(bound));
                self.expect_expression(expr.id, Type::Range(range), expected, expr.span)?
            }
            ExprKind::Block(block) => {
                self.scopes.push(HashMap::new());
                let tail = block
                    .statements
                    .last()
                    .and_then(|statement| match statement {
                        crate::ast::Stmt::Expression(expression) => Some(expression),
                        _ => None,
                    });
                let prefix_len = block.statements.len() - usize::from(tail.is_some());
                for statement in &block.statements[..prefix_len] {
                    self.statement(statement);
                }
                let prefix_is_terminal = block.statements[..prefix_len]
                    .iter()
                    .any(|statement| super::body_pass::statement_is_terminal(self, statement));
                let ty = if prefix_is_terminal {
                    if let Some(tail) = tail {
                        self.expr(tail, None);
                    }
                    Some(self.core_type(crate::stdlib::CoreTypeId::Never))
                } else if let Some(tail) = tail {
                    self.expr(tail, expected)
                } else if super::body_pass::block_is_terminal(self, block) {
                    Some(self.core_type(crate::stdlib::CoreTypeId::Never))
                } else {
                    let none = self.core_type(crate::stdlib::CoreTypeId::None);
                    match expected.map(|expected| self.shallow_type(expected)) {
                        Some(expected) if self.expected_accepts_none(expected) => {
                            self.semantics.resolve_value_conversion(
                                expr.id,
                                crate::semantic::ValueConversionKind::NoneToOptional,
                                none,
                                expected,
                            );
                            Some(expected)
                        }
                        Some(expected) if self.none_policy == NonePolicy::DomainNullable => {
                            self.semantics.resolve_value_conversion(
                                expr.id,
                                crate::semantic::ValueConversionKind::NoneToDomainNullable,
                                none,
                                expected,
                            );
                            Some(expected)
                        }
                        Some(expected)
                            if !matches!(expected, Type::Variable(_)) && expected != none =>
                        {
                            let expected_name = self.type_name(expected);
                            let closing = Span {
                                start: block.span.end.saturating_sub(1),
                                end: block.span.end,
                            };
                            self.errors.push(
                                Diagnostic::type_error(
                                    format!(
                                        "this value block needs a final `{expected_name}` expression"
                                    ),
                                    closing,
                                )
                                .with_primary_label("the block reaches its end without a value")
                                .with_note(
                                    "write the value as the block's final expression; `return` exits the enclosing function instead",
                                ),
                            );
                            None
                        }
                        expected => self.expect_expression(expr.id, none, expected, expr.span),
                    }
                };
                self.scopes.pop();
                ty?
            }
            ExprKind::Loop(block) => {
                let inferred_result = expected.is_none();
                let result =
                    expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                self.scopes.push(HashMap::new());
                let (_, has_break) =
                    self.with_value_loop(result, |checker| checker.block(block, true));
                self.scopes.pop();
                if has_break {
                    self.expect_expression(expr.id, result, expected, expr.span)?
                } else {
                    let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                    if inferred_result {
                        self.unify(result, never, expr.span);
                    }
                    self.expect_expression(expr.id, never, expected, expr.span)?
                }
            }
            ExprKind::Struct { name, fields, .. } => {
                let declaration = self
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.name == *name)
                    .cloned();
                if let Some(declaration) = declaration {
                    self.semantics
                        .resolve_struct_literal(expr.id, ResolvedStructId::Source(declaration.id));
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declaration.fields.iter().find(|field| field.name == *name)
                        {
                            let field_type = self.syntax_type(field.ty);
                            let field_type_name = self.type_name(field_type);
                            self.with_expected_type_source(
                                super::ExpectedTypeSource {
                                    span: field.span,
                                    label: format!(
                                        "struct field `{}.{}` is declared as `{field_type_name}`",
                                        declaration.name, field.name,
                                    ),
                                },
                                |checker| checker.expr(value, Some(field_type)),
                            );
                            resolved_fields.push(ResolvedStructFieldId::Source(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declaration.fields {
                        if !seen.contains(&field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(
                        expr.id,
                        self.struct_type(declaration.id),
                        expected,
                        expr.span,
                    )?
                } else if let Some(declaration) = (if self.is_library_function() {
                    self.standard_library.type_by_name_including_private(name)
                } else {
                    self.standard_library.type_by_name(name)
                })
                .copied()
                {
                    let privileged_library_body = self.is_library_function();
                    if !privileged_library_body
                        || !matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                    {
                        self.error(
                            format!(
                                "standard-library type `{name}` can only be constructed by standard-library source"
                            ),
                            expr.span,
                        );
                        return None;
                    }
                    self.semantics.resolve_struct_literal(
                        expr.id,
                        ResolvedStructId::Standard(declaration.id),
                    );
                    let declared_fields = self
                        .standard_library
                        .fields_of(declaration.id)
                        .copied()
                        .collect::<Vec<_>>();
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declared_fields.iter().find(|field| field.name == *name)
                        {
                            self.expr(value, Some(self.standard_field_type(field.id)));
                            resolved_fields.push(ResolvedStructFieldId::Standard(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declared_fields {
                        if !seen.contains(field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(
                        expr.id,
                        self.standard_type(declaration.id),
                        expected,
                        expr.span,
                    )?
                } else if self.is_library_function()
                    && let Some(declaration) = self
                        .standard_library
                        .named_type_constructor_by_name(name)
                        .copied()
                {
                    let variables = declaration
                        .parameters
                        .iter()
                        .map(|parameter| {
                            let requirements = parameter.constraints.iter().fold(
                                Requirements::none(),
                                |requirements, constraint| {
                                    requirements | Requirements::capability(*constraint)
                                },
                            );
                            (parameter.name, self.fresh_inference(requirements, None))
                        })
                        .collect::<HashMap<_, _>>();
                    let arguments = declaration
                        .parameters
                        .iter()
                        .map(|parameter| variables[parameter.name])
                        .collect::<Vec<_>>();
                    let struct_type = self.catalog_application_type(declaration.id, arguments);
                    let Type::Application(application) = struct_type else {
                        unreachable!("named runtime struct constructors use application layouts")
                    };
                    self.semantics.resolve_struct_literal(
                        expr.id,
                        ResolvedStructId::StandardConstructor(application),
                    );
                    let declared_fields = self
                        .standard_library
                        .fields_of_constructor(declaration.id)
                        .copied()
                        .collect::<Vec<_>>();
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for literal_field in fields {
                        let name = &literal_field.name;
                        let value = &literal_field.value;
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate struct field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declared_fields.iter().find(|field| field.name == *name)
                        {
                            let field_type = self.catalog_type(field.ty, &variables);
                            self.expr(value, Some(field_type));
                            resolved_fields.push(ResolvedStructFieldId::Standard(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("struct `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_struct_literal_fields(expr.id, resolved_fields);
                    for field in &declared_fields {
                        if !seen.contains(field.name) {
                            self.error(
                                format!(
                                    "struct `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(expr.id, struct_type, expected, expr.span)?
                } else {
                    self.error(format!("unknown struct type `{name}`"), expr.span);
                    return None;
                }
            }
            ExprKind::Match { value, arms } => {
                let value_type = self.expr(value, None)?;
                let refines_state_layout = matches!(
                    &value.kind,
                    ExprKind::Path(path)
                        if matches!(path.as_slice(), [name] if name == "layout" || name == "provider")
                            && self.layout_value.is_some()
                );
                let mut unguarded_patterns = Vec::<PatternCoverage>::new();
                let mut has_unguarded_irrefutable = false;
                let mut result_type = expected;
                let mut never_arm_type = None;
                for arm in arms {
                    if has_unguarded_irrefutable {
                        self.error(
                            "unreachable match arm after an irrefutable pattern",
                            arm.span,
                        );
                    }
                    self.scopes.push(HashMap::new());
                    let checked =
                        self.check_pattern(&arm.pattern, arm.pattern_id, value_type, arm.span);
                    let state_layouts = if refines_state_layout {
                        checked.layout_variants.clone()
                    } else {
                        None
                    };
                    let arm_type = self.with_state_layouts(state_layouts, |checker| {
                        if let Some(guard) = &arm.guard {
                            checker.expr(
                                guard,
                                Some(checker.core_type(crate::stdlib::CoreTypeId::Bool)),
                            );
                        }
                        checker.expr(&arm.value, result_type)
                    });
                    self.scopes.pop();
                    if expected.is_none()
                        && let Some(arm_type) = arm_type
                    {
                        if self.is_never_type(arm_type) {
                            never_arm_type.get_or_insert(arm_type);
                        } else if result_type.is_none()
                            || result_type.is_some_and(|ty| self.is_never_type(ty))
                        {
                            result_type = Some(arm_type);
                        }
                    }

                    if arm.guard.is_none() {
                        if patterns_cover(&unguarded_patterns, &checked.coverage) {
                            let message = if unguarded_patterns.contains(&checked.coverage) {
                                format!("duplicate match arm `{}`", checked.coverage.display())
                            } else {
                                format!("unreachable match arm `{}`", checked.coverage.display())
                            };
                            self.error(message, arm.span);
                        }
                        if checked.coverage.is_irrefutable() {
                            has_unguarded_irrefutable = true;
                        }
                        unguarded_patterns.push(checked.coverage);
                    } else if patterns_cover(&unguarded_patterns, &checked.coverage) {
                        self.error("unreachable guarded match arm", arm.span);
                    }
                }

                if !has_unguarded_irrefutable {
                    match self.shallow_type(value_type) {
                        ty if ty == self.core_type(crate::stdlib::CoreTypeId::Bool) => {
                            for value in [false, true] {
                                if !patterns_cover(
                                    &unguarded_patterns,
                                    &PatternCoverage::Bool(value),
                                ) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{value}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Option(_) => {
                            for (pattern, display) in [
                                (PatternCoverage::OptionNone, "None"),
                                (
                                    PatternCoverage::OptionSome(Box::new(
                                        PatternCoverage::Irrefutable,
                                    )),
                                    "Some(value)",
                                ),
                            ] {
                                if !patterns_cover(&unguarded_patterns, &pattern) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Result(_) => {
                            for (pattern, display) in [
                                (
                                    PatternCoverage::ResultSuccess(Box::new(
                                        PatternCoverage::Irrefutable,
                                    )),
                                    "Ok(value)",
                                ),
                                (
                                    PatternCoverage::ResultError(Box::new(
                                        PatternCoverage::Irrefutable,
                                    )),
                                    "Err(error)",
                                ),
                            ] {
                                if !patterns_cover(&unguarded_patterns, &pattern) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Application(step)
                            if self.inference.application_constructor(step)
                                == crate::stdlib::StdlibTypeConstructorId::IteratorStep =>
                        {
                            for (pattern, display) in [
                                (
                                    PatternCoverage::IteratorItem(Box::new(
                                        PatternCoverage::Irrefutable,
                                    )),
                                    "Item(value)",
                                ),
                                (PatternCoverage::IteratorEnd, "End"),
                            ] {
                                if !patterns_cover(&unguarded_patterns, &pattern) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        ty if self.inference.is_integer(ty) => {
                            self.error("non-exhaustive integer match: add a `_` arm", expr.span)
                        }
                        ty if ty == self.core_type(crate::stdlib::CoreTypeId::Char) => {
                            self.error("non-exhaustive character match: add a `_` arm", expr.span)
                        }
                        ty if ty == self.standard_type(StdlibTypeId::String) => {
                            self.error("non-exhaustive string match: add a `_` arm", expr.span)
                        }
                        ty if ty == self.standard_type(StdlibTypeId::FileVersion) => self.error(
                            "non-exhaustive file-version match: add a `_` arm",
                            expr.span,
                        ),
                        Type::Array(_) => {
                            self.error("non-exhaustive array match: add a `_` arm", expr.span)
                        }
                        ty @ Type::Known(_) if self.source_struct_id(ty).is_some() => {
                            self.error("non-exhaustive struct match: add a `_` arm", expr.span)
                        }
                        ty @ Type::Known(_) => {
                            if let Some((_, declaration)) = self.enum_info_for_type(ty) {
                                for variant in &declaration.variants {
                                    let coverage = PatternCoverage::Enum {
                                        variant: variant.id,
                                        name: variant.name.clone(),
                                        payload: Box::new(PatternCoverage::Irrefutable),
                                    };
                                    if !patterns_cover(&unguarded_patterns, &coverage) {
                                        self.error(
                                            format!(
                                                "non-exhaustive match: missing `{}`",
                                                variant.name
                                            ),
                                            expr.span,
                                        );
                                    }
                                }
                            } else {
                                let ty = self.type_name(ty);
                                self.error(format!("type `{ty}` cannot be matched"), value.span);
                            }
                        }
                        Type::Variable(_) => self.error(
                            "match patterns do not determine the matched value's type",
                            value.span,
                        ),
                        ty => {
                            let ty = self.type_name(ty);
                            self.error(format!("type `{ty}` cannot be matched"), value.span);
                        }
                    }
                }
                let Some(result_type) = result_type.or(never_arm_type) else {
                    self.error("a match needs at least one arm", expr.span);
                    return None;
                };
                self.expect_expression(expr.id, result_type, expected, expr.span)?
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(
                    condition,
                    Some(self.core_type(crate::stdlib::CoreTypeId::Bool)),
                );
                let layout_constraints = self.layout_constraints(condition);
                let inverse_layout_constraints = self.inverse_layout_constraints(condition);
                let result_type =
                    expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                // `None` can either remain the zero-sized unit value or lift into
                // any optional type. When the conditional has no surrounding
                // expected type, let the value-bearing branch establish the
                // result before checking a bare `None`. This keeps inference
                // independent of which branch happens to be written first.
                let (then_type, else_type) = if expected.is_none()
                    && expression_is_bare_none(then_expr)
                    && !expression_is_bare_none(else_expr)
                {
                    let else_type = self.with_layout_constraints(
                        inverse_layout_constraints.as_deref(),
                        |checker| checker.expr(else_expr, Some(result_type)),
                    );
                    let then_type = self
                        .with_layout_constraints(layout_constraints.as_deref(), |checker| {
                            checker.expr(then_expr, Some(result_type))
                        });
                    (then_type, else_type)
                } else {
                    let then_type = self
                        .with_layout_constraints(layout_constraints.as_deref(), |checker| {
                            checker.expr(then_expr, Some(result_type))
                        });
                    let else_type = self.with_layout_constraints(
                        inverse_layout_constraints.as_deref(),
                        |checker| checker.expr(else_expr, Some(result_type)),
                    );
                    (then_type, else_type)
                };
                if expected.is_none()
                    && then_type.is_some_and(|ty| self.is_never_type(ty))
                    && else_type.is_some_and(|ty| self.is_never_type(ty))
                {
                    let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                    self.unify(result_type, never, expr.span);
                    never
                } else {
                    self.expect_expression(expr.id, result_type, expected, expr.span)?
                }
            }
            ExprKind::Fallback { value, fallback } => {
                let wrapper = self.expr(value, None)?;
                let wrapper = self.shallow_type(wrapper);
                let value_type = match wrapper {
                    Type::Option(option) => self.inference.option_value(option),
                    Type::Result(result) => self.inference.result_value(result),
                    ty => {
                        let ty = self.type_name(ty);
                        self.error(
                            format!("`else` can only unwrap `T?` or `T!`, found `{ty}`"),
                            value.span,
                        );
                        return None;
                    }
                };
                let direct_optional_return = if self.expression_mode
                    == super::context::ExpressionMode::DirectReturn
                    && matches!(wrapper, Type::Result(_))
                    && matches!(fallback.kind, ExprKind::None)
                {
                    expected.is_some_and(|expected| {
                        matches!(self.shallow_type(expected), Type::Option(_))
                    })
                } else {
                    false
                };
                if direct_optional_return {
                    self.expr(fallback, None);
                    self.errors.push(
                        Diagnostic::type_error(
                            "`else None` supplies the unwrapped fallback value, not the optional function return",
                            fallback.span,
                        )
                        .with_primary_label("return the absent optional value from this branch")
                        .with_note(
                            "write `else return None` when failure should make the enclosing function return `None`",
                        )
                        .with_machine_applicable_fix(
                            "return `None` from the fallback branch",
                            Span {
                                start: fallback.span.start,
                                end: fallback.span.start,
                            },
                            "return ",
                        ),
                    );
                } else {
                    self.expr(fallback, Some(value_type));
                }
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Break(value) => {
                match self.loops.break_target() {
                    None => {
                        if let Some(value) = value {
                            self.expr(value, None);
                        }
                        self.error("`break` is only available inside a loop", expr.span);
                    }
                    Some(super::context::BreakTarget::Statement) => {
                        if let Some(value) = value {
                            self.expr(value, None);
                            self.error(
                                "only a `loop` expression can break with a value",
                                value.span,
                            );
                        }
                        self.loops.record_break();
                    }
                    Some(super::context::BreakTarget::Value(result)) => {
                        if let Some(value) = value {
                            self.expr(value, Some(result));
                        } else {
                            let none = self.core_type(crate::stdlib::CoreTypeId::None);
                            self.unify(result, none, expr.span);
                        }
                        self.loops.record_break();
                    }
                }
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Continue => {
                if !self.loops.is_inside() {
                    self.error("`continue` is only available inside a loop", expr.span);
                }
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Return(value) => {
                self.check_return(value.as_deref(), expr.span);
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Throw(error) => {
                let boundary = self.failure.propagate();
                if let Some(boundary) = boundary {
                    self.semantics.resolve_propagation_target(
                        expr.id,
                        boundary,
                        self.failure.retry_expression(),
                    );
                } else {
                    self.error(
                        "`throw` needs `onAttach`, a fallible function, `selectProcess`, or an explicit catch boundary",
                        expr.span,
                    );
                }
                self.expr(error, Some(self.standard_type(StdlibTypeId::String)));
                let never = self.core_type(crate::stdlib::CoreTypeId::Never);
                self.expect_expression(expr.id, never, expected, expr.span)?
            }
            ExprKind::Suspend {
                mode,
                destination,
                value,
            } => {
                if self.expression_mode == super::context::ExpressionMode::SuspensionOperand {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!(
                            "`{keyword}` cannot be evaluated inside an `await` or `retry` operand"
                        ),
                        expr.span,
                    );
                }
                if !self.callable.can_suspend() {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is not available in this synchronous body"),
                        expr.span,
                    );
                }
                let (operand, retry_boundary) = match mode {
                    SuspensionMode::Await => (
                        self.with_expression_mode(
                            super::context::ExpressionMode::SuspensionOperand,
                            |checker| checker.expr(value, None),
                        )?,
                        None,
                    ),
                    SuspensionMode::Retry => {
                        let completion = expected
                            .unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                        let boundary = Type::Result(self.inference.result_type(completion));
                        // `value` is checked through the normal expression
                        // entry point. In particular, a block operand is the
                        // same generic value block used everywhere else; only
                        // its surrounding failure boundary is retry-specific.
                        let (operand, failure) = self.with_expression_mode(
                            super::context::ExpressionMode::SuspensionOperand,
                            |checker| {
                                checker.with_failure_context(
                                    super::context::FailureContext::retry(boundary, expr.id),
                                    |checker| checker.expr(value, Some(boundary)),
                                )
                            },
                        );
                        if !failure.propagated() && !failure.observed_result() {
                            self.error(
                                "`retry` expects a result value (`T!`) or synchronous fallible work using `?` or `throw`",
                                value.span,
                            );
                            return None;
                        }
                        (operand?, Some((boundary, completion)))
                    }
                };
                let result = match mode {
                    SuspensionMode::Await => match self.shallow_type(operand) {
                        Type::Async(future) => self.inference.async_value(future),
                        operand => {
                            let operand = self.type_name(operand);
                            self.error(
                                format!("`await` expects an async value, found `{operand}`"),
                                value.span,
                            );
                            return None;
                        }
                    },
                    SuspensionMode::Retry => match self.shallow_type(operand) {
                        Type::Result(result) => {
                            let value = self.inference.result_value(result);
                            let (_, completion) = retry_boundary
                                .expect("retry operands create a local failure boundary");
                            self.unify(value, completion, expr.span)?;
                            completion
                        }
                        operand => {
                            let operand = self.type_name(operand);
                            self.error(
                                format!("`retry` expects a result value (`T!`), found `{operand}`"),
                                value.span,
                            );
                            return None;
                        }
                    },
                };
                self.semantics.resolve_value_type(*destination, result);
                self.expect_expression(expr.id, result, expected, expr.span)?
            }
            ExprKind::Propagate(value) => {
                let input = self.expr(value, None)?;
                let Type::Result(input_result) = self.shallow_type(input) else {
                    self.error("`?` requires a result value (`T!`)", value.span);
                    return None;
                };
                let Some(boundary) = self.failure.propagate() else {
                    self.error(
                        "`?` needs `onAttach`, a state-field boundary, `selectProcess`, or a function returning `T!`",
                        expr.span,
                    );
                    return None;
                };
                let Type::Result(_) = self.shallow_type(boundary) else {
                    unreachable!("failure boundaries are result types")
                };
                self.semantics.resolve_propagation_target(
                    expr.id,
                    boundary,
                    self.failure.retry_expression(),
                );
                let value_type = self.inference.result_value(input_result);
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Path(path) => {
                if let Some(enumeration) = self.resolutions.expression_enum(expr.id) {
                    let variant = path
                        .last()
                        .expect("resolved enum paths retain a variant segment");
                    self.enum_constructor(expr.id, enumeration, variant, &[], expected, expr.span)?
                } else if let [name] = path.as_slice()
                    && self.binding(name).is_none()
                    && let Some(signature) = self.declarations.functions.get(name).cloned()
                {
                    let signature = if self.active_function_component.contains(&signature.id) {
                        signature.monomorphic_call()
                    } else {
                        signature.instantiate(&mut self.inference)
                    };
                    let callable = Type::Callable(
                        self.inference
                            .callable_type(signature.params.clone(), signature.result),
                    );
                    self.semantics.resolve_function_value(
                        expr.id,
                        PendingFunctionValue {
                            function: signature.id,
                            type_arguments: signature.type_arguments,
                            signature: signature
                                .params
                                .iter()
                                .copied()
                                .chain([signature.result])
                                .collect(),
                        },
                    );
                    self.expect_expression(expr.id, callable, expected, expr.span)?
                } else {
                    let resolution = self.path(path, expr.span, Some(expr.id))?;
                    if let Some(value) = resolution.value {
                        self.semantics.resolve_value(expr.id, value);
                    }
                    if let Some(members) = resolution.members {
                        self.semantics.resolve_path_members(expr.id, members);
                    }
                    self.expect_expression(expr.id, resolution.ty, expected, expr.span)?
                }
            }
            ExprKind::Member {
                receiver,
                name,
                name_span,
            } => {
                let receiver_ty = self.expr(receiver, None)?;
                let (ty, members) = self.resolve_members_or_defer(
                    receiver_ty,
                    std::slice::from_ref(name),
                    *name_span,
                    Some(expr.id),
                )?;
                if let Some(members) = members {
                    self.semantics.resolve_path_members(expr.id, members);
                }
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Index {
                receiver,
                index,
                bracket_span,
            } => {
                let element = self.indexed_element_type(receiver, index, *bracket_span)?;
                self.expect_expression(expr.id, element, expected, expr.span)?
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnaryOp::Not => {
                    let operand_hint = expected.map(|ty| self.expected_value_type(ty));
                    let inner_ty = self.expr(inner, operand_hint)?;
                    let result = self
                        .resolve_unary_operator(*op, inner_ty, expr.id, inner.id, expr.span)
                        .unwrap_or_else(|| self.error_type());
                    self.expect_expression(expr.id, result, expected, expr.span)?
                }
                UnaryOp::Neg => {
                    let operand_hint = expected.map(|ty| self.expected_value_type(ty));
                    let inner_ty = self.expr(inner, operand_hint)?;
                    if let Some(result) =
                        self.resolve_unary_operator(*op, inner_ty, expr.id, inner.id, expr.span)
                    {
                        self.expect_expression(expr.id, result, expected, expr.span)?
                    } else {
                        self.require(
                            inner_ty,
                            Requirements::capabilities([
                                StdlibCapabilityId::Numeric,
                                StdlibCapabilityId::Signed,
                            ]),
                            expr.span,
                        )?;
                        self.expect_expression(expr.id, inner_ty, expected, expr.span)?
                    }
                }
            },
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let source = self.expr(inner, None)?;
                let target = self.syntax_type(*target);
                if target == self.core_type(crate::stdlib::CoreTypeId::U32)
                    && source == self.core_type(crate::stdlib::CoreTypeId::Char)
                {
                    // Unicode scalar values convert losslessly to their code point.
                } else if self.inference.is_numeric(target) {
                    self.require(
                        source,
                        Requirements::capability(StdlibCapabilityId::Numeric),
                        expr.span,
                    )?;
                } else if target == self.standard_type(StdlibTypeId::String) {
                    self.require(
                        source,
                        Requirements::capability(StdlibCapabilityId::Display),
                        expr.span,
                    )?;
                } else {
                    let target_name = self.type_name(target);
                    self.error(
                        format!("`as` cannot convert a value to `{target_name}`"),
                        expr.span,
                    );
                    return None;
                }
                self.expect_expression(expr.id, target, expected, expr.span)?
            }
            ExprKind::Binary { op, left, right } => {
                self.binary(*op, left, right, expected, expr.id, expr.span)?
            }
            ExprKind::Call {
                callee,
                name_span,
                receiver,
                type_arguments,
                args,
                ..
            } => {
                if receiver.is_none()
                    && type_arguments.is_empty()
                    && let [name] = callee.as_slice()
                    && self.binding(name).is_some()
                {
                    let binding = self.binding_for_use(name, *name_span)?;
                    let callable = match self.shallow_type(binding.ty) {
                        Type::Callable(callable) => callable,
                        Type::Variable(_) => {
                            // Calling an otherwise unconstrained parameter is itself
                            // enough information to infer a callable type. The
                            // parameter and result variables become part of the
                            // surrounding function's generalized signature.
                            let parameters = (0..args.len())
                                .map(|_| self.fresh_inference(Requirements::none(), None))
                                .collect::<Vec<_>>();
                            let result = expected.unwrap_or_else(|| {
                                self.fresh_inference(Requirements::none(), None)
                            });
                            let callable = self.inference.callable_type(parameters, result);
                            self.unify(binding.ty, Type::Callable(callable), *name_span)?;
                            callable
                        }
                        _ => {
                            let actual = self.type_name(binding.ty);
                            self.error(
                                format!("`{name}` is not callable; found `{actual}`"),
                                *name_span,
                            );
                            return None;
                        }
                    };
                    self.semantics.resolve_dynamic_call(
                        expr.id,
                        crate::semantic::DynamicCallCallee::Value(
                            binding.id.expect("local callable bindings have identities"),
                        ),
                    );
                    self.invoke_callable_type(callable, args, expected, expr.id, expr.span)?
                } else if let Some(enumeration) = self.resolutions.expression_enum(expr.id) {
                    debug_assert!(receiver.is_none());
                    if !type_arguments.is_empty() {
                        self.error("enum variants do not accept type arguments", expr.span);
                        return None;
                    }
                    let variant = callee
                        .last()
                        .expect("resolved enum constructors retain a variant segment");
                    self.enum_constructor(expr.id, enumeration, variant, args, expected, expr.span)?
                } else {
                    self.call(
                        super::CallSyntax {
                            callee,
                            name_span: *name_span,
                            postfix_receiver: receiver.as_deref(),
                            type_arguments,
                        },
                        args,
                        expected,
                        expr.id,
                        expr.span,
                    )?
                }
            }
            ExprKind::Invoke { callee, args } => {
                self.invoke_expression(callee, args, expected, expr.id, expr.span)?
            }
            ExprKind::Closure {
                params,
                return_annotation,
                return_annotation_span,
                body,
                ..
            } => {
                let hinted = expected
                    .map(|ty| self.expected_value_type(ty))
                    .map(|ty| self.shallow_type(ty))
                    .and_then(|ty| match ty {
                        Type::Callable(callable) => Some((
                            self.inference.callable_parameters(callable).to_vec(),
                            self.inference.callable_result(callable),
                        )),
                        _ => None,
                    });
                if let Some((parameters, _)) = &hinted
                    && parameters.len() != params.len()
                {
                    self.error(
                        format!(
                            "closure expects {} parameters from context, but declares {}",
                            parameters.len(),
                            params.len()
                        ),
                        expr.span,
                    );
                    return None;
                }
                let parameter_types = params
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                        let annotation = parameter.annotation.map(|ty| self.syntax_type(ty));
                        let contextual = hinted.as_ref().map(|(parameters, _)| parameters[index]);
                        match (annotation, contextual) {
                            (Some(annotation), Some(contextual)) => self
                                .unify(annotation, contextual, parameter.span)
                                .unwrap_or(annotation),
                            (Some(annotation), None) => annotation,
                            (None, Some(contextual)) => contextual,
                            (None, None) => self.fresh_inference(Requirements::none(), None),
                        }
                    })
                    .collect::<Vec<_>>();
                let annotated_result = return_annotation.map(|ty| self.syntax_type(ty));
                let annotated_completion =
                    annotated_result.map(|result| match self.shallow_type(result) {
                        Type::Async(future) => self.inference.async_value(future),
                        result => result,
                    });
                let completion = annotated_completion
                    .or_else(|| {
                        hinted
                            .as_ref()
                            .map(|(_, result)| match self.shallow_type(*result) {
                                Type::Async(future) => self.inference.async_value(future),
                                result => result,
                            })
                    })
                    .unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
                let is_async = annotated_result
                    .is_some_and(|result| matches!(self.shallow_type(result), Type::Async(_)))
                    || crate::typeck::control_flow::expression_contains_suspension(body);
                let result = if is_async {
                    match annotated_result {
                        Some(result @ Type::Async(_)) => result,
                        _ => Type::Async(self.inference.async_type(completion)),
                    }
                } else {
                    completion
                };
                if let Some(annotation) = annotated_result {
                    self.unify(
                        result,
                        annotation,
                        return_annotation_span.unwrap_or(expr.span),
                    );
                }
                if let Some((_, expected_result)) = hinted {
                    self.unify(result, expected_result, expr.span);
                }

                self.scopes.push(HashMap::new());
                for (parameter, ty) in params.iter().zip(parameter_types.iter().copied()) {
                    self.semantics.resolve_value_type(parameter.id, ty);
                    let duplicate = self.scopes.last_mut().unwrap().insert(
                        parameter.name.clone(),
                        Binding {
                            id: Some(parameter.id),
                            ty,
                            mutable: true,
                            debug_only: self.debug_context.is_debug(),
                            declaration_span: Some(parameter.name_span),
                        },
                    );
                    if duplicate.is_some() {
                        self.error(
                            format!("duplicate closure parameter `{}`", parameter.name),
                            parameter.name_span,
                        );
                    }
                }
                let failure = match self.shallow_type(completion) {
                    result @ Type::Result(_) => super::context::FailureContext::boundary(result),
                    _ => super::context::FailureContext::None,
                };
                let return_type_source = return_annotation_span.map(|span| {
                    let result = self.type_name(completion);
                    super::ExpectedTypeSource {
                        span,
                        label: format!("closure is declared to return `{result}`"),
                    }
                });
                self.with_return_type_source(return_type_source.clone(), |checker| {
                    checker.with_callable_context(
                        super::context::CallableContext::Closure,
                        completion,
                        failure,
                        |checker| {
                            if let Some(source) = return_type_source {
                                checker.with_expected_type_source(source, |checker| {
                                    checker.expr(body, Some(completion));
                                });
                            } else {
                                checker.expr(body, Some(completion));
                            }
                        },
                    );
                });
                self.scopes.pop();
                let callable =
                    Type::Callable(self.inference.callable_type(parameter_types, result));
                self.expect_expression(expr.id, callable, expected, expr.span)?
            }
        };
        self.semantics.resolve_expression_type(expr.id, ty);
        Some(ty)
    }

    /// Whether a contextual wrapper type has an optional success value to
    /// which a bare `None` can flow.
    ///
    /// Result layers are transparent here because an ordinary value is lifted
    /// into their successful branch. The first option layer consumes `None` as
    /// absence. This lets implicit failure boundaries contextualize `None`
    /// without inventing a special state-field conversion.
    fn expected_accepts_none(&mut self, expected: Type) -> bool {
        match self.shallow_type(expected) {
            Type::Option(_) => true,
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                self.expected_accepts_none(value)
            }
            _ => false,
        }
    }

    fn invoke_expression(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let parameters = (0..arguments.len())
            .map(|_| self.fresh_inference(Requirements::none(), None))
            .collect::<Vec<_>>();
        let result = expected.unwrap_or_else(|| self.fresh_inference(Requirements::none(), None));
        let callable = Type::Callable(self.inference.callable_type(parameters.clone(), result));
        let callee_type = self.expr(callee, Some(callable))?;
        self.unify(callee_type, callable, callee.span)?;
        for (argument, parameter) in arguments.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        // The backend resolves this expression as a dynamic call after capture
        // analysis has selected its closure representation.
        self.semantics.resolve_dynamic_call(
            expression,
            crate::semantic::DynamicCallCallee::Expression(callee.id),
        );
        self.expect_expression(expression, result, expected, span)
    }

    fn invoke_callable_type(
        &mut self,
        callable: crate::ast::CallableTypeId,
        arguments: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let parameters = self.inference.callable_parameters(callable).to_vec();
        if parameters.len() != arguments.len() {
            self.error(
                format!(
                    "callable expects {} arguments, found {}",
                    parameters.len(),
                    arguments.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in arguments.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        let result = self.inference.callable_result(callable);
        self.expect_expression(expression, result, expected, span)
    }

    fn enum_constructor(
        &mut self,
        expression: ExprId,
        enumeration: EnumTypeId,
        variant: &str,
        arguments: &[Expr],
        expected: Option<Type>,
        span: Span,
    ) -> Option<Type> {
        let Some(declaration) = self.enum_info(enumeration) else {
            self.error("unknown enum type", span);
            return None;
        };
        let Some(declared_variant) = declaration
            .variants
            .iter()
            .find(|declared| declared.name == variant)
        else {
            self.error(
                format!("enum `{}` has no variant `{variant}`", declaration.name),
                span,
            );
            return None;
        };
        self.semantics
            .resolve_enum_variant(expression, declared_variant.id);
        match (declared_variant.payload, arguments.first()) {
            (Some(payload_type), Some(payload)) => {
                if let Some(source_span) = declared_variant.source_span {
                    let payload_type_name = self.type_name(payload_type);
                    self.with_expected_type_source(
                        super::ExpectedTypeSource {
                            span: source_span,
                            label: format!(
                                "variant `{}.{variant}` declares a payload of type `{payload_type_name}`",
                                declaration.name,
                            ),
                        },
                        |checker| checker.expr(payload, Some(payload_type)),
                    );
                } else {
                    self.expr(payload, Some(payload_type));
                }
            }
            (Some(_), None) => self.error(format!("variant `{variant}` requires a payload"), span),
            (None, Some(payload)) => {
                self.expr(payload, None);
                self.error(
                    format!("variant `{variant}` does not accept a payload"),
                    span,
                );
            }
            (None, None) => {}
        }
        for extra in arguments.iter().skip(1) {
            self.expr(extra, None);
        }
        self.expect_expression(expression, self.enum_type(enumeration), expected, span)
    }

    fn infer_option_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<crate::ast::OptionTypeId> {
        match self.shallow_type(value_type) {
            Type::Option(option) => Some(option),
            Type::Variable(_) => {
                let value = self.fresh_inference(Requirements::none(), None);
                let option = self.inference.option_type(value);
                self.unify(value_type, Type::Option(option), span)?;
                Some(option)
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn infer_result_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<crate::ast::ResultTypeId> {
        match self.shallow_type(value_type) {
            Type::Result(result) => Some(result),
            Type::Variable(_) => {
                let value = self.fresh_inference(Requirements::none(), None);
                let result = self.inference.result_type(value);
                self.unify(value_type, Type::Result(result), span)?;
                Some(result)
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn infer_iterator_step_pattern(
        &mut self,
        value_type: Type,
        span: Span,
        requirement: &str,
    ) -> Option<(crate::ast::TypeApplicationId, Type)> {
        match self.shallow_type(value_type) {
            Type::Application(step)
                if self.inference.application_constructor(step)
                    == crate::stdlib::StdlibTypeConstructorId::IteratorStep =>
            {
                Some((step, self.inference.application_arguments(step)[0]))
            }
            Type::Variable(_) => {
                let item = self.fresh_inference(Requirements::none(), None);
                let step = self.inference.application_type(
                    crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                    vec![item],
                );
                self.unify(value_type, Type::Application(step), span)?;
                Some((step, item))
            }
            ty => {
                let ty = self.type_name(ty);
                self.error(format!("{requirement}, found `{ty}`"), span);
                None
            }
        }
    }

    fn check_array_pattern(
        &mut self,
        elements: &[crate::ast::PatternNode],
        pattern_id: crate::ast::PatternId,
        value_type: Type,
        span: Span,
    ) -> CheckedPattern {
        let Type::Array(array) = self.shallow_type(value_type) else {
            let ty = self.type_name(value_type);
            self.error(
                format!("an array pattern requires an array value, found `{ty}`"),
                span,
            );
            return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
        };
        let element_type = self.inference.array_element(array);
        let fixed_length = self.inference.array_length(array);
        if let Some(length) = fixed_length
            && elements.len() != length as usize
        {
            let array_name = self.type_name(value_type);
            self.error(
                format!(
                    "array pattern has {} elements, but `{array_name}` requires exactly {length}",
                    elements.len()
                ),
                span,
            );
        }
        let mut coverage = Vec::with_capacity(elements.len());
        let mut all_irrefutable = true;
        for element in elements {
            let checked = self.check_pattern(&element.kind, element.id, element_type, element.span);
            all_irrefutable &= checked.coverage.is_irrefutable();
            coverage.push(checked.coverage);
        }
        CheckedPattern::new(
            if fixed_length == Some(elements.len() as u32) && all_irrefutable {
                PatternCoverage::Irrefutable
            } else {
                PatternCoverage::Array(coverage)
            },
        )
    }

    fn check_alternation_pattern(
        &mut self,
        alternatives: &[crate::ast::PatternNode],
        value_type: Type,
    ) -> CheckedPattern {
        let base_scope = self
            .scopes
            .last()
            .expect("patterns are checked inside an arm scope")
            .clone();
        let mut expected_names = None::<HashSet<String>>;
        let mut arm_bindings = HashMap::<String, Binding>::new();
        let mut coverage = Vec::with_capacity(alternatives.len());
        let mut layout_variants = Some(HashSet::new());

        for alternative in alternatives {
            *self.scopes.last_mut().unwrap() = base_scope.clone();
            let checked = self.check_pattern(
                &alternative.kind,
                alternative.id,
                value_type,
                alternative.span,
            );
            if coverage
                .iter()
                .any(|previous: &PatternCoverage| previous.covers(&checked.coverage))
            {
                self.error(
                    format!("unreachable alternative `{}`", checked.coverage.display()),
                    alternative.span,
                );
            }

            let mut names = HashSet::new();
            alternative.kind.visit_bindings(&mut |binding| {
                names.insert(binding.name.clone());
            });
            if let Some(expected) = &expected_names {
                for missing in expected.difference(&names) {
                    self.error(
                        format!("this alternative does not bind `{missing}`"),
                        alternative.span,
                    );
                }
                for extra in names.difference(expected) {
                    self.error(
                        format!(
                            "this alternative binds `{extra}`, but the other alternatives do not"
                        ),
                        alternative.span,
                    );
                }
            } else {
                expected_names = Some(names.clone());
                for name in &names {
                    if let Some(binding) = self.scopes.last().unwrap().get(name).copied() {
                        arm_bindings.insert(name.clone(), binding);
                    }
                }
            }

            match (&mut layout_variants, checked.layout_variants) {
                (Some(all), Some(current)) => all.extend(current),
                (slot, None) => *slot = None,
                (None, Some(_)) => {}
            }
            coverage.push(checked.coverage);
        }

        *self.scopes.last_mut().unwrap() = base_scope;
        self.scopes.last_mut().unwrap().extend(arm_bindings);

        let coverage = if coverage.iter().any(PatternCoverage::is_irrefutable) {
            PatternCoverage::Irrefutable
        } else {
            PatternCoverage::Alternation(coverage)
        };
        CheckedPattern {
            coverage,
            layout_variants,
        }
    }

    fn check_pattern(
        &mut self,
        pattern: &MatchPattern,
        pattern_id: crate::ast::PatternId,
        value_type: Type,
        span: Span,
    ) -> CheckedPattern {
        match pattern {
            MatchPattern::Struct { name, fields, .. } => {
                let Some(declaration) = self
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.name == *name)
                    .cloned()
                else {
                    self.error(format!("unknown struct type `{name}`"), span);
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.unify(value_type, self.struct_type(declaration.id), span);
                self.semantics
                    .resolve_struct_pattern(pattern_id, declaration.id);
                let mut seen = HashSet::new();
                let mut resolved_fields = Vec::with_capacity(fields.len());
                let mut coverage = Vec::with_capacity(fields.len());
                let mut all_irrefutable = true;
                for pattern_field in fields {
                    if !seen.insert(pattern_field.name.clone()) {
                        self.error(
                            format!("duplicate struct pattern field `{}`", pattern_field.name),
                            pattern_field.name_span,
                        );
                        continue;
                    }
                    let Some(field) = declaration
                        .fields
                        .iter()
                        .find(|field| field.name == pattern_field.name)
                    else {
                        self.error(
                            format!(
                                "struct `{}` has no field `{}`",
                                declaration.name, pattern_field.name
                            ),
                            pattern_field.name_span,
                        );
                        continue;
                    };
                    let checked = self.check_pattern(
                        &pattern_field.pattern.kind,
                        pattern_field.pattern.id,
                        self.syntax_type(field.ty),
                        pattern_field.pattern.span,
                    );
                    all_irrefutable &= checked.coverage.is_irrefutable();
                    resolved_fields.push(field.id);
                    coverage.push((field.id, field.name.clone(), checked.coverage));
                }
                self.semantics
                    .resolve_struct_pattern_fields(pattern_id, resolved_fields);
                // Field order is not semantically significant. Canonicalize
                // coverage so duplicate and unreachable-arm diagnostics do
                // not depend on the order chosen in source.
                coverage.sort_by_key(|(field, _, _)| *field);
                CheckedPattern::new(if all_irrefutable {
                    PatternCoverage::Irrefutable
                } else {
                    PatternCoverage::Struct {
                        structure: declaration.id,
                        name: declaration.name,
                        fields: coverage,
                    }
                })
            }
            MatchPattern::Enum {
                variant, payload, ..
            } => {
                let Some(enumeration) = self.resolutions.pattern_enum(pattern_id) else {
                    self.error("unresolved enum type", span);
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.unify(value_type, self.enum_type(enumeration), span);
                let mut resolved_variant = None;
                let mut declared_payload = None;
                if let Some(declaration) = self.enum_info(enumeration) {
                    if let Some(declared_variant) = declaration
                        .variants
                        .iter()
                        .find(|declared| declared.name == *variant)
                    {
                        resolved_variant = Some(declared_variant.id);
                        declared_payload = declared_variant.payload;
                    } else {
                        self.error(
                            format!("enum `{}` has no variant `{variant}`", declaration.name),
                            span,
                        );
                    }
                } else {
                    self.error("unknown enum type", span);
                }
                let Some(resolved_variant) = resolved_variant else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics
                    .resolve_pattern_variant(pattern_id, resolved_variant);
                let payload_coverage = match (declared_payload, payload) {
                    (Some(payload_type), Some(payload)) => {
                        self.check_pattern(&payload.kind, payload.id, payload_type, payload.span)
                            .coverage
                    }
                    (None, Some(payload)) => {
                        self.error(
                            format!("variant `{variant}` has no payload to match"),
                            payload.span,
                        );
                        PatternCoverage::Invalid(payload.id)
                    }
                    _ => PatternCoverage::Irrefutable,
                };
                CheckedPattern {
                    coverage: PatternCoverage::Enum {
                        variant: resolved_variant,
                        name: variant.clone(),
                        payload: Box::new(payload_coverage),
                    },
                    layout_variants: match resolved_variant {
                        ResolvedEnumVariantId::Source(variant) => Some(HashSet::from([variant])),
                        ResolvedEnumVariantId::Standard(_) => None,
                    },
                }
            }
            MatchPattern::Bool(value) => {
                self.unify(
                    value_type,
                    self.core_type(crate::stdlib::CoreTypeId::Bool),
                    span,
                );
                CheckedPattern::new(PatternCoverage::Bool(*value))
            }
            MatchPattern::Char(value) => {
                self.unify(
                    value_type,
                    self.core_type(crate::stdlib::CoreTypeId::Char),
                    span,
                );
                CheckedPattern::new(PatternCoverage::Char(*value))
            }
            MatchPattern::String(value) => {
                self.unify(value_type, self.standard_type(StdlibTypeId::String), span);
                CheckedPattern::new(PatternCoverage::String(value.clone()))
            }
            MatchPattern::Int {
                value,
                negative,
                suffix,
            } => {
                let pattern_type = if let Some(suffix) = suffix {
                    if !suffix.is_integer() {
                        self.error("integer match patterns require an integer type", span);
                    } else if !(if *negative {
                        self.inference
                            .fits_negative_literal(*value, self.syntax_type(*suffix))
                    } else {
                        self.inference
                            .fits_unsigned_literal(*value, self.syntax_type(*suffix))
                    }) {
                        self.error(format!("integer literal does not fit in `{suffix}`"), span);
                    }
                    self.syntax_type(*suffix)
                } else {
                    if *negative {
                        self.inference.fresh_negative_integer_literal(
                            Requirements::capability(StdlibCapabilityId::Integer),
                            *value,
                        )
                    } else {
                        self.fresh_inference(
                            Requirements::capability(StdlibCapabilityId::Integer),
                            Some(*value),
                        )
                    }
                };
                self.unify(value_type, pattern_type, span);
                CheckedPattern::new(PatternCoverage::Int {
                    value: *value,
                    negative: *negative,
                })
            }
            MatchPattern::FileVersion(components) => {
                self.unify(
                    value_type,
                    self.standard_type(StdlibTypeId::FileVersion),
                    span,
                );
                CheckedPattern::new(PatternCoverage::FileVersion(*components))
            }
            MatchPattern::None => {
                let Some(option) = self.infer_option_pattern(
                    value_type,
                    span,
                    "a `None` pattern requires an optional value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::OptionNone(option),
                );
                CheckedPattern::new(PatternCoverage::OptionNone)
            }
            MatchPattern::OptionSome(payload) => {
                let Some(option) = self.infer_option_pattern(
                    value_type,
                    span,
                    "a `Some(value)` pattern requires an optional value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::OptionSome(option),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.inference.option_value(option),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::OptionSome(Box::new(checked.coverage)))
            }
            MatchPattern::IteratorEnd => {
                let Some((step, _)) = self.infer_iterator_step_pattern(
                    value_type,
                    span,
                    "an `End` pattern requires an iterator step",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics
                    .resolve_wrapper_pattern(pattern_id, ResolvedWrapperPattern::IteratorEnd(step));
                CheckedPattern::new(PatternCoverage::IteratorEnd)
            }
            MatchPattern::IteratorItem(payload) => {
                let Some((step, item)) = self.infer_iterator_step_pattern(
                    value_type,
                    span,
                    "an `Item(value)` pattern requires an iterator step",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::IteratorItem(step),
                );
                let checked = self.check_pattern(&payload.kind, payload.id, item, payload.span);
                CheckedPattern::new(PatternCoverage::IteratorItem(Box::new(checked.coverage)))
            }
            MatchPattern::ResultSuccess(payload) => {
                let Some(result) = self.infer_result_pattern(
                    value_type,
                    span,
                    "an `Ok(value)` pattern requires a result value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::ResultSuccess(result),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.inference.result_value(result),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::ResultSuccess(Box::new(checked.coverage)))
            }
            MatchPattern::ResultError(payload) => {
                let Some(result) = self.infer_result_pattern(
                    value_type,
                    span,
                    "an `Err(error)` pattern requires a result value",
                ) else {
                    return CheckedPattern::new(PatternCoverage::Invalid(pattern_id));
                };
                self.semantics.resolve_wrapper_pattern(
                    pattern_id,
                    ResolvedWrapperPattern::ResultError(result),
                );
                let checked = self.check_pattern(
                    &payload.kind,
                    payload.id,
                    self.standard_type(StdlibTypeId::String),
                    payload.span,
                );
                CheckedPattern::new(PatternCoverage::ResultError(Box::new(checked.coverage)))
            }
            MatchPattern::Array(elements) => {
                self.check_array_pattern(elements, pattern_id, value_type, span)
            }
            MatchPattern::Alternation(alternatives) => {
                self.check_alternation_pattern(alternatives, value_type)
            }
            MatchPattern::Binding(binding) => {
                self.bind_pattern_value(binding, value_type, span);
                CheckedPattern::new(PatternCoverage::Irrefutable)
            }
            MatchPattern::Wildcard => CheckedPattern::new(PatternCoverage::Irrefutable),
        }
    }

    pub(super) fn binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if matches!(op, BinaryOp::Or | BinaryOp::And) {
            let bool_type = self.core_type(crate::stdlib::CoreTypeId::Bool);
            self.expr(left, Some(bool_type));
            let constraints = match op {
                BinaryOp::And => self.truthy_layout_constraints(left),
                BinaryOp::Or => self.falsy_layout_constraints(left),
                _ => unreachable!("logical operators were matched above"),
            };
            self.with_layout_constraints(Some(&constraints), |checker| {
                checker.expr(right, Some(bool_type));
            });
            return self.expect_expression(expression, bool_type, expected, span);
        }

        let result_is_bool = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        let operand_hint = if result_is_bool {
            None
        } else {
            expected.map(|ty| self.expected_value_type(ty))
        };
        // `None` has no payload from which to infer its `T?` type. Equality,
        // however, supplies exactly that context through the opposite operand.
        // Check the informative side first without otherwise making binary
        // expression inference depend on operand order.
        let (left_ty, right_ty) = match (&left.kind, &right.kind) {
            (ExprKind::None, ExprKind::None) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, operand_hint)?;
                (left_ty, right_ty)
            }
            (ExprKind::None, _) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let right_ty = self.expr(right, operand_hint)?;
                let left_ty = self.expr(left, Some(right_ty))?;
                (left_ty, right_ty)
            }
            (_, ExprKind::None) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, Some(left_ty))?;
                (left_ty, right_ty)
            }
            (ExprKind::Array(_), _) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let right_ty = self.expr(right, operand_hint)?;
                let left_ty = self.expr(left, Some(right_ty))?;
                (left_ty, right_ty)
            }
            (_, ExprKind::Array(_)) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                let left_ty = self.expr(left, operand_hint)?;
                let right_ty = self.expr(right, Some(left_ty))?;
                (left_ty, right_ty)
            }
            _ => {
                let left_ty = self.expr(left, operand_hint)?;
                // A catalog-defined operator may accept a different right-hand
                // type, such as `Instant + Duration`. The expected result can
                // guide the receiver; the operator declaration guides this
                // expression once a candidate has been selected.
                let right_ty = self.expr(right, None)?;
                (left_ty, right_ty)
            }
        };

        if self.is_error_type(left_ty) || self.is_error_type(right_ty) {
            let result = if result_is_bool {
                self.core_type(crate::stdlib::CoreTypeId::Bool)
            } else {
                self.error_type()
            };
            return self.expect_expression(expression, result, expected, span);
        }

        if self.diagnose_string_addition(op, left_ty, right_ty, span) {
            let result = self.error_type();
            return self.expect_expression(expression, result, expected, span);
        }

        if let Some(result) =
            self.resolve_binary_operator(op, left_ty, right_ty, expression, left.id, span)
        {
            return self.expect_expression(expression, result, expected, span);
        }

        let operand_ty = self.unify(left_ty, right_ty, span)?;

        self.require_binary_operand(op, operand_ty, span)?;

        let result = if result_is_bool {
            self.core_type(crate::stdlib::CoreTypeId::Bool)
        } else {
            operand_ty
        };
        self.expect_expression(expression, result, expected, span)
    }

    pub(super) fn diagnose_string_addition(
        &mut self,
        op: BinaryOp,
        left: Type,
        right: Type,
        span: Span,
    ) -> bool {
        if op != BinaryOp::Add {
            return false;
        }
        let left = self.shallow_type(left);
        let right = self.shallow_type(right);
        if self.standard_type_id(left) != Some(StdlibTypeId::String)
            && self.standard_type_id(right) != Some(StdlibTypeId::String)
        {
            return false;
        }

        self.errors.push(
            Diagnostic::type_error("`+` does not concatenate strings in SplitScript", span)
                .with_primary_label("construct the string explicitly")
                .with_note(
                    "use a template literal such as `{left}{right}` when joining values; interpolation accepts any `Display` value",
                )
                .with_note(
                    "use `String.concat(values)` when the inputs are already stored in a `[String]`",
                )
                .with_note(
                    "no automatic rewrite is offered because the intended text, separators, and evaluation grouping belong to the author",
                ),
        );
        true
    }

    pub(super) fn diagnose_string_compound_assignment(
        &mut self,
        op: BinaryOp,
        left: Type,
        right: Type,
        span: Span,
    ) -> bool {
        let left = self.shallow_type(left);
        self.standard_type_id(left) == Some(StdlibTypeId::String)
            && self.diagnose_string_addition(op, left, right, span)
    }

    pub(super) fn require_binary_operand(
        &mut self,
        op: BinaryOp,
        operand_ty: Type,
        span: Span,
    ) -> Option<()> {
        match op {
            BinaryOp::Eq | BinaryOp::Ne => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Equatable),
                span,
            ),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Numeric),
                span,
            ),
            BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::Shl
            | BinaryOp::Shr => self.require(
                operand_ty,
                Requirements::capability(StdlibCapabilityId::Integer),
                span,
            ),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => self
                .require(
                    operand_ty,
                    if op == BinaryOp::Rem {
                        Requirements::capability(StdlibCapabilityId::Integer)
                    } else {
                        Requirements::capability(StdlibCapabilityId::Numeric)
                    },
                    span,
                ),
            BinaryOp::Or | BinaryOp::And => {
                unreachable!("logical operators are checked separately")
            }
        }
    }

    pub(super) fn indexed_element_type(
        &mut self,
        receiver: &Expr,
        index: &Expr,
        bracket_span: Span,
    ) -> Option<Type> {
        let receiver_ty = self.expr(receiver, None)?;
        if self.is_error_type(receiver_ty) {
            self.expr(index, None);
            return Some(receiver_ty);
        }
        if self.diagnose_unhandled_result(receiver_ty, "indexing it", bracket_span) {
            self.expr(index, None);
            return None;
        }
        if matches!(
            self.shallow_type(receiver_ty),
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
        ) {
            self.expr(index, Some(self.standard_type(StdlibTypeId::String)));
            let metadata = migration_diagnostic(ASL_SETTINGS_LOOKUP_DIAGNOSTIC)
                .expect("type checker migration diagnostic IDs must exist");
            let opening = Span {
                start: bracket_span.start,
                end: bracket_span.start + 1,
            };
            let closing = Span {
                start: bracket_span.end - 1,
                end: bracket_span.end,
            };
            let mut diagnostic = Diagnostic::type_error(metadata.message, bracket_span)
                .with_primary_label(metadata.primary_label)
                .with_migration_topic(metadata.concept.as_str())
                .with_fix(DiagnosticFix {
                    title: "replace indexed lookup with `enabled`".to_owned(),
                    applicability: FixApplicability::MachineApplicable,
                    edits: vec![
                        TextEdit {
                            span: opening,
                            replacement: ".enabled(".to_owned(),
                        },
                        TextEdit {
                            span: closing,
                            replacement: ")".to_owned(),
                        },
                    ],
                });
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return None;
        }
        let element = match self.shallow_type(receiver_ty) {
            Type::Array(array) => self.inference.array_element(array),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                crate::types::TypeKind::Array { element, .. } => Type::Known(*element),
                _ => {
                    let actual = self.type_name(receiver_ty);
                    self.error(
                        format!("type `{actual}` cannot be indexed; expected an array"),
                        bracket_span,
                    );
                    return None;
                }
            },
            Type::Variable(variable)
                if self.inference.variable_requirements(variable).is_empty() =>
            {
                let element = self.fresh_inference(Requirements::none(), None);
                let array = Type::Array(self.array_type_id(element));
                self.unify(receiver_ty, array, receiver.span)?;
                element
            }
            _ => {
                let actual = self.type_name(receiver_ty);
                self.error(
                    format!("type `{actual}` cannot be indexed; expected an array"),
                    bracket_span,
                );
                return None;
            }
        };
        let u32_type = self.core_type(crate::stdlib::CoreTypeId::U32);
        self.expr(index, Some(u32_type));
        Some(element)
    }
}

pub(super) fn expression_is_bare_none(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::None => true,
        ExprKind::Block(block) => block
            .statements
            .last()
            .is_none_or(|statement| match statement {
                crate::ast::Stmt::Expression(value) => expression_is_bare_none(value),
                _ => true,
            }),
        _ => false,
    }
}
