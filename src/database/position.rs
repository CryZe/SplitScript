//! Cursor-position analysis over strict or recovered semantic products.

use std::cmp::Reverse;

use crate::{
    CheckedProgram,
    ast::{ExprId, ExprKind, Span},
    hir::{ExpressionResolution, TypedExpression, TypedExpressionKind},
    lexer::TokenKind,
    semantic::SemanticModel,
    stdlib::StandardLibrary,
    syntax::SourceDocument,
    types::{EnumTypeId, TypeId, TypeKind},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionAnalysis {
    pub expression: ExprId,
    pub span: Span,
    /// Exact identifier tokens forming a path or call target. This excludes
    /// call arguments and other identifiers in child expressions.
    pub segments: Vec<IdentifierSegment>,
    pub ty: TypeId,
    pub type_kind: TypeKind,
    pub resolution: Option<ExpressionResolution>,
}

impl PositionAnalysis {
    pub fn segment_at(&self, offset: usize) -> Option<&IdentifierSegment> {
        self.segments
            .iter()
            .find(|segment| segment.span.start <= offset && offset < segment.span.end)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierSegment {
    pub name: String,
    pub span: Span,
}

pub(super) fn expression_segments(
    checked: &CheckedProgram,
    expression: &TypedExpression,
) -> Vec<IdentifierSegment> {
    let names = match &expression.kind {
        TypedExpressionKind::Path(names) => names.clone(),
        TypedExpressionKind::Member { name, .. } => vec![name.clone()],
        TypedExpressionKind::Call { source_path, .. } => source_path.clone(),
        TypedExpressionKind::Enum {
            enumeration,
            variant,
            ..
        } => enum_type_name(
            *enumeration,
            checked.enum_types(),
            &checked.context().standard_library(),
        )
        .map(|enumeration| vec![enumeration, variant.clone()])
        .unwrap_or_default(),
        _ => return Vec::new(),
    };
    let mut tokens = checked.source_document().tokens().filter(|token| {
        expression.span.start <= token.span.start && token.span.end <= expression.span.end
    });
    names
        .iter()
        .filter_map(|name| {
            tokens.find_map(|token| match &token.kind {
                TokenKind::Ident(spelling) if spelling == name => Some(IdentifierSegment {
                    name: name.clone(),
                    span: token.span,
                }),
                _ => None,
            })
        })
        .collect()
}

pub(super) fn syntax_expression_segments(
    document: &SourceDocument,
    expression: &crate::ast::Expr,
) -> Vec<IdentifierSegment> {
    let names = match &expression.kind {
        ExprKind::Path(names) => names.clone(),
        ExprKind::Member { name, .. } => vec![name.clone()],
        ExprKind::Call { callee, .. } => callee.clone(),
        _ => return Vec::new(),
    };
    let mut tokens = document.tokens().filter(|token| {
        expression.span.start <= token.span.start && token.span.end <= expression.span.end
    });
    names
        .iter()
        .filter_map(|name| {
            tokens.find_map(|token| match &token.kind {
                TokenKind::Ident(spelling) if spelling == name => Some(IdentifierSegment {
                    name: name.clone(),
                    span: token.span,
                }),
                _ => None,
            })
        })
        .collect()
}

fn enum_type_name(
    enumeration: EnumTypeId,
    enum_types: &[crate::ast::EnumDecl],
    standard_library: &StandardLibrary,
) -> Option<String> {
    match enumeration {
        EnumTypeId::Source(id) => enum_types
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|declaration| declaration.name.clone()),
        EnumTypeId::Standard(id) => Some(standard_library.type_decl(id).name.to_owned()),
    }
}

pub(super) fn syntax_expression_resolution(
    semantics: &SemanticModel,
    expression: &crate::ast::Expr,
) -> Option<ExpressionResolution> {
    if let Some(variant) = semantics.enum_variant(expression.id) {
        return Some(ExpressionResolution::EnumConstructor { variant });
    }
    if let Some(call) = semantics.call(expression.id) {
        return Some(ExpressionResolution::Call(call.clone()));
    }
    match &expression.kind {
        ExprKind::Path(_) => Some(ExpressionResolution::ValuePath {
            root: semantics.value(expression.id),
            members: semantics
                .path_members(expression.id)
                .unwrap_or_default()
                .to_vec(),
        }),
        ExprKind::Member { .. } => Some(ExpressionResolution::Member {
            members: semantics
                .path_members(expression.id)
                .unwrap_or_default()
                .to_vec(),
        }),
        ExprKind::Call { .. } => None,
        ExprKind::Record { .. } => semantics
            .record_literal_fields(expression.id)
            .map(|fields| ExpressionResolution::RecordLiteral {
                record: semantics
                    .record_literal(expression.id)
                    .expect("resolved record fields have a nominal record"),
                fields: fields.to_vec(),
            }),
        _ => None,
    }
}

struct ExpressionCollector<'ast> {
    expressions: Vec<&'ast crate::ast::Expr>,
}

impl<'ast> Visitor<'ast> for ExpressionCollector<'ast> {
    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        self.expressions.push(expression);
        visit::walk_expr(self, expression);
    }
}

fn syntax_expressions(program: &crate::ast::Program) -> Vec<&crate::ast::Expr> {
    let mut collector = ExpressionCollector {
        expressions: Vec::new(),
    };
    collector.visit_program(program);
    collector.expressions
}

pub(super) fn syntax_expression_at<'a>(
    program: &'a crate::ast::Program,
    semantics: &SemanticModel,
    offset: usize,
) -> Option<&'a crate::ast::Expr> {
    syntax_expressions(program)
        .into_iter()
        .filter(|expression| {
            expression.span.start <= offset
                && offset < expression.span.end
                && expression.span.start != expression.span.end
                && semantics.expression_type(expression.id).is_some()
        })
        .min_by_key(|expression| {
            (
                expression.span.end - expression.span.start,
                Reverse(expression.id.index()),
            )
        })
}

pub(super) fn syntax_expression_by_id(
    program: &crate::ast::Program,
    id: ExprId,
) -> Option<&crate::ast::Expr> {
    syntax_expressions(program)
        .into_iter()
        .find(|expression| expression.id == id)
}
