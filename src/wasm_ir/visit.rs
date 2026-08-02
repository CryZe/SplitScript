//! Shared traversal for lowered Wasm control flow and expression DAGs.

use crate::ast::ExprId;

use super::{
    Block, Expression, ExpressionKind, FallbackBranch, InterpolatedPart, Program, Statement,
    Terminator,
};

pub trait Visitor {
    fn visit_program(&mut self, program: &Program) {
        walk_program(self, program);
    }

    fn visit_block(&mut self, block: &Block, program: &Program) {
        walk_block(self, block, program);
    }

    fn visit_statement(&mut self, statement: &Statement, program: &Program) {
        walk_statement(self, statement, program);
    }

    fn visit_terminator(&mut self, terminator: &Terminator, program: &Program) {
        walk_terminator(self, terminator, program);
    }

    fn visit_expression_id(&mut self, expression: ExprId, program: &Program) {
        let expression = program
            .expression(expression)
            .expect("lowered expression references belong to the Wasm IR program");
        self.visit_expression(expression, program);
    }

    fn visit_expression(&mut self, expression: &Expression, program: &Program) {
        walk_expression(self, expression, program);
    }
}

pub fn walk_program(visitor: &mut (impl Visitor + ?Sized), program: &Program) {
    for body in program.bodies() {
        visitor.visit_block(&body.entry, program);
    }
    for (_, expression) in program.global_initializers() {
        visitor.visit_expression_id(expression, program);
    }
    for state in program.state_expressions() {
        visitor.visit_expression_id(state.expression, program);
    }
}

pub fn walk_block(visitor: &mut (impl Visitor + ?Sized), block: &Block, program: &Program) {
    for statement in &block.statements {
        visitor.visit_statement(statement, program);
    }
    visitor.visit_terminator(&block.terminator, program);
}

pub fn walk_statement(
    visitor: &mut (impl Visitor + ?Sized),
    statement: &Statement,
    program: &Program,
) {
    match statement {
        Statement::Store { value, .. }
        | Statement::Evaluate {
            expression: value, ..
        } => visitor.visit_expression_id(*value, program),
        Statement::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(then_block, program);
            visitor.visit_block(else_block, program);
        }
        Statement::While { condition, body } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(body, program);
        }
        Statement::For { iterable, body, .. } => {
            visitor.visit_expression_id(*iterable, program);
            visitor.visit_block(body, program);
        }
        Statement::ForInit { iterable, .. } => visitor.visit_expression_id(*iterable, program),
    }
}

pub fn walk_terminator(
    visitor: &mut (impl Visitor + ?Sized),
    terminator: &Terminator,
    program: &Program,
) {
    match terminator {
        Terminator::Fallthrough | Terminator::Break | Terminator::Continue => {}
        Terminator::AsyncWhile {
            condition,
            body,
            continuation,
            ..
        } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(body, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::AsyncFor {
            body, continuation, ..
        } => {
            visitor.visit_block(body, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::Return(value) => {
            if let Some(value) = value {
                visitor.visit_expression_id(*value, program);
            }
        }
        Terminator::Throw { error, .. } => visitor.visit_expression_id(*error, program),
        Terminator::Suspend {
            value,
            continuation,
            ..
        } => {
            visitor.visit_expression_id(*value, program);
            visitor.visit_block(continuation, program);
        }
    }
}

pub fn walk_expression(
    visitor: &mut (impl Visitor + ?Sized),
    expression: &Expression,
    program: &Program,
) {
    visit_expression_children(&expression.kind, |child| {
        visitor.visit_expression_id(child, program);
    });
}

/// Visits direct expression edges in deterministic evaluation order.
///
/// Analyses that own their own worklist can use this without inheriting the
/// recursive behavior of [`Visitor`]. This is the one exhaustive child-shape
/// match for the Wasm expression IR.
pub fn visit_expression_children(kind: &ExpressionKind, mut visit: impl FnMut(ExprId)) {
    match kind {
        ExpressionKind::None
        | ExpressionKind::Bool(_)
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::String(_)
        | ExpressionKind::Signature(_)
        | ExpressionKind::Path { .. } => {}
        ExpressionKind::Member { receiver, .. } => visit(*receiver),
        ExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expression { expression, .. } = part {
                    visit(*expression);
                }
            }
        }
        ExpressionKind::Array(elements) => elements.iter().copied().for_each(&mut visit),
        ExpressionKind::Record { fields, .. } => {
            fields.iter().map(|(_, value)| *value).for_each(&mut visit);
        }
        ExpressionKind::Enum { payload, .. } => payload.iter().copied().for_each(&mut visit),
        ExpressionKind::Unary { operand, .. } => visit(*operand),
        ExpressionKind::Cast { value } | ExpressionKind::Propagate { value, .. } => visit(*value),
        ExpressionKind::Binary { left, right, .. } => {
            visit(*left);
            visit(*right);
        }
        ExpressionKind::Call { target, arguments } => {
            let receiver = match target {
                super::CallTarget::UserMethod {
                    receiver:
                        crate::semantic::ResolvedReceiver::Expression {
                            expression: receiver,
                            ..
                        },
                    ..
                }
                | super::CallTarget::Intrinsic {
                    receiver:
                        Some(crate::semantic::ResolvedReceiver::Expression {
                            expression: receiver,
                            ..
                        }),
                    ..
                } => Some(*receiver),
                _ => None,
            };
            if let Some(receiver) = receiver {
                visit(receiver);
            }
            arguments.iter().copied().for_each(&mut visit);
        }
        ExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visit(*condition);
            visit(*then_expr);
            visit(*else_expr);
        }
        ExpressionKind::Fallback { value, fallback } => {
            visit(*value);
            match fallback {
                FallbackBranch::Value(value) => visit(*value),
                FallbackBranch::Return(value) => value.iter().copied().for_each(&mut visit),
                FallbackBranch::Break | FallbackBranch::Continue => {}
            }
        }
        ExpressionKind::Match { value, arms } => {
            visit(*value);
            for arm in arms {
                arm.guard.iter().copied().for_each(&mut visit);
                visit(arm.value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{ExprKind, Stmt};

    use super::*;

    #[test]
    fn child_edges_follow_deterministic_evaluation_order() {
        let parsed = crate::parse(
            r#"
                state "game.exe" {}

                fn choose(value: i32) {
                    let selected = match value {
                        1 if true => 2,
                        _ => 3
                    }
                }
            "#,
        )
        .unwrap();
        let Stmt::Variable(variable) = &parsed.syntax().functions[0].body.statements[0] else {
            panic!("expected a variable declaration")
        };
        let ExprKind::Match { value, arms } = &variable.value.kind else {
            panic!("expected a match expression")
        };
        let expected = [
            value.id,
            arms[0].guard.as_ref().unwrap().id,
            arms[0].value.id,
            arms[1].value.id,
        ];
        let match_id = variable.value.id;
        let lowered = crate::lower(parsed);
        let checked = crate::check(lowered).unwrap();
        let program = crate::lower_wasm(&checked);
        let kind = program
            .expression(match_id)
            .expect("the match expression is lowered")
            .kind
            .clone();
        let mut visited = Vec::new();
        visit_expression_children(&kind, |child| visited.push(child));
        assert_eq!(visited, expected);
    }
}
