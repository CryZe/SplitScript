//! Symbolic refinement for attachment-wide layout dimensions.
//!
//! Layout conditions are ordinary boolean expressions in the syntax tree,
//! but declarations deliberately accept only predicates the compiler can
//! prove statically. Keeping the canonical facts here lets state fields,
//! managed metadata, function effects, and code generation share one model.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{BinaryOp, ConditionalFieldsDecl, Expr, ExprKind, MatchPattern, UnaryOp},
    types::ResolvedTypeRef,
};

use super::{
    Checker,
    declarations::{LayoutConstraint, LayoutDimension, LayoutPredicate},
};

impl Checker {
    pub(super) fn is_attachment_shape_global(&self, value: crate::ast::ValueId) -> bool {
        self.layout_dimensions
            .contains(&LayoutDimension::Global(value))
    }

    /// Extracts a conjunction of enum-variant facts from an arbitrary
    /// condition. Returning `None` merely means that the expression does not
    /// refine layout-dependent declarations.
    pub(super) fn layout_constraints(&self, expression: &Expr) -> Option<Vec<LayoutConstraint>> {
        let mut constraints = Vec::new();
        self.collect_layout_constraints(expression, &mut constraints)?;
        let mut dimensions = HashMap::new();
        for constraint in &constraints {
            if dimensions
                .insert(constraint.dimension, constraint.variant)
                .is_some_and(|previous| previous != constraint.variant)
            {
                return None;
            }
        }
        constraints.sort_by_key(|constraint| layout_dimension_sort_key(constraint.dimension));
        constraints.dedup();
        Some(constraints)
    }

    /// Derives the facts established by the false branch when that complement
    /// is itself one exact layout assignment. At present this is possible for
    /// a single equality over a two-variant enum. Broader predicates would
    /// require a disjunction rather than the canonical conjunction represented
    /// by [`LayoutConstraint`].
    pub(super) fn inverse_layout_constraints(
        &self,
        expression: &Expr,
    ) -> Option<Vec<LayoutConstraint>> {
        let constraints = self.layout_constraints(expression)?;
        let [constraint] = constraints.as_slice() else {
            return None;
        };
        let enum_id = self.layout_dimension_enum(constraint.dimension)?;
        let enumeration = self
            .declarations
            .enums
            .iter()
            .find(|enumeration| enumeration.id == enum_id)?;
        if enumeration.variants.len() != 2 {
            return None;
        }
        let variant = enumeration
            .variants
            .iter()
            .find(|variant| variant.id != constraint.variant)?;
        Some(vec![LayoutConstraint {
            dimension: constraint.dimension,
            variant: variant.id,
        }])
    }

    /// Returns layout facts that are necessarily true whenever `expression`
    /// is true. Unlike declaration predicates, ordinary boolean expressions
    /// may contain unrelated conditions; a conjunction still preserves every
    /// layout fact contributed by either side.
    pub(super) fn truthy_layout_constraints(&self, expression: &Expr) -> Vec<LayoutConstraint> {
        let mut candidates = Vec::new();
        self.collect_truthy_layout_constraints(expression, &mut candidates);
        canonical_constraints(candidates)
    }

    /// Returns layout facts that are necessarily true whenever `expression`
    /// is false. A disjunction must have every operand false, so exact
    /// two-variant complements remain available through an entire `||` chain.
    pub(super) fn falsy_layout_constraints(&self, expression: &Expr) -> Vec<LayoutConstraint> {
        let mut candidates = Vec::new();
        self.collect_falsy_layout_constraints(expression, &mut candidates);
        canonical_constraints(candidates)
    }

