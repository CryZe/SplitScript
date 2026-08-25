//! Symbolic refinement for attachment-wide layout dimensions.
//!
//! Layout conditions are ordinary boolean expressions in the syntax tree,
//! but declarations deliberately accept only predicates the compiler can
//! prove statically. Keeping the canonical facts here lets state fields,
//! managed metadata, function effects, and code generation share one model.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{BinaryOp, Expr, ExprKind},
    types::ResolvedTypeRef,
};

use super::{Checker, declarations::LayoutConstraint};

impl Checker {
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
        constraints.sort_by_key(|constraint| constraint.dimension.index());
        constraints.dedup();
        Some(constraints)
    }

    /// Requires a declaration predicate to be statically understandable.
    pub(super) fn require_layout_constraints(
        &mut self,
        expression: &Expr,
    ) -> Option<Vec<LayoutConstraint>> {
        let constraints = self.layout_constraints(expression);
        if constraints.as_ref().is_none_or(Vec::is_empty) {
            self.errors.push(
                Diagnostic::type_error(
                    "conditional fields need a statically decidable layout predicate",
                    expression.span,
                )
                .with_primary_label(
                    "compare `layout.<dimension>` with an enum variant using `==`",
                )
                .with_note(
                    "multiple independent dimensions may be combined with `&&`; broader boolean layout predicates will be added on top of this canonical constraint model",
                ),
            );
            return None;
        }
        constraints
    }

    pub(super) fn with_layout_constraints<T>(
        &mut self,
        constraints: Option<&[LayoutConstraint]>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.active_layout_constraints.clone();
        if let Some(constraints) = constraints {
            self.active_layout_constraints.extend(
                constraints
                    .iter()
                    .map(|constraint| (constraint.dimension, constraint.variant)),
            );
        }
        let output = operation(self);
        self.active_layout_constraints = previous;
        output
    }

    pub(super) fn layout_constraints_satisfied(&self, required: &[LayoutConstraint]) -> bool {
        required.iter().all(|constraint| {
            self.active_layout_constraints.get(&constraint.dimension) == Some(&constraint.variant)
        })
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
            _ => None,
        }
    }

    fn layout_constraint_atom(&self, dimension: &Expr, variant: &Expr) -> Option<LayoutConstraint> {
        let dimension_path = expression_path(dimension)?;
        let [root, dimension_name] = dimension_path.as_slice() else {
            return None;
        };
        if *root != "layout" {
            return None;
        }
        let variant_path = expression_path(variant)?;
        let [enum_name, variant_name] = variant_path.as_slice() else {
            return None;
        };

        let layout = self
            .declarations
            .records
            .iter()
            .find(|record| record.name == "Layout")?;
        let field = layout
            .fields
            .iter()
            .find(|field| field.name == *dimension_name)?;
        let ResolvedTypeRef::Enum(enum_id) = self.resolutions.type_ref(field.ty)? else {
            return None;
        };
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
            dimension: field.id,
            variant: variant.id,
        })
    }
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
