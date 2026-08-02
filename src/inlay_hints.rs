//! Compiler-owned inferred-type hints shared by LSP clients.

use crate::{
    ast::{ForBinding, FunctionDecl, Span, StateField, SuspensionBinding, VariableDecl},
    database::SemanticSnapshot,
    lexer::{Token, TokenKind},
    type_display::display_type,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHint {
    /// Byte position immediately after the declaration's identifier.
    pub position: usize,
    /// Source-shaped inferred type, including the declaration separator.
    pub label: String,
}

pub(crate) fn inferred_type_hints(
    snapshot: &SemanticSnapshot,
    requested_range: Span,
) -> Vec<InlayHint> {
    let tokens = snapshot.source_document().tokens().collect::<Vec<_>>();
    let mut collector = InlayHintCollector {
        snapshot,
        requested_range,
        tokens,
        hints: Vec::new(),
    };
    collector.visit_program(snapshot.syntax());
    collector.hints.sort_by_key(|hint| hint.position);
    collector.hints
}

struct InlayHintCollector<'a> {
    snapshot: &'a SemanticSnapshot,
    requested_range: Span,
    tokens: Vec<&'a Token>,
    hints: Vec<InlayHint>,
}

impl InlayHintCollector<'_> {
    fn add_hint(&mut self, position: usize, label: String) {
        if position < self.requested_range.start || position > self.requested_range.end {
            return;
        }
        self.hints.push(InlayHint { position, label });
    }

    fn add_inferred_value(&mut self, id: crate::ast::ValueId, name: &str, span: Span) {
        let Some(ty) = self.snapshot.semantics().value_type(id) else {
            return;
        };
        let first_candidate = self
            .tokens
            .partition_point(|token| token.span.start < span.start);
        let Some(identifier) = self.tokens[first_candidate..]
            .iter()
            .take_while(|token| token.span.end <= span.end)
            .find(|token| matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name))
        else {
            return;
        };
        self.add_hint(
            identifier.span.end,
            format!(": {}", display_type(ty, self.snapshot)),
        );
    }

    fn add_inferred_function_result(&mut self, function: &FunctionDecl) {
        let Some(ty) = self.snapshot.semantics().function_result(function.id) else {
            return;
        };
        let first_candidate = self
            .tokens
            .partition_point(|token| token.span.start < function.span.start);
        let Some(closing_parenthesis) = self.tokens[first_candidate..]
            .iter()
            .take_while(|token| token.span.end <= function.body.span.start)
            .filter(|token| token.kind == TokenKind::RParen)
            .last()
        else {
            return;
        };
        self.add_hint(
            closing_parenthesis.span.end,
            format!(" -> {}", display_type(ty, self.snapshot)),
        );
    }
}

impl<'ast> Visitor<'ast> for InlayHintCollector<'_> {
    fn visit_state_field(&mut self, field: &'ast StateField) {
        if field.annotation.is_none() {
            self.add_inferred_value(field.id, &field.name, field.span);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        for parameter in &function.params {
            if parameter.annotation.is_none() {
                self.add_inferred_value(parameter.id, &parameter.name, parameter.span);
            }
        }
        if function.return_annotation.is_none() {
            self.add_inferred_function_result(function);
        }
        visit::walk_function(self, function);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        if variable.annotation.is_none() {
            self.add_inferred_value(variable.id, &variable.name, variable.span);
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast SuspensionBinding) {
        if binding.annotation.is_none() {
            self.add_inferred_value(binding.id, &binding.name, binding.span);
        }
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast ForBinding) {
        self.add_inferred_value(binding.id, &binding.name, binding.span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CompilerDatabase;

    #[test]
    fn reports_only_inferred_declaration_types_in_the_requested_range() {
        let source = r#"state "game.exe" {}
let global = 7
let explicit: i32 = 8
fn identity(value) {
    return value
}
fn annotatedReturn() -> i32 {
    return 1
}
whileAttached {
    let local = identity(global)
    let annotated: i32 = local
    for item in [1, 2] {
        print(item as String)
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let range = Span {
            start: source.find("fn identity").unwrap(),
            end: source.len(),
        };
        let hints = inferred_type_hints(&snapshot, range);
        assert_eq!(
            hints
                .iter()
                .map(|hint| (&source[..hint.position], hint.label.as_str()))
                .map(|(before, label)| (before.split_whitespace().last().unwrap(), label))
                .collect::<Vec<_>>(),
            [
                ("identity(value", ": T"),
                ("identity(value)", " -> T"),
                ("local", ": i32"),
                ("item", ": i32")
            ]
        );
    }
}
