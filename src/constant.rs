//! Syntax-level classification for expressions that can be evaluated without
//! runtime state.

use crate::{
    ast::{Expr, ExprKind},
    resolution::ProgramResolutions,
};

pub(crate) fn is_constant(expression: &Expr, resolutions: &ProgramResolutions) -> bool {
    match &expression.kind {
        ExprKind::None
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::String(_) => true,
        ExprKind::Array(elements) => elements
            .iter()
            .all(|element| is_constant(element, resolutions)),
        ExprKind::Range { start, end, .. } => {
            is_constant(start, resolutions) && is_constant(end, resolutions)
        }
        ExprKind::Record { fields, .. } => fields
            .iter()
            .all(|field| is_constant(&field.value, resolutions)),
        ExprKind::Path(_) => resolutions.expression_enum(expression.id).is_some(),
        ExprKind::Call { args, .. } => {
            args.is_empty() && resolutions.expression_enum(expression.id).is_some()
        }
        ExprKind::Unary { expr, .. } => is_constant(expr, resolutions),
        _ => false,
    }
}
