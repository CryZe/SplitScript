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
        ResolvedEnumVariantId, ResolvedRecordFieldId, ResolvedRecordId, ResolvedWrapperPattern,
    },
    signature::parse_signature,
    stdlib::{Implementation, RuntimeRepresentation, StdlibCapabilityId, StdlibTypeId},
    types::{EnumTypeId, TypeKind},
};

use super::{Checker, context::NonePolicy, declarations::Binding};

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
                    Some(expected @ Type::Option(_)) => {
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
                    Some(expected) => {
                        self.expect_expression(expr.id, none, Some(expected), expr.span)?
                    }
                    None => none,
                }
            }
            ExprKind::Bool(_) => self.expect_expression(
                expr.id,
                self.core_type(crate::stdlib::CoreTypeId::Bool),
                expected,
                expr.span,
            )?,
            ExprKind::Int { value, suffix } => {
                let ty = if let Some(suffix) = suffix {
                    let suffix = self.syntax_type(*suffix);
                    if !self.inference.fits_unsigned_literal(*value, suffix) {
                        self.error(
                            format!("integer literal does not fit in `{suffix}`"),
                            expr.span,
                        );
                        return None;
                    }
                    suffix
                } else {
                    self.fresh_inference(
                        Requirements::capability(StdlibCapabilityId::Numeric),
                        Some(*value),
                    )
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
                        Some(expected @ Type::Option(_)) => {
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
            ExprKind::Record { name, fields, .. } => {
                let declaration = self
                    .declarations
                    .records
                    .iter()
                    .find(|declaration| declaration.name == *name)
                    .cloned();
                if let Some(declaration) = declaration {
                    self.semantics
                        .resolve_record_literal(expr.id, ResolvedRecordId::Source(declaration.id));
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for (name, value) in fields {
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate record field `{name}`"), value.span);
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
                                        "record field `{}.{}` is declared as `{field_type_name}`",
                                        declaration.name, field.name,
                                    ),
                                },
                                |checker| checker.expr(value, Some(field_type)),
                            );
                            resolved_fields.push(ResolvedRecordFieldId::Source(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("record `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_record_literal_fields(expr.id, resolved_fields);
                    for field in &declaration.fields {
                        if !seen.contains(&field.name) {
                            self.error(
                                format!(
                                    "record `{}` initializer is missing field `{}`",
                                    declaration.name, field.name
                                ),
                                expr.span,
                            );
                        }
                    }
                    self.expect_expression(
                        expr.id,
                        self.record_type(declaration.id),
                        expected,
                        expr.span,
                    )?
                } else if let Some(declaration) = self.standard_library.type_by_name(name).copied()
                {
                    let privileged_library_body = matches!(
                        &self.callable,
                        super::context::CallableContext::LibraryFunction(item)
                            if matches!(
                                self.standard_library.item(*item).implementation,
                                Implementation::LibraryBody { .. }
                                    | Implementation::LibraryOverloads { .. }
                            )
                    );
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
                    self.semantics.resolve_record_literal(
                        expr.id,
                        ResolvedRecordId::Standard(declaration.id),
                    );
                    let declared_fields = self
                        .standard_library
                        .fields_of(declaration.id)
                        .copied()
                        .collect::<Vec<_>>();
                    let mut seen = HashSet::new();
                    let mut resolved_fields = Vec::with_capacity(fields.len());
                    for (name, value) in fields {
                        if !seen.insert(name.clone()) {
                            self.error(format!("duplicate record field `{name}`"), value.span);
                            continue;
                        }
                        if let Some(field) =
                            declared_fields.iter().find(|field| field.name == *name)
                        {
                            self.expr(value, Some(self.standard_field_type(field.id)));
                            resolved_fields.push(ResolvedRecordFieldId::Standard(field.id));
                        } else {
                            self.expr(value, None);
                            self.error(
                                format!("record `{}` has no field `{name}`", declaration.name),
                                value.span,
                            );
                        }
                    }
                    self.semantics
                        .resolve_record_literal_fields(expr.id, resolved_fields);
                    for field in &declared_fields {
                        if !seen.contains(field.name) {
                            self.error(
                                format!(
                                    "record `{}` initializer is missing field `{}`",
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
                } else {
                    self.error(format!("unknown record type `{name}`"), expr.span);
                    return None;
                }
            }
            ExprKind::Match { value, arms } => {
                let value_type = self.expr(value, None)?;
                let refines_state_layout = matches!(
                    &value.kind,
                    ExprKind::Path(path)
                        if path.as_slice() == ["layout"] && self.layout_value.is_some()
                );
                let mut unguarded_patterns = HashSet::new();
                let mut has_unguarded_wildcard = false;
                let mut result_type = expected;
                let mut never_arm_type = None;
                for arm in arms {
                    if has_unguarded_wildcard {
                        self.error("unreachable match arm after `_`", arm.span);
                    }
                    self.scopes.push(HashMap::new());
                    let mut state_layout = None;
                    let pattern_key = match &arm.pattern {
                        MatchPattern::Enum {
                            variant, binding, ..
                        } => {
                            let Some(enumeration) = self.resolutions.pattern_enum(arm.pattern_id)
                            else {
                                self.error("unresolved enum type", arm.span);
                                self.scopes.pop();
                                continue;
                            };
                            self.unify(value_type, self.enum_type(enumeration), arm.span);
                            let declaration = self.enum_info(enumeration);
                            if let Some(declaration) = declaration {
                                if let Some(declared_variant) = declaration
                                    .variants
                                    .iter()
                                    .find(|declared| declared.name == *variant)
                                {
                                    self.semantics.resolve_pattern_variant(
                                        arm.pattern_id,
                                        declared_variant.id,
                                    );
                                    if refines_state_layout
                                        && matches!(
                                            declared_variant.id,
                                            ResolvedEnumVariantId::Source(_)
                                        )
                                    {
                                        let ResolvedEnumVariantId::Source(variant) =
                                            declared_variant.id
                                        else {
                                            unreachable!()
                                        };
                                        state_layout = Some(variant);
                                    }
                                    match (declared_variant.payload, binding) {
                                        (Some(payload_type), Some(binding)) => {
                                            self.semantics
                                                .resolve_value_type(binding.id, payload_type);
                                            self.scopes.last_mut().unwrap().insert(
                                                binding.name.clone(),
                                                Binding {
                                                    id: Some(binding.id),
                                                    ty: payload_type,
                                                    mutable: false,
                                                    debug_only: self.debug_context.is_debug(),
                                                    declaration_span: Some(binding.name_span),
                                                },
                                            );
                                        }
                                        (None, Some(_)) => self.error(
                                            format!("variant `{variant}` has no payload to bind"),
                                            arm.span,
                                        ),
                                        _ => {}
                                    }
                                } else {
                                    self.error(
                                        format!(
                                            "enum `{}` has no variant `{variant}`",
                                            declaration.name
                                        ),
                                        arm.span,
                                    );
                                }
                            } else {
                                self.error("unknown enum type", arm.span);
                            }
                            format!("enum:{enumeration}:{variant}")
                        }
                        MatchPattern::Bool(value) => {
                            self.unify(
                                value_type,
                                self.core_type(crate::stdlib::CoreTypeId::Bool),
                                arm.span,
                            );
                            format!("bool:{value}")
                        }
                        MatchPattern::Char(value) => {
                            self.unify(
                                value_type,
                                self.core_type(crate::stdlib::CoreTypeId::Char),
                                arm.span,
                            );
                            format!("char:{value}")
                        }
                        MatchPattern::String(value) => {
                            self.unify(
                                value_type,
                                self.standard_type(StdlibTypeId::String),
                                arm.span,
                            );
                            format!("string:{value:?}")
                        }
                        MatchPattern::Int { value, suffix } => {
                            let pattern_type = if let Some(suffix) = suffix {
                                if !suffix.is_integer() {
                                    self.error(
                                        "integer match patterns require an integer type",
                                        arm.span,
                                    );
                                } else if !self
                                    .inference
                                    .fits_unsigned_literal(*value, self.syntax_type(*suffix))
                                {
                                    self.error(
                                        format!("integer literal does not fit in `{suffix}`"),
                                        arm.span,
                                    );
                                }
                                self.syntax_type(*suffix)
                            } else {
                                self.fresh_inference(
                                    Requirements::capability(StdlibCapabilityId::Integer),
                                    Some(*value),
                                )
                            };
                            self.unify(value_type, pattern_type, arm.span);
                            format!("int:{value}")
                        }
                        MatchPattern::FileVersion(components) => {
                            self.unify(
                                value_type,
                                self.standard_type(StdlibTypeId::FileVersion),
                                arm.span,
                            );
                            format!(
                                "version:{}:{}:{}:{}",
                                components[0], components[1], components[2], components[3]
                            )
                        }
                        MatchPattern::None => {
                            if let Some(option) = self.infer_option_pattern(
                                value_type,
                                arm.span,
                                "a `None` pattern requires an optional value",
                            ) {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::OptionNone(option),
                                );
                                format!("option:{option}:none")
                            } else {
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        }
                        MatchPattern::OptionSome(binding) => {
                            if let Some(option) = self.infer_option_pattern(
                                value_type,
                                arm.span,
                                "a `Some(value)` pattern requires an optional value",
                            ) {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::OptionSome(option),
                                );
                                let binding_type = self.inference.option_value(option);
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(binding, binding_type, arm.span);
                                }
                                format!("option:{option}:some")
                            } else {
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        }
                        MatchPattern::ResultSuccess(binding) => {
                            if let Some(result) = self.infer_result_pattern(
                                value_type,
                                arm.span,
                                "an `Ok(value)` pattern requires a result value",
                            ) {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::ResultSuccess(result),
                                );
                                let binding_type = self.inference.result_value(result);
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(binding, binding_type, arm.span);
                                }
                                format!("result:{result}:success")
                            } else {
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        }
                        MatchPattern::ResultError(binding) => {
                            if let Some(result) = self.infer_result_pattern(
                                value_type,
                                arm.span,
                                "an `Err(error)` pattern requires a result value",
                            ) {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::ResultError(result),
                                );
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(
                                        binding,
                                        self.standard_type(StdlibTypeId::String),
                                        arm.span,
                                    );
                                }
                                format!("result:{result}:error")
                            } else {
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        }
                        MatchPattern::Wildcard => "wildcard".to_owned(),
                    };
                    let state_layout = state_layout.or(self.active_state_layout);
                    let arm_type = self.with_state_layout(state_layout, |checker| {
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
                        if !unguarded_patterns.insert(pattern_key.clone()) {
                            self.error(format!("duplicate match arm `{pattern_key}`"), arm.span);
                        }
                        if matches!(arm.pattern, MatchPattern::Wildcard) {
                            has_unguarded_wildcard = true;
                        }
                    } else if unguarded_patterns.contains(&pattern_key) {
                        self.error("unreachable guarded match arm", arm.span);
                    }
                }

                if !has_unguarded_wildcard {
                    match self.shallow_type(value_type) {
                        ty if ty == self.core_type(crate::stdlib::CoreTypeId::Bool) => {
                            for value in [false, true] {
                                if !unguarded_patterns.contains(&format!("bool:{value}")) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{value}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Option(option) => {
                            for (state, display) in [("none", "None"), ("some", "Some(value)")] {
                                if !unguarded_patterns.contains(&format!("option:{option}:{state}"))
                                {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Result(result) => {
                            for (state, display) in
                                [("success", "Ok(value)"), ("error", "Err(error)")]
                            {
                                if !unguarded_patterns.contains(&format!("result:{result}:{state}"))
                                {
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
                        ty @ Type::Known(_) => {
                            if let Some((enum_key, declaration)) = self.enum_info_for_type(ty) {
                                for variant in &declaration.variants {
                                    let key = format!("enum:{enum_key}:{}", variant.name);
                                    if !unguarded_patterns.contains(&key) {
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
                    let else_type = self.expr(else_expr, Some(result_type));
                    let then_type = self.expr(then_expr, Some(result_type));
                    (then_type, else_type)
                } else {
                    let then_type = self.expr(then_expr, Some(result_type));
                    let else_type = self.expr(else_expr, Some(result_type));
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
                        "`throw` needs a function returning `T!` or an explicit catch boundary",
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
                        "`?` needs a state-field boundary or a function returning `T!`",
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
                    let [_, variant] = path.as_slice() else {
                        unreachable!("resolved enum paths have two segments")
                    };
                    self.enum_constructor(expr.id, enumeration, variant, &[], expected, expr.span)?
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
                if let Some(enumeration) = self.resolutions.expression_enum(expr.id) {
                    debug_assert!(receiver.is_none());
                    if !type_arguments.is_empty() {
                        self.error("enum variants do not accept type arguments", expr.span);
                        return None;
                    }
                    let [_, variant] = callee.as_slice() else {
                        unreachable!("resolved enum constructors have two segments")
                    };
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
        };
        self.semantics.resolve_expression_type(expr.id, ty);
        Some(ty)
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
            self.expr(right, Some(bool_type));
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

fn expression_is_bare_none(expression: &Expr) -> bool {
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
