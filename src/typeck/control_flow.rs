//! Syntax-level control-flow facts used to select and validate body types.

use crate::{
    ast::{Block, Expr, ExprKind, Stmt},
    resolution::ProgramResolutions,
    visit::{self, Visitor},
};

pub(super) fn is_constant(expression: &Expr, resolutions: &ProgramResolutions) -> bool {
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
            .all(|(_, value)| is_constant(value, resolutions)),
        ExprKind::Path(_) => resolutions.expression_enum(expression.id).is_some(),
        ExprKind::Call { args, .. } => {
            args.is_empty() && resolutions.expression_enum(expression.id).is_some()
        }
        ExprKind::Unary { expr, .. } => is_constant(expr, resolutions),
        _ => false,
    }
}

pub(super) fn contains_value_return(block: &Block) -> bool {
    let mut finder = ValueReturnFinder(false);
    finder.visit_block(block);
    finder.0
}

pub(super) fn contains_suspension(block: &Block) -> bool {
    struct SuspensionFinder(bool);

    impl<'ast> Visitor<'ast> for SuspensionFinder {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if matches!(statement, Stmt::Suspend { .. }) {
                self.0 = true;
            } else if !self.0 {
                visit::walk_stmt(self, statement);
            }
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression.kind, ExprKind::Suspend { .. }) {
                self.0 = true;
            } else if !self.0 {
                visit::walk_expr(self, expression);
            }
        }
    }

    let mut finder = SuspensionFinder(false);
    finder.visit_block(block);
    finder.0
}

pub(super) fn contains_propagation(expression: &Expr) -> bool {
    #[derive(Default)]
    struct PropagationFinder(bool);

    impl<'ast> Visitor<'ast> for PropagationFinder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression.kind, ExprKind::Propagate(_)) {
                self.0 = true;
            } else if !self.0 {
                visit::walk_expr(self, expression);
            }
        }
    }

    let mut finder = PropagationFinder::default();
    finder.visit_expr(expression);
    finder.0
}

struct ValueReturnFinder(bool);

impl<'ast> Visitor<'ast> for ValueReturnFinder {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if matches!(statement, Stmt::Suspend { returns: true, .. }) {
            self.0 = true;
        } else if !self.0 {
            visit::walk_stmt(self, statement);
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression.kind, ExprKind::Return(Some(_))) {
            self.0 = true;
        } else if !self.0 {
            visit::walk_expr(self, expression);
        }
    }
}
