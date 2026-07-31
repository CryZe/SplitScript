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
        ExpressionKind::Call { arguments, .. } => {
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
    use crate::ast::{ExprId, PatternId};

    use super::{super::LoweredPattern, *};

    fn expression(index: u32) -> ExprId {
        ExprId::from_index(index)
    }

    #[test]
    fn child_edges_follow_deterministic_evaluation_order() {
        let kind = ExpressionKind::Match {
            value: expression(1),
            arms: vec![
                super::super::MatchArm {
                    pattern_id: PatternId::from_index(0),
                    pattern: LoweredPattern::Wildcard,
                    guard: Some(expression(2)),
                    value: expression(3),
                },
                super::super::MatchArm {
                    pattern_id: PatternId::from_index(1),
                    pattern: LoweredPattern::Wildcard,
                    guard: None,
                    value: expression(4),
                },
            ],
        };
        let mut visited = Vec::new();
        visit_expression_children(&kind, |child| visited.push(child));
        assert_eq!(
            visited,
            [expression(1), expression(2), expression(3), expression(4)]
        );
    }
}