    /// Resolves flat parser branches into exact, mutually exclusive layout
    /// alternatives. A branch introduced by `else` starts with the layouts
    /// left unmatched by the preceding branches in the same chain.
    pub(super) fn layout_branch_predicates<Field>(
        &mut self,
        groups: &[ConditionalFieldsDecl<Field>],
    ) -> Vec<LayoutPredicate> {
        for condition in groups.iter().filter_map(|group| group.condition.as_ref()) {
            self.collect_declared_layout_dimensions(condition);
        }
        let universe = self.layout_assignments();
        if universe.is_empty() && !groups.is_empty() {
            self.errors.push(
                Diagnostic::type_error(
                    "conditional fields require a bounded attachment layout",
                    groups[0].keyword_span,
                )
                .with_primary_label("this conditional declaration needs exact layout branches")
                .with_note(format!(
                    "conditional layout declarations support at most {} layout combinations",
                    crate::layout_selection::MAX_ENUMERATED_LAYOUT_COMBINATIONS,
                )),
            );
        }
        let mut remaining = universe.clone();
        let mut predicates = Vec::with_capacity(groups.len());
        for group in groups {
            if group.else_span.is_none() {
                remaining = universe.clone();
            }
            let selected = if let Some(condition) = &group.condition {
                let mut understood = true;
                let selected = remaining
                    .iter()
                    .filter(|assignment| {
                        self.evaluate_layout_condition(condition, assignment)
                            .unwrap_or_else(|| {
                                understood = false;
                                false
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !understood {
                    self.errors.push(
                        Diagnostic::type_error(
                            "conditional fields need a statically decidable layout predicate",
                            condition.span,
                        )
                        .with_primary_label(
                            "test layout dimensions with enum variants using `is`, `==`, or `!=` and combine them with `&&`, `||`, or `!`",
                        ),
                    );
                }
                selected
            } else {
                remaining.clone()
            };
            remaining.retain(|assignment| !selected.contains(assignment));
            predicates.push(LayoutPredicate {
                alternatives: selected,
            });
        }
        predicates
    }

    pub(super) fn with_layout_constraints<T>(
        &mut self,
        constraints: Option<&[LayoutConstraint]>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let predicate = constraints.map(|constraints| LayoutPredicate {
            alternatives: self
                .layout_assignments()
                .into_iter()
                .filter(|assignment| assignment_satisfies_constraints(assignment, constraints))
                .collect(),
        });
        self.with_layout_predicate(predicate.as_ref(), operation)
    }

    pub(super) fn with_layout_predicate<T>(
        &mut self,
        predicate: Option<&LayoutPredicate>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.active_layouts.clone();
        if let Some(predicate) = predicate {
            let active = self.active_layouts.as_ref().map_or_else(
                || self.layout_assignments(),
                |active| active.alternatives.clone(),
            );
            self.active_layouts = Some(LayoutPredicate {
                alternatives: active
                    .into_iter()
                    .filter(|assignment| predicate_matches_assignment(predicate, assignment))
                    .collect(),
            });
        }
        let output = operation(self);
        self.active_layouts = previous;
        output
    }

    pub(super) fn layout_predicate_satisfied(&self, required: &LayoutPredicate) -> bool {
        self.active_layout_assignments()
            .iter()
            .all(|assignment| predicate_matches_assignment(required, assignment))
    }

    fn collect_layout_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<LayoutConstraint>,
    ) -> Option<()> {
        match &expression.kind {
            ExprKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_layout_constraints(left, output)?;
                self.collect_layout_constraints(right, output)
            }
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } => self
                .layout_constraint_atom(left, right)
                .or_else(|| self.layout_constraint_atom(right, left))
                .map(|constraint| output.push(constraint)),
            ExprKind::Is { value, pattern, .. } => self
                .layout_is_constraint_atom(value, &pattern.kind)
                .map(|constraint| output.push(constraint)),
            _ => None,
        }
    }

    fn collect_truthy_layout_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<LayoutConstraint>,
    ) {
        match &expression.kind {
            ExprKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_truthy_layout_constraints(left, output);
                self.collect_truthy_layout_constraints(right, output);
            }
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } => {
                if let Some(constraint) = self
                    .layout_constraint_atom(left, right)
                    .or_else(|| self.layout_constraint_atom(right, left))
                {
                    output.push(constraint);
                }
            }
            ExprKind::Is { value, pattern, .. } => {
                if let Some(constraint) = self.layout_is_constraint_atom(value, &pattern.kind) {
                    output.push(constraint);
                }
            }
            _ => {}
        }
    }

    fn collect_falsy_layout_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<LayoutConstraint>,
    ) {
        if let ExprKind::Binary {
            op: BinaryOp::Or,
            left,
            right,
        } = &expression.kind
        {
            self.collect_falsy_layout_constraints(left, output);
            self.collect_falsy_layout_constraints(right, output);
        } else if let Some(constraints) = self.inverse_layout_constraints(expression) {
            output.extend(constraints);
        }
    }

    fn layout_constraint_atom(&self, dimension: &Expr, variant: &Expr) -> Option<LayoutConstraint> {
        let dimension_path = expression_path(dimension)?;
        let variant_path = expression_path(variant)?;
        let [enum_name, variant_name] = variant_path.as_slice() else {
            return None;
        };
        let (dimension, enum_id) = self.resolve_layout_dimension(&dimension_path)?;
        let enumeration = self
            .declarations
            .enums
            .iter()
            .find(|enumeration| enumeration.id == enum_id && enumeration.name == *enum_name)?;
        let variant = enumeration
            .variants
            .iter()
            .find(|variant| variant.name == *variant_name)?;
        Some(LayoutConstraint {
            dimension,
            variant: variant.id,
        })
    }

    fn layout_is_constraint_atom(
        &self,
        dimension: &Expr,
        pattern: &MatchPattern,
    ) -> Option<LayoutConstraint> {
        let MatchPattern::Enum {
            enumeration,
            variant,
            payload: None,
        } = pattern
        else {
            return None;
        };
        let dimension_path = expression_path(dimension)?;
        let (dimension, enum_id) = self.resolve_layout_dimension(&dimension_path)?;
        let enumeration_decl = self
            .declarations
            .enums
            .iter()
            .find(|candidate| candidate.id == enum_id && candidate.name == enumeration.name)?;
        let variant = enumeration_decl
            .variants
            .iter()
            .find(|candidate| candidate.name == *variant)?;
        Some(LayoutConstraint {
            dimension,
            variant: variant.id,
        })
    }

    fn resolve_layout_dimension(
        &self,
        path: &[&str],
    ) -> Option<(LayoutDimension, crate::ast::EnumId)> {
        match path {
            ["layout", dimension_name] => {
                let layout = self
                    .declarations
                    .structs
                    .iter()
                    .find(|structure| structure.name == "Layout")?;
                let field = layout
                    .fields
                    .iter()
                    .find(|field| field.name == *dimension_name)?;
                let ResolvedTypeRef::Enum(enum_id) = self.resolutions.type_ref(field.ty)? else {
                    return None;
                };
                Some((LayoutDimension::LayoutField(field.id), enum_id))
            }
            [name] => {
                if let Some(binding) = self.declarations.globals.get(*name) {
                    let value = binding.id?;
                    let ResolvedTypeRef::Enum(enum_id) =
                        binding.ty.to_ref(self.inference.type_store())
                    else {
                        return None;
                    };
                    return Some((LayoutDimension::Global(value), enum_id));
                }
                let (value, ty) = self.declarations.state_fields.get(*name).copied()?;
                let ResolvedTypeRef::Enum(enum_id) = ty.to_ref(self.inference.type_store()) else {
                    return None;
                };
                Some((LayoutDimension::StateField(value), enum_id))
            }
            ["current", name] => {
                let (value, ty) = self.declarations.state_fields.get(*name).copied()?;
                let ResolvedTypeRef::Enum(enum_id) = ty.to_ref(self.inference.type_store()) else {
                    return None;
                };
                Some((LayoutDimension::StateField(value), enum_id))
            }
            _ => None,
        }
    }

    fn layout_dimension_enum(&self, dimension: LayoutDimension) -> Option<crate::ast::EnumId> {
        match dimension {
            LayoutDimension::LayoutField(field_id) => {
                let layout = self
                    .declarations
                    .structs
                    .iter()
                    .find(|structure| structure.name == "Layout")?;
                let field = layout.fields.iter().find(|field| field.id == field_id)?;
                let ResolvedTypeRef::Enum(enum_id) = self.resolutions.type_ref(field.ty)? else {
                    return None;
                };
                Some(enum_id)
            }
            LayoutDimension::Global(value) => {
                let ty = self
                    .declarations
                    .globals
                    .values()
                    .find_map(|binding| (binding.id == Some(value)).then_some(binding.ty))?;
                let ResolvedTypeRef::Enum(enum_id) = ty.to_ref(self.inference.type_store()) else {
                    return None;
                };
                Some(enum_id)
            }
            LayoutDimension::StateField(value) => {
                let ty = self.declarations.state_fields_by_id.get(&value).copied()?;
                let ResolvedTypeRef::Enum(enum_id) = ty.to_ref(self.inference.type_store()) else {
                    return None;
                };
                Some(enum_id)
            }
        }
    }

    fn collect_declared_layout_dimensions(&mut self, expression: &Expr) {
        let mut paths = Vec::new();
        collect_expression_paths(expression, &mut paths);
        for path in paths {
            let Some((dimension, _)) = self.resolve_layout_dimension(&path) else {
                continue;
            };
            if !self.layout_dimensions.contains(&dimension) {
                self.layout_dimensions.push(dimension);
            }
        }
    }

    fn layout_assignments(&self) -> Vec<Vec<LayoutConstraint>> {
        let layout = self
            .declarations
            .structs
            .iter()
            .find(|structure| structure.name == "Layout");
        let mut assignments = vec![Vec::new()];
        for field in layout.into_iter().flat_map(|layout| &layout.fields) {
            let Some(ResolvedTypeRef::Enum(enum_id)) = self.resolutions.type_ref(field.ty) else {
                return Vec::new();
            };
            let Some(enumeration) = self
                .declarations
                .enums
                .iter()
                .find(|enumeration| enumeration.id == enum_id)
            else {
                return Vec::new();
            };
            if assignments
                .len()
                .checked_mul(enumeration.variants.len())
                .is_none_or(|count| {
                    count > crate::layout_selection::MAX_ENUMERATED_LAYOUT_COMBINATIONS
                })
            {
                return Vec::new();
            }
            assignments = assignments
                .into_iter()
                .flat_map(|assignment| {
                    enumeration.variants.iter().map(move |variant| {
                        let mut assignment = assignment.clone();
                        assignment.push(LayoutConstraint {
                            dimension: LayoutDimension::LayoutField(field.id),
                            variant: variant.id,
                        });
                        assignment
                    })
                })
                .collect();
        }
        for dimension in &self.layout_dimensions {
            if matches!(dimension, LayoutDimension::LayoutField(_)) {
                continue;
            }
            let Some(enum_id) = self.layout_dimension_enum(*dimension) else {
                return Vec::new();
            };
            let Some(enumeration) = self
                .declarations
                .enums
                .iter()
                .find(|enumeration| enumeration.id == enum_id)
            else {
                return Vec::new();
            };
            if assignments
                .len()
                .checked_mul(enumeration.variants.len())
                .is_none_or(|count| {
                    count > crate::layout_selection::MAX_ENUMERATED_LAYOUT_COMBINATIONS
                })
            {
                return Vec::new();
            }
            assignments = assignments
                .into_iter()
                .flat_map(|assignment| {
                    enumeration.variants.iter().map(move |variant| {
                        let mut assignment = assignment.clone();
                        assignment.push(LayoutConstraint {
                            dimension: *dimension,
                            variant: variant.id,
                        });
                        assignment
                    })
                })
                .collect();
        }
        assignments
    }

    fn active_layout_assignments(&self) -> Vec<Vec<LayoutConstraint>> {
        self.active_layouts.as_ref().map_or_else(
            || self.layout_assignments(),
            |active| active.alternatives.clone(),
        )
    }

    fn evaluate_layout_condition(
        &self,
        expression: &Expr,
        assignment: &[LayoutConstraint],
    ) -> Option<bool> {
        match &expression.kind {
            ExprKind::Bool(value) => Some(*value),
            ExprKind::Unary {
                op: UnaryOp::Not,
                expr,
            } => Some(!self.evaluate_layout_condition(expr, assignment)?),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::And => Some(
                    self.evaluate_layout_condition(left, assignment)?
                        && self.evaluate_layout_condition(right, assignment)?,
                ),
                BinaryOp::Or => Some(
                    self.evaluate_layout_condition(left, assignment)?
                        || self.evaluate_layout_condition(right, assignment)?,
                ),
                BinaryOp::Eq | BinaryOp::Ne => {
                    let constraint = self
                        .layout_constraint_atom(left, right)
                        .or_else(|| self.layout_constraint_atom(right, left))?;
                    let equal = assignment.contains(&constraint);
                    Some(if *op == BinaryOp::Eq { equal } else { !equal })
                }
                _ => None,
            },
            ExprKind::Is { value, pattern, .. } => {
                let constraint = self.layout_is_constraint_atom(value, &pattern.kind)?;
                Some(assignment.contains(&constraint))
            }
            _ => None,
        }
    }
}

