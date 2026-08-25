//! Syntax-level control-flow facts used to select and validate body types.

use crate::{
    ast::{Block, Expr, ExprKind, Stmt},
    visit::{self, Visitor},
};

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

pub(super) fn expression_contains_suspension(expression: &Expr) -> bool {
    struct SuspensionFinder(bool);

    impl<'ast> Visitor<'ast> for SuspensionFinder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression.kind, ExprKind::Suspend { .. }) {
                self.0 = true;
            } else if !self.0 && !matches!(expression.kind, ExprKind::Closure { .. }) {
                visit::walk_expr(self, expression);
            }
        }

        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if matches!(statement, Stmt::Suspend { .. }) {
                self.0 = true;
            } else if !self.0 {
                visit::walk_stmt(self, statement);
            }
        }
    }

    let mut finder = SuspensionFinder(false);
    finder.visit_expr(expression);
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