fn canonical_constraints(candidates: Vec<LayoutConstraint>) -> Vec<LayoutConstraint> {
    let mut dimensions = HashMap::new();
    for constraint in candidates {
        dimensions
            .entry(constraint.dimension)
            .and_modify(|variant: &mut Option<_>| {
                if *variant != Some(constraint.variant) {
                    *variant = None;
                }
            })
            .or_insert(Some(constraint.variant));
    }
    let mut constraints = dimensions
        .into_iter()
        .filter_map(|(dimension, variant)| {
            variant.map(|variant| LayoutConstraint { dimension, variant })
        })
        .collect::<Vec<_>>();
    constraints.sort_by_key(|constraint| layout_dimension_sort_key(constraint.dimension));
    constraints
}

fn layout_dimension_sort_key(dimension: LayoutDimension) -> (u8, usize) {
    match dimension {
        LayoutDimension::LayoutField(field) => (0, field.index()),
        LayoutDimension::Global(value) => (1, value.index()),
        LayoutDimension::StateField(value) => (2, value.index()),
    }
}

fn predicate_matches_assignment(
    predicate: &LayoutPredicate,
    assignment: &[LayoutConstraint],
) -> bool {
    predicate.alternatives.iter().any(|alternative| {
        alternative
            .iter()
            .all(|constraint| assignment.contains(constraint))
    })
}

fn collect_expression_paths<'a>(expression: &'a Expr, output: &mut Vec<Vec<&'a str>>) {
    match &expression.kind {
        ExprKind::Unary { expr, .. } => collect_expression_paths(expr, output),
        ExprKind::Binary { left, right, .. } => {
            collect_expression_paths(left, output);
            collect_expression_paths(right, output);
        }
        ExprKind::Is { value, .. } => collect_expression_paths(value, output),
        _ => {
            if let Some(path) = expression_path(expression) {
                output.push(path);
            }
        }
    }
}

fn assignment_satisfies_constraints(
    assignment: &[LayoutConstraint],
    constraints: &[LayoutConstraint],
) -> bool {
    constraints
        .iter()
        .all(|constraint| assignment.contains(constraint))
}

fn expression_path(expression: &Expr) -> Option<Vec<&str>> {
    match &expression.kind {
        ExprKind::Path(path) => Some(path.iter().map(String::as_str).collect()),
        ExprKind::Member { receiver, name, .. } => {
            let mut path = expression_path(receiver)?;
            path.push(name);
            Some(path)
        }
        _ => None,
    }
}
