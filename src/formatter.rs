//! Canonical source formatting built on the compiler's lossless lexer and
//! strict parser.

use std::collections::HashSet;

use crate::{
    Diagnostic,
    ast::{
        Action, EnumDecl, Expr, ExprKind, FunctionDecl, Program, RecordDecl, SettingDecl,
        SettingKind, StateDecl, StateField, Stmt, VariableDecl,
    },
    lexer::{Lexeme, TokenKind, TriviaKind},
    syntax::SourceDocument,
    visit::{self, Visitor},
};

const INDENT: &str = "    ";

/// Formats a syntactically valid SplitScript source file.
///
/// Formatting first runs the ordinary compiler parser, so it never needs a
/// second, potentially divergent grammar. Invalid source is returned as the
/// same structured syntax diagnostics produced by [`crate::parse`].
pub fn format_source(source: &str) -> Result<String, Vec<Diagnostic>> {
    let parsed = crate::parse(source)?;
    Ok(format_parsed(&parsed))
}

pub(crate) fn format_parsed(parsed: &crate::ParsedProgram) -> String {
    Formatter::new(parsed.source_document(), parsed.syntax()).finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Separation {
    None,
    Space,
    Newline,
    BlankLine,
}

#[derive(Debug, Clone, Copy)]
struct HeaderRange {
    start: usize,
    opening_brace: usize,
}

#[derive(Debug, Clone, Copy)]
struct ContinuationRange {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct DelimiterRange {
    opening: usize,
    closing: usize,
    direct_breaks: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
struct BraceFrame {
    parent_indentation: usize,
    brace_indentation: usize,
    opening: usize,
}

#[derive(Default)]
struct HeaderCollector {
    ranges: Vec<HeaderRange>,
}

impl HeaderCollector {
    fn push(&mut self, start: usize, opening_brace: usize) {
        self.ranges.push(HeaderRange {
            start,
            opening_brace,
        });
    }
}

impl<'ast> Visitor<'ast> for HeaderCollector {
    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        self.push(function.span.start, function.body.span.start);
        visit::walk_function(self, function);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.push(action.span.start, action.body.span.start);
        self.visit_block(&action.body);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::If {
                then_block,
                else_block,
                span,
                ..
            } => {
                self.push(span.start, then_block.span.start);
                if let Some(else_block) = else_block {
                    self.push(then_block.span.end, else_block.span.start);
                }
            }
            Stmt::While { body, span, .. } | Stmt::For { body, span, .. } => {
                self.push(span.start, body.span.start)
            }
            _ => {}
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::If {
            then_expr,
            else_expr,
            ..
        } = &expression.kind
        {
            self.push(expression.span.start, then_expr.span.start);
            self.push(then_expr.span.end, else_expr.span.start);
        }
        visit::walk_expr(self, expression);
    }
}

struct SyntaxLayoutCollector<'a> {
    document: &'a SourceDocument,
    continuations: Vec<ContinuationRange>,
    join_before: HashSet<usize>,
    break_after: HashSet<usize>,
    generic_opens: HashSet<usize>,
    generic_closes: HashSet<usize>,
    index_opens: HashSet<usize>,
}

impl SyntaxLayoutCollector<'_> {
    fn continuation(&mut self, start: usize, end: usize) {
        if line_breaks(&self.document.source()[start..end]) > 0 {
            self.continuations.push(ContinuationRange { start, end });
        }
    }

    fn continuation_before_block(&mut self, start: usize, end: usize) {
        let end = self
            .document
            .tokens()
            .find(|token| {
                start <= token.span.start
                    && token.span.end <= end
                    && token.kind == TokenKind::LBrace
            })
            .map_or(end, |token| token.span.start);
        self.continuation(start, end);
    }

    fn mark_separator_after(&mut self, offset: usize) {
        if let Some(token) = self
            .document
            .tokens()
            .find(|token| token.span.start >= offset)
            && matches!(token.kind, TokenKind::Comma | TokenKind::Semicolon)
        {
            self.break_after.insert(token.span.end);
        }
    }

    fn block_after_keyword(&self, setting: &SettingDecl, keyword: &str) -> Option<(usize, usize)> {
        let mut saw_keyword = false;
        let mut depth = 0usize;
        let mut opening = None;
        for token in self.document.tokens().filter(|token| {
            setting.span.start <= token.span.start && token.span.end <= setting.span.end
        }) {
            if !saw_keyword {
                saw_keyword = matches!(&token.kind, TokenKind::Ident(name) if name == keyword);
                continue;
            }
            match token.kind {
                TokenKind::LBrace => {
                    if opening.is_none() {
                        opening = Some(token.span.start);
                    }
                    depth += 1;
                }
                TokenKind::RBrace if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return opening.map(|opening| (opening, token.span.start));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn mark_top_level_commas(&mut self, opening: usize, closing: usize) {
        let mut depth = 0usize;
        for token in self
            .document
            .tokens()
            .filter(|token| opening < token.span.start && token.span.start < closing)
        {
            match token.kind {
                TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::TemplateExprStart => depth += 1,
                TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::TemplateExprEnd => depth = depth.saturating_sub(1),
                TokenKind::Comma if depth == 0 => {
                    self.break_after.insert(token.span.end);
                }
                _ => {}
            }
        }
    }

    fn mark_declaration_items_vertical(
        &mut self,
        span: crate::ast::Span,
        item_ends: impl IntoIterator<Item = usize>,
    ) {
        if let Some(opening) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && token.kind == TokenKind::LBrace
        }) {
            self.break_after.insert(opening.span.end);
        }
        for end in item_ends {
            self.break_after.insert(end);
            self.mark_separator_after(end);
        }
    }
}

impl<'ast> Visitor<'ast> for SyntaxLayoutCollector<'_> {
    fn visit_record(&mut self, record: &'ast RecordDecl) {
        self.mark_declaration_items_vertical(
            record.span,
            record.fields.iter().map(|field| field.span.end),
        );
        visit::walk_record(self, record);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        self.mark_declaration_items_vertical(
            enumeration.span,
            enumeration.variants.iter().map(|variant| variant.span.end),
        );
        visit::walk_enum(self, enumeration);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        for layout in &state.layouts {
            self.mark_separator_after(layout.span.end);
            for field in &layout.fields {
                self.visit_state_field(field);
            }
        }
        for field in &state.fields {
            self.visit_state_field(field);
        }
        self.break_after.insert(state.span.end);
    }

    fn visit_state_field(&mut self, field: &'ast StateField) {
        self.continuation_before_block(field.span.start, field.span.end);
        visit::walk_state_field(self, field);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        self.continuation_before_block(variable.span.start, variable.span.end);
        visit::walk_variable(self, variable);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::Assign { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Throw { span, .. }
            | Stmt::Suspend { span, .. } => {
                self.continuation_before_block(span.start, span.end);
            }
            Stmt::Expression(expression) => {
                self.continuation_before_block(expression.span.start, expression.span.end);
            }
            Stmt::If { span, .. } | Stmt::While { span, .. } | Stmt::For { span, .. } => {
                self.break_after.insert(span.end);
            }
            Stmt::Debug { .. } | Stmt::Variable(_) | Stmt::Break { .. } | Stmt::Continue { .. } => {
            }
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        if !matches!(setting.kind, SettingKind::Title { .. }) {
            self.join_before.extend(
                self.document
                    .tokens()
                    .filter(|token| {
                        setting.span.start <= token.span.start
                            && token.span.end <= setting.span.end
                            && token.kind == TokenKind::FatArrow
                    })
                    .map(|token| token.span.start),
            );
        }
        match &setting.kind {
            SettingKind::Bool { .. } => {
                self.continuation(setting.span.start, setting.span.end);
            }
            SettingKind::Choice { options, .. } => {
                if let Some((opening, closing)) = self.block_after_keyword(setting, "choice") {
                    self.continuation(setting.span.start, opening);
                    self.mark_top_level_commas(opening, closing);
                }
                for option in options {
                    self.continuation(option.span.start, option.span.end);
                }
            }
            SettingKind::File { .. } => {
                if let Some((opening, closing)) = self.block_after_keyword(setting, "file") {
                    self.continuation(setting.span.start, opening);
                    self.mark_top_level_commas(opening, closing);
                }
            }
            SettingKind::Title { .. } => {}
        }
        self.mark_separator_after(setting.span.end);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Match { value, .. } = &expression.kind
            && let Some(opening) = self.document.tokens().find(|token| {
                value.span.end <= token.span.start
                    && token.span.end <= expression.span.end
                    && token.kind == TokenKind::LBrace
            })
        {
            self.mark_top_level_commas(opening.span.start, expression.span.end);
        }
        if let ExprKind::Call {
            name_span,
            type_arguments,
            ..
        } = &expression.kind
            && !type_arguments.is_empty()
        {
            let mut angles = self.document.tokens().filter(|token| {
                name_span.end <= token.span.start
                    && token.span.end <= expression.span.end
                    && matches!(token.kind, TokenKind::Lt | TokenKind::Gt)
            });
            if let Some(open) = angles.next() {
                self.generic_opens.insert(open.span.start);
            }
            if let Some(close) = angles.next() {
                self.generic_closes.insert(close.span.start);
            }
        }
        if let ExprKind::Index { bracket_span, .. } = &expression.kind {
            self.index_opens.insert(bracket_span.start);
        }
        visit::walk_expr(self, expression);
    }
}

struct Formatter<'a> {
    document: &'a SourceDocument,
    output: String,
    indentation: usize,
    line_start: bool,
    previous: Option<&'a Lexeme>,
    previous_was_prefix: bool,
    previous_was_generic_close: bool,
    gap_line_breaks: usize,
    generic_depth: usize,
    generic_opens: HashSet<usize>,
    generic_closes: HashSet<usize>,
    index_opens: HashSet<usize>,
    bracket_depth: usize,
    multiline_headers: Vec<HeaderRange>,
    continuations: Vec<ContinuationRange>,
    delimiters: Vec<DelimiterRange>,
    break_after: HashSet<usize>,
    join_before: HashSet<usize>,
    trailing_comma_before: HashSet<usize>,
    trailing_semicolon_before: HashSet<usize>,
    omitted_commas: HashSet<usize>,
    brace_stack: Vec<BraceFrame>,
    current_line_indentation: usize,
}

impl<'a> Formatter<'a> {
    fn new(document: &'a SourceDocument, syntax: &Program) -> Self {
        let mut headers = HeaderCollector::default();
        headers.visit_program(syntax);
        let multiline_headers = headers
            .ranges
            .into_iter()
            .filter(|range| line_breaks(&document.source()[range.start..range.opening_brace]) > 0)
            .collect();
        let mut layout = SyntaxLayoutCollector {
            document,
            continuations: Vec::new(),
            join_before: HashSet::new(),
            break_after: HashSet::new(),
            generic_opens: HashSet::new(),
            generic_closes: HashSet::new(),
            index_opens: HashSet::new(),
        };
        layout.visit_program(syntax);
        let trailing_punctuation = trailing_list_punctuation(document, syntax, &layout.break_after);
        Self {
            document,
            output: String::with_capacity(document.source().len()),
            indentation: 0,
            line_start: true,
            previous: None,
            previous_was_prefix: false,
            previous_was_generic_close: false,
            gap_line_breaks: 0,
            generic_depth: 0,
            generic_opens: layout.generic_opens,
            generic_closes: layout.generic_closes,
            index_opens: layout.index_opens,
            bracket_depth: 0,
            multiline_headers,
            continuations: layout.continuations,
            delimiters: delimiter_ranges(document),
            join_before: layout.join_before,
            break_after: layout.break_after,
            trailing_comma_before: trailing_punctuation.commas,
            trailing_semicolon_before: trailing_punctuation.semicolons,
            omitted_commas: trailing_punctuation.omitted_commas,
            brace_stack: Vec::new(),
            current_line_indentation: 0,
        }
    }

    fn finish(mut self) -> String {
        for lexeme in self.document.lexemes() {
            match lexeme {
                Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::Whitespace => {
                    let text = self.document.text(trivia.span);
                    self.gap_line_breaks += line_breaks(text);
                }
                Lexeme::Token(token) if token.kind == TokenKind::Eof => {}
                _ => self.write_lexeme(lexeme),
            }
        }

        while self.output.ends_with(char::is_whitespace) {
            self.output.pop();
        }
        self.output.push('\n');
        self.output
    }

    fn write_lexeme(&mut self, current: &'a Lexeme) {
        let current_token = token_kind(current);
        if matches!(current_token, Some(TokenKind::Comma))
            && self.omitted_commas.contains(&current.span().start)
        {
            return;
        }
        if matches!(current_token, Some(TokenKind::RBrace)) {
            self.indentation = self.brace_stack.last().map_or_else(
                || self.indentation.saturating_sub(1),
                |frame| frame.brace_indentation,
            );
        }

        if self.trailing_comma_before.contains(&current.span().start)
            && !matches!(self.previous.and_then(token_kind), Some(TokenKind::Comma))
            && self.previous.is_some_and(|previous| !is_comment(previous))
        {
            self.output.push(',');
        }
        if self
            .trailing_semicolon_before
            .contains(&current.span().start)
            && !matches!(
                self.previous.and_then(token_kind),
                Some(TokenKind::Semicolon)
            )
            && self.previous.is_some_and(|previous| !is_comment(previous))
        {
            self.output.push(';');
        }

        let separation = self.separation(current);
        self.write_separation(separation);
        self.write_indentation(current);
        self.output.push_str(self.document.text(current.span()));
        self.line_start = self.output.ends_with(['\n', '\r']);

        let current_is_generic_open = self.generic_opens.contains(&current.span().start);
        let current_is_generic_close = self.generic_closes.contains(&current.span().start);
        if current_is_generic_open {
            self.generic_depth += 1;
        } else if current_is_generic_close {
            self.generic_depth -= 1;
        }
        if matches!(current_token, Some(TokenKind::LBracket)) {
            self.bracket_depth += 1;
        } else if matches!(current_token, Some(TokenKind::RBracket)) {
            self.bracket_depth = self.bracket_depth.saturating_sub(1);
        }

        self.previous_was_prefix = current_token
            .is_some_and(|kind| is_prefix_operator(kind, self.previous.and_then(token_kind)));
        self.previous_was_generic_close = current_is_generic_close;
        if matches!(current_token, Some(TokenKind::LBrace)) {
            self.brace_stack.push(BraceFrame {
                parent_indentation: self.indentation,
                brace_indentation: self.current_line_indentation,
                opening: current.span().start,
            });
            self.indentation = self.current_line_indentation + 1;
        } else if matches!(current_token, Some(TokenKind::RBrace))
            && let Some(frame) = self.brace_stack.pop()
        {
            self.indentation = frame.parent_indentation;
        }
        self.previous = Some(current);
        self.gap_line_breaks = 0;
    }

    fn separation(&self, current: &Lexeme) -> Separation {
        let Some(previous) = self.previous else {
            return Separation::None;
        };
        let previous_token = token_kind(previous);
        let current_token = token_kind(current);
        let preserved_break = if self.gap_line_breaks >= 2 {
            Separation::BlankLine
        } else {
            Separation::Newline
        };

        if self.generic_opens.contains(&current.span().start)
            || self.generic_closes.contains(&previous.span().start)
        {
            return Separation::None;
        }
        if self.index_opens.contains(&current.span().start) && !is_comment(previous) {
            return Separation::None;
        }

        // Commas terminate the preceding list item even when a recovery fix
        // inserted one at the beginning of the following source line.
        if matches!(current_token, Some(TokenKind::Comma)) {
            return Separation::None;
        }

        if self.is_multiline_block_opening(current) {
            return Separation::Newline;
        }

        if self.join_before.contains(&current.span().start) {
            return Separation::Space;
        }

        if self.break_after.contains(&previous.span().end) && !is_comment(current) {
            return if matches!(current_token, Some(TokenKind::RBrace)) {
                Separation::Newline
            } else {
                preserved_break
            };
        }

        if self.gap_line_breaks > 0
            && (is_comment(previous) || is_comment(current) || is_doc_comment(current))
        {
            return preserved_break;
        }
        if is_line_comment(previous) || is_doc_comment(previous) {
            return Separation::Newline;
        }
        if is_doc_comment(current) {
            return Separation::Newline;
        }
        if matches!(current_token, Some(TokenKind::RBrace))
            && !matches!(previous_token, Some(TokenKind::LBrace))
        {
            return Separation::Newline;
        }
        if matches!(previous_token, Some(TokenKind::LBrace))
            && !matches!(current_token, Some(TokenKind::RBrace))
        {
            return Separation::Newline;
        }
        if matches!(previous_token, Some(TokenKind::Semicolon)) {
            return if self.bracket_depth > 0 {
                Separation::Space
            } else {
                Separation::Newline
            };
        }
        if matches!(previous_token, Some(TokenKind::RBrace))
            && matches!(current_token, Some(TokenKind::Ident(name)) if name == "else")
        {
            return Separation::Space;
        }
        if self.gap_line_breaks > 0 {
            return preserved_break;
        }
        if is_comment(current) || is_comment(previous) {
            return Separation::Space;
        }
        if needs_space(
            previous_token,
            current_token,
            self.previous_was_prefix,
            self.previous_was_generic_close,
            self.generic_depth,
        ) {
            Separation::Space
        } else {
            Separation::None
        }
    }

    fn write_separation(&mut self, separation: Separation) {
        match separation {
            Separation::None => {}
            Separation::Space => {
                if !self.line_start && !self.output.ends_with(char::is_whitespace) {
                    self.output.push(' ');
                }
            }
            Separation::Newline | Separation::BlankLine => {
                while self.output.ends_with([' ', '\t', '\r', '\n']) {
                    self.output.pop();
                }
                self.output.push('\n');
                if separation == Separation::BlankLine {
                    self.output.push('\n');
                }
                self.line_start = true;
            }
        }
    }

    fn write_indentation(&mut self, current: &Lexeme) {
        if self.line_start {
            let continuation = self.continuation_indentation(current);
            self.current_line_indentation = self.indentation + continuation;
            for _ in 0..self.current_line_indentation {
                self.output.push_str(INDENT);
            }
            self.line_start = false;
        }
    }

    fn is_multiline_block_opening(&self, lexeme: &Lexeme) -> bool {
        self.multiline_headers
            .iter()
            .any(|range| range.opening_brace == lexeme.span().start)
    }

    fn continuation_indentation(&self, lexeme: &Lexeme) -> usize {
        let offset = lexeme.span().start;
        let syntax_continuation = !matches!(
            token_kind(lexeme),
            Some(TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace)
        ) && (self
            .multiline_headers
            .iter()
            .filter(|range| range.start < offset && offset < range.opening_brace)
            .max_by_key(|range| range.start)
            .is_some_and(|range| line_breaks(&self.document.source()[range.start..offset]) > 0)
            || self.continuations.iter().any(|range| {
                range.start < offset
                    && offset < range.end
                    && line_breaks(&self.document.source()[range.start..offset]) > 0
            }));
        let delimiter_continuation = self
            .delimiters
            .iter()
            .filter(|range| {
                self.brace_stack
                    .last()
                    .is_none_or(|brace| brace.opening < range.opening)
                    && offset < range.closing
                    && range
                        .direct_breaks
                        .iter()
                        .any(|break_offset| *break_offset <= offset)
            })
            .count();
        delimiter_continuation.max(usize::from(syntax_continuation))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Parenthesis,
    Bracket,
    Brace,
}

struct PendingDelimiter {
    kind: DelimiterKind,
    opening: usize,
    direct_breaks: Vec<usize>,
}

fn delimiter_ranges(document: &SourceDocument) -> Vec<DelimiterRange> {
    let mut stack = Vec::<PendingDelimiter>::new();
    let mut ranges = Vec::new();
    for lexeme in document.lexemes() {
        match lexeme {
            Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::Whitespace => {
                if line_breaks(document.text(trivia.span)) > 0
                    && let Some(delimiter) = stack.last_mut()
                {
                    delimiter.direct_breaks.push(trivia.span.end);
                }
            }
            Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::BlockComment => {
                if line_breaks(document.text(trivia.span)) > 0
                    && let Some(delimiter) = stack.last_mut()
                {
                    delimiter.direct_breaks.push(trivia.span.end);
                }
            }
            Lexeme::Token(token) => match token.kind {
                TokenKind::LParen => stack.push(PendingDelimiter {
                    kind: DelimiterKind::Parenthesis,
                    opening: token.span.start,
                    direct_breaks: Vec::new(),
                }),
                TokenKind::LBracket => stack.push(PendingDelimiter {
                    kind: DelimiterKind::Bracket,
                    opening: token.span.start,
                    direct_breaks: Vec::new(),
                }),
                TokenKind::RParen => close_delimiter(
                    &mut stack,
                    &mut ranges,
                    DelimiterKind::Parenthesis,
                    token.span.start,
                ),
                TokenKind::RBracket => close_delimiter(
                    &mut stack,
                    &mut ranges,
                    DelimiterKind::Bracket,
                    token.span.start,
                ),
                _ => {}
            },
            Lexeme::Trivia(_) => {}
        }
    }
    ranges
}

#[derive(Default)]
struct TrailingPunctuation {
    commas: HashSet<usize>,
    semicolons: HashSet<usize>,
    omitted_commas: HashSet<usize>,
}

fn trailing_list_punctuation(
    document: &SourceDocument,
    syntax: &Program,
    break_after: &HashSet<usize>,
) -> TrailingPunctuation {
    #[derive(Debug)]
    struct ListDelimiter {
        kind: DelimiterKind,
        has_direct_break: bool,
        has_direct_comma: bool,
        last_direct_comma: Option<usize>,
        pending_break_after_comma: bool,
    }

    let mut stack = Vec::<ListDelimiter>::new();
    let mut closings = HashSet::new();
    let mut omitted_commas = HashSet::new();
    for lexeme in document.lexemes() {
        match lexeme {
            Lexeme::Trivia(trivia)
                if matches!(
                    trivia.kind,
                    TriviaKind::Whitespace | TriviaKind::BlockComment
                ) && line_breaks(document.text(trivia.span)) > 0 =>
            {
                if let Some(delimiter) = stack.last_mut()
                    && delimiter.last_direct_comma.is_some()
                {
                    delimiter.pending_break_after_comma = true;
                }
            }
            Lexeme::Token(token) => {
                let opening = match token.kind {
                    TokenKind::LParen => Some(DelimiterKind::Parenthesis),
                    TokenKind::LBracket => Some(DelimiterKind::Bracket),
                    TokenKind::LBrace => Some(DelimiterKind::Brace),
                    _ => None,
                };
                if let Some(kind) = opening {
                    if let Some(parent) = stack.last_mut() {
                        parent.has_direct_break |= parent.pending_break_after_comma;
                        parent.pending_break_after_comma = false;
                        parent.last_direct_comma = None;
                    }
                    stack.push(ListDelimiter {
                        kind,
                        has_direct_break: false,
                        has_direct_comma: false,
                        last_direct_comma: None,
                        pending_break_after_comma: false,
                    });
                    continue;
                }
                if token.kind == TokenKind::Comma {
                    if let Some(delimiter) = stack.last_mut() {
                        delimiter.has_direct_comma = true;
                        delimiter.has_direct_break |= break_after.contains(&token.span.end);
                        delimiter.last_direct_comma = Some(token.span.start);
                        delimiter.pending_break_after_comma = false;
                    }
                    continue;
                }
                let closing = match token.kind {
                    TokenKind::RParen => Some(DelimiterKind::Parenthesis),
                    TokenKind::RBracket => Some(DelimiterKind::Bracket),
                    TokenKind::RBrace => Some(DelimiterKind::Brace),
                    _ => None,
                };
                if let Some(kind) = closing
                    && let Some(delimiter) = stack.pop()
                    && delimiter.kind == kind
                {
                    if delimiter.has_direct_break && delimiter.has_direct_comma {
                        closings.insert(token.span.start);
                    } else if !delimiter.has_direct_break
                        && let Some(comma) = delimiter.last_direct_comma
                    {
                        omitted_commas.insert(comma);
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.last_direct_comma = None;
                    }
                    continue;
                }
                if let Some(delimiter) = stack.last_mut() {
                    delimiter.has_direct_break |= delimiter.pending_break_after_comma;
                    delimiter.pending_break_after_comma = false;
                    delimiter.last_direct_comma = None;
                }
            }
            Lexeme::Trivia(_) => {}
        }
    }
    let mut collector = TrailingPunctuationCollector {
        document,
        punctuation: TrailingPunctuation {
            commas: closings,
            semicolons: HashSet::new(),
            omitted_commas,
        },
    };
    collector.visit_program(syntax);
    collector.punctuation
}

struct TrailingPunctuationCollector<'a> {
    document: &'a SourceDocument,
    punctuation: TrailingPunctuation,
}

impl TrailingPunctuationCollector<'_> {
    fn mark_comma(&mut self, span: crate::ast::Span) {
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.insert(closing);
        }
    }

    fn mark_semicolon(&mut self, span: crate::ast::Span) {
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.remove(&closing);
            self.punctuation.semicolons.insert(closing);
        }
    }

    fn mark_comma_for_items(
        &mut self,
        span: crate::ast::Span,
        items: impl IntoIterator<Item = crate::ast::Span>,
        single_item_is_vertical: bool,
    ) {
        let items = items.into_iter().collect::<Vec<_>>();
        let itemized = (items.len() == 1
            && (single_item_is_vertical
                || line_breaks(&self.document.source()[span.start..span.end]) > 0))
            || items
                .windows(2)
                .any(|pair| line_breaks(&self.document.source()[pair[0].end..pair[1].start]) > 0);
        if itemized {
            self.mark_comma(span);
            return;
        }
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.remove(&closing);
            if let Some(comma) = trailing_comma_before(self.document, span, closing) {
                self.punctuation.omitted_commas.insert(comma);
            }
        }
    }
}

impl<'ast> Visitor<'ast> for TrailingPunctuationCollector<'_> {
    fn visit_program(&mut self, program: &'ast Program) {
        if !program.settings.is_empty()
            && let Some(span) = program.settings_span
        {
            self.mark_comma(span);
        }
        visit::walk_program(self, program);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        if !state.fields.is_empty() {
            self.mark_semicolon(state.span);
        } else if !state.layouts.is_empty() {
            self.mark_comma(state.span);
        }
        for layout in &state.layouts {
            if !layout.fields.is_empty() {
                self.mark_semicolon(layout.span);
            }
            for field in &layout.fields {
                self.visit_state_field(field);
            }
        }
        for field in &state.fields {
            self.visit_state_field(field);
        }
    }

    fn visit_record(&mut self, record: &'ast RecordDecl) {
        if !record.fields.is_empty() {
            self.mark_comma(record.span);
        }
        visit::walk_record(self, record);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        if !enumeration.variants.is_empty() {
            self.mark_comma(enumeration.span);
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        match &setting.kind {
            SettingKind::Title { .. } => {
                self.mark_comma(setting.span);
            }
            SettingKind::Choice { options, .. } if !options.is_empty() => {
                self.mark_comma(setting.span);
            }
            SettingKind::File { filters } if !filters.is_empty() => {
                self.mark_comma(setting.span);
            }
            SettingKind::Bool { .. } | SettingKind::Choice { .. } | SettingKind::File { .. } => {}
        }
        visit::walk_setting(self, setting);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match &expression.kind {
            ExprKind::Record { fields, .. } if !fields.is_empty() => {
                self.mark_comma_for_items(
                    expression.span,
                    fields.iter().map(|(_, value)| value.span),
                    false,
                );
            }
            ExprKind::Match { arms, .. } if !arms.is_empty() => {
                self.mark_comma(expression.span);
            }
            _ => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn nonempty_closing_brace(document: &SourceDocument, span: crate::ast::Span) -> Option<usize> {
    let tokens = document
        .tokens()
        .filter(|token| span.start <= token.span.start && token.span.end <= span.end)
        .collect::<Vec<_>>();
    let closing_index = tokens
        .iter()
        .rposition(|token| token.kind == TokenKind::RBrace)?;
    let mut depth = 0usize;
    let mut opening_index = None;
    for index in (0..closing_index).rev() {
        match tokens[index].kind {
            TokenKind::RBrace => depth += 1,
            TokenKind::LBrace if depth == 0 => {
                opening_index = Some(index);
                break;
            }
            TokenKind::LBrace => depth -= 1,
            _ => {}
        }
    }
    let opening_index = opening_index?;
    (opening_index + 1 < closing_index).then_some(tokens[closing_index].span.start)
}

fn trailing_comma_before(
    document: &SourceDocument,
    span: crate::ast::Span,
    closing: usize,
) -> Option<usize> {
    document
        .tokens()
        .take_while(|token| token.span.start < closing)
        .filter(|token| span.start <= token.span.start)
        .last()
        .filter(|token| token.kind == TokenKind::Comma)
        .map(|token| token.span.start)
}

fn close_delimiter(
    stack: &mut Vec<PendingDelimiter>,
    ranges: &mut Vec<DelimiterRange>,
    expected: DelimiterKind,
    closing: usize,
) {
    let Some(delimiter) = stack.pop() else {
        return;
    };
    if delimiter.kind == expected && !delimiter.direct_breaks.is_empty() {
        ranges.push(DelimiterRange {
            opening: delimiter.opening,
            closing,
            direct_breaks: delimiter.direct_breaks,
        });
    }
}

fn token_kind(lexeme: &Lexeme) -> Option<&TokenKind> {
    match lexeme {
        Lexeme::Token(token) => Some(&token.kind),
        Lexeme::Trivia(_) => None,
    }
}

fn is_comment(lexeme: &Lexeme) -> bool {
    matches!(
        lexeme,
        Lexeme::Trivia(trivia)
            if matches!(trivia.kind, TriviaKind::LineComment | TriviaKind::BlockComment)
    )
}

fn is_line_comment(lexeme: &Lexeme) -> bool {
    matches!(lexeme, Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::LineComment)
}

fn is_doc_comment(lexeme: &Lexeme) -> bool {
    matches!(token_kind(lexeme), Some(TokenKind::DocComment(_)))
}

fn line_breaks(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => count += 1,
            b'\r' => {
                count += 1;
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    count
}

fn needs_space(
    previous: Option<&TokenKind>,
    current: Option<&TokenKind>,
    previous_was_prefix: bool,
    previous_was_generic_close: bool,
    generic_depth: usize,
) -> bool {
    let (Some(previous), Some(current)) = (previous, current) else {
        return false;
    };

    if matches!(previous, TokenKind::Minus) && matches!(current, TokenKind::Gt) {
        return false;
    }
    if matches!(previous, TokenKind::Ident(name) if matches!(name.as_str(), "sig" | "v"))
        && matches!(current, TokenKind::String(_))
    {
        return false;
    }
    if generic_depth > 0 && (matches!(previous, TokenKind::Lt) || matches!(current, TokenKind::Gt))
    {
        return false;
    }
    // A block opening is always separated from its header. This must take
    // precedence over prefix-operator tracking because `!` is also the postfix
    // Result marker in a return type.
    if matches!(current, TokenKind::LBrace) {
        return true;
    }
    if previous_was_prefix {
        return false;
    }
    if previous_was_generic_close && matches!(current, TokenKind::Bang | TokenKind::Question) {
        return false;
    }

    if matches!(
        current,
        TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::RBracket
            | TokenKind::Comma
            | TokenKind::Semicolon
            | TokenKind::Dot
            | TokenKind::Colon
            | TokenKind::Question
            | TokenKind::TemplateChunk(_)
            | TokenKind::TemplateExprStart
            | TokenKind::TemplateExprEnd
            | TokenKind::TemplateEnd
    ) || matches!(
        previous,
        TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::Dot
            | TokenKind::TemplateStart
            | TokenKind::TemplateChunk(_)
            | TokenKind::TemplateExprStart
            | TokenKind::TemplateExprEnd
    ) {
        return false;
    }

    if matches!(current, TokenKind::LParen) {
        return matches!(previous, TokenKind::Ident(name) if matches!(name.as_str(), "if" | "while" | "match"))
            || (!previous_was_generic_close && is_spaced_operator(previous));
    }
    if matches!(current, TokenKind::LBracket) {
        return true;
    }
    if matches!(previous, TokenKind::Comma | TokenKind::Colon) {
        return true;
    }
    if matches!(current, TokenKind::Bang | TokenKind::Tilde) {
        return matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
            || is_spaced_operator(previous);
    }
    if matches!(current, TokenKind::Minus) && is_prefix_boundary(previous) {
        return !matches!(
            previous,
            TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Comma
                | TokenKind::Colon
                | TokenKind::TemplateExprStart
        );
    }
    if is_spaced_operator(previous) || is_spaced_operator(current) {
        return true;
    }

    true
}

fn is_prefix_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "while" | "match" | "return" | "throw" | "else" | "await" | "retry"
    )
}

fn is_prefix_operator(current: &TokenKind, previous: Option<&TokenKind>) -> bool {
    matches!(current, TokenKind::Bang | TokenKind::Tilde)
        && previous.is_none_or(|previous| {
            is_prefix_boundary(previous)
                || matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
        })
        || matches!(current, TokenKind::Minus) && previous.is_none_or(is_prefix_boundary)
}

fn is_prefix_boundary(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::Comma
            | TokenKind::Colon
            | TokenKind::FatArrow
            | TokenKind::Assign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::PercentAssign
            | TokenKind::OrAssign
            | TokenKind::AndAssign
            | TokenKind::CaretAssign
            | TokenKind::ShlAssign
            | TokenKind::ShrAssign
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Or
            | TokenKind::And
            | TokenKind::Caret
            | TokenKind::OrOr
            | TokenKind::AndAnd
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Shl
            | TokenKind::Shr
            | TokenKind::TemplateExprStart
    )
}

fn is_spaced_operator(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::FatArrow
            | TokenKind::Assign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::PercentAssign
            | TokenKind::OrAssign
            | TokenKind::AndAssign
            | TokenKind::CaretAssign
            | TokenKind::ShlAssign
            | TokenKind::ShrAssign
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Or
            | TokenKind::And
            | TokenKind::Caret
            | TokenKind::OrOr
            | TokenKind::AndAnd
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Shl
            | TokenKind::Shr
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_basic_declarations_statements_and_expressions() {
        let source = r#"state "game.exe"{}
fn add(left:i32,right:i32)->i32{let sum=left+right;if sum>10{return sum}else{return 10}}
whileAttached{let answer=add(1,2);print(answer as String)}"#;
        let expected = r#"state "game.exe" {}
fn add(left: i32, right: i32) -> i32 {
    let sum = left + right;
    if sum > 10 {
        return sum
    } else {
        return 10
    }
}
whileAttached {
    let answer = add(1, 2);
    print(answer as String)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn uses_trailing_commas_only_for_multiline_lists() {
        let source = r#"record Point { x: i32, y: i32, }
enum Mode { First, Second, }
state "game.exe" { value = 1 }
fn compact() { print(1, 2,) }
fn multiline() {
    print(
        1,
        2
    )
    let values = [
        1,
        2
    ]
}"#;

        let formatted = format_source(source).unwrap();
        assert!(
            formatted.contains("record Point {\n    x: i32,\n    y: i32,\n}"),
            "{formatted}"
        );
        assert!(
            formatted.contains("enum Mode {\n    First,\n    Second,\n}"),
            "{formatted}"
        );
        assert!(formatted.contains("print(1, 2)"), "{formatted}");
        assert!(formatted.contains("        2,\n    )"), "{formatted}");
        assert!(formatted.contains("        2,\n    ]"), "{formatted}");
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_literal_spellings_templates_signatures_and_type_postfixes() {
        let source = r#"state "game.exe"{bytes:[u8;6] at 0x100}
fn probe(value:[u8]!)->String{let missing:[u8]?=None;let outcome:i32! = Err("missing");let fixed:[u8;3]=[1,2,3];let sigValue=sig"48 8B ??";let version=v"1.2.3.4";return `{value.length()}:{fixed.length()}:{missing==None}:{sigValue as String}`}
fn nested()->[[u8]]{return [[1]]}
fn fallible()->f32!{return Err("missing")}
fn optional()->f32?{return None}"#;
        let source = format!("{source}\nfn asynchronous()->async f32{{return 1.0}}");
        let formatted = format_source(&source).unwrap();

        assert!(formatted.contains("bytes: [u8; 6] at 0x100"), "{formatted}");
        assert!(
            formatted.contains("fixed: [u8; 3] = [1, 2, 3]"),
            "{formatted}"
        );
        assert!(formatted.contains("value: [u8]!"), "{formatted}");
        assert!(formatted.contains("missing: [u8]? = None"));
        assert!(formatted.contains("outcome: i32! = Err(\"missing\")"));
        assert!(!formatted.contains("i32!="));
        assert!(formatted.contains("fn nested() -> [[u8]] {"), "{formatted}");
        assert!(formatted.contains("fn fallible() -> f32! {"), "{formatted}");
        assert!(formatted.contains("fn optional() -> f32? {"), "{formatted}");
        assert!(
            formatted.contains("fn asynchronous() -> async f32 {"),
            "{formatted}"
        );
        assert!(!formatted.contains("f32!{"), "{formatted}");
        assert!(!formatted.contains("f32?{"), "{formatted}");
        assert!(formatted.contains("sig\"48 8B ??\""));
        assert!(formatted.contains("v\"1.2.3.4\""));
        assert!(formatted.contains("`{value.length()}:"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn joins_array_indexes_to_their_receiver() {
        let source = r#"state "game.exe" {}
fn select(matrix: [[i32]], row: u32, column: u32) -> i32 {
    return matrix [ row ] [ column ]
}"#;
        let formatted = format_source(source).unwrap();
        assert!(
            formatted.contains("return matrix[row][column]"),
            "{formatted}"
        );
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn formats_bounded_state_string_decoders_as_part_of_the_pointer_path() {
        let source = r#"state "game.exe"{mapName at "game.dll",0x100,0x20 as utf8(64)}"#;
        let expected = r#"state "game.exe" {
    mapName at "game.dll", 0x100, 0x20 as utf8(64);
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn returns_parser_diagnostics_for_invalid_source() {
        let errors = format_source("state \"game.exe\" { let broken = }").unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn preserves_ordinary_and_setting_documentation_comments() {
        let source = "  // heading\r\nstate \"game.exe\"{} // process\r\n\r\nsettings{\r\n/// first line\r\n/// second line\r\n\"Enabled\"=>enabled key \"legacy-enabled\":true // setting\r\n}\r\n/* footer */";
        let expected = r#"// heading
state "game.exe" {} // process

settings {
    /// first line
    /// second line
    "Enabled" => enabled key "legacy-enabled": true // setting
}
/* footer */
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_source_declaration_documentation_comments() {
        let source = r#"state "game.exe"{
/// Current stage.
stage:i32 at 0x100
}
/// Shared counter.
let count=0
/// Coordinate pair.
record Point{
/// Horizontal coordinate.
x:i32
}
/// Converts a point.
fn describe(point:Point){return point.x as String}
"#;
        let expected = r#"state "game.exe" {
    /// Current stage.
    stage: i32 at 0x100;
}
/// Shared counter.
let count = 0
/// Coordinate pair.
record Point {
    /// Horizontal coordinate.
    x: i32,
}
/// Converts a point.
fn describe(point: Point) {
    return point.x as String
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn puts_a_multiline_header_brace_at_the_header_indentation() {
        let source = r#"state "game.exe" {}
split {
    if old.timerStopped != current.timerStopped
        && !current.timerStopped
        && isFirstLevel(current.levelOrScene) {
        return true
    }

}"#;
        let expected = r#"state "game.exe" {}
split {
    if old.timerStopped != current.timerStopped
        && !current.timerStopped
        && isFirstLevel(current.levelOrScene)
    {
        return true
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_for_headers_and_bodies_idempotently() {
        let source = r#"state "game.exe"{}
whileAttached{for value in [1,2,3]{if value==2{continue}print(value as String)}}"#;
        let expected = r#"state "game.exe" {}
whileAttached {
    for value in [1, 2, 3] {
        if value == 2 {
            continue
        }
        print(value as String)
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_intentional_blank_lines_between_control_flow_statements() {
        let separated = r#"state "game.exe" {}
split {
    if first {
        return true
    }

    if second {
        return true
    }
}
"#;
        assert_eq!(format_source(separated).unwrap(), separated);

        let compact = r#"state "game.exe" {}
split {
    if first {
        return true
    }
    if second {
        return true
    }
}
"#;
        assert_eq!(format_source(compact).unwrap(), compact);
    }

    #[test]
    fn indents_state_reads_and_breaks_match_arms() {
        let source = r#"state "game.exe" {
value = process.read(
0x1000
)
}
fn describe(value) {
return match value {0=>"zero",1=>`value {value+1}`,_=>"other"}
}"#;
        let expected = r#"state "game.exe" {
    value = process.read(
        0x1000
    );
}
fn describe(value) {
    return match value {
        0 => "zero",
        1 => `value {value + 1}`,
        _ => "other",
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn indents_multiline_statement_expressions_without_indenting_nested_blocks_twice() {
        let source = r#"state "game.exe" {
choice = if enabled {
value
} else {
fallback
}
}
split {
let creditsTransition = !isDlcDemo
&& isFinalBaseLevel(old.levelOrScene)
&& isBaseCredits(current.levelOrScene)
return creditsTransition
}"#;
        let expected = r#"state "game.exe" {
    choice = if enabled {
        value
    } else {
        fallback
    };
}
split {
    let creditsTransition = !isDlcDemo
        && isFinalBaseLevel(old.levelOrScene)
        && isBaseCredits(current.levelOrScene)
    return creditsTransition
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn block_indentation_supersedes_an_enclosing_multiline_call() {
        let source = r#"state "game.exe" {}
enum DelayedSplit {
Inactive,
ReceiveMinishCap,
GetFourSword
}
whileAttached {
debug print(match delayedSplit {
DelayedSplit.ReceiveMinishCap => "Receive Minish Cap",
DelayedSplit.GetFourSword => "Get Four Sword",
DelayedSplit.Inactive => "Delayed split"
})
}"#;
        let expected = r#"state "game.exe" {}
enum DelayedSplit {
    Inactive,
    ReceiveMinishCap,
    GetFourSword,
}
whileAttached {
    debug print(match delayedSplit {
        DelayedSplit.ReceiveMinishCap => "Receive Minish Cap",
        DelayedSplit.GetFourSword => "Get Four Sword",
        DelayedSplit.Inactive => "Delayed split",
    })
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn indents_nested_settings_and_their_specialized_entries() {
        let source = r#"state "game.exe" {}
enum Mode {
A,
B
}
settings{"Group"{/// Enables the feature.
"Enabled"
=>enabled:true,"Mode"
=>mode:choice{"First"=>Mode.A,"Second"=>Mode.B default},"Input"=>input:file{"Text"=>"*.txt",mime=>"text/plain"}}}"#;
        let expected = r#"state "game.exe" {}
enum Mode {
    A,
    B,
}
settings {
    "Group" {
        /// Enables the feature.
        "Enabled" => enabled: true,
        "Mode" => mode: choice {
            "First" => Mode.A,
            "Second" => Mode.B default,
        },
        "Input" => input: file {
            "Text" => "*.txt",
            mime => "text/plain",
        },
    },
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_the_language_showcases_idempotently() {
        for source in [
            include_str!("../examples/lunistice.split"),
            include_str!("../examples/hello_lunistice.split"),
            include_str!("../examples/lso_desktop_settings.split"),
            include_str!("../examples/minish_cap.split"),
        ] {
            let source = source.replace("\r\n", "\n");
            for input in [&source, &source.replace('\n', "\r\n")] {
                let formatted = format_source(input).unwrap();
                crate::parse(&formatted).unwrap();
                assert_eq!(format_source(&formatted).unwrap(), formatted);
            }
        }
        let minish_cap = include_str!("../examples/minish_cap.split").replace("\r\n", "\n");
        assert_eq!(format_source(&minish_cap).unwrap(), minish_cap);
        assert!(
            format_source(include_str!("../examples/lunistice.split"))
                .unwrap()
                .contains("state [\"Lunistice.exe\"")
        );
    }

    #[test]
    fn formats_named_state_layouts_and_their_selector() {
        let source = r#"state "game.exe"{layout Steam{level:u32 at 0x100},layout GOG{level:u32 at 0x200}}onAttach{return StateLayout.Steam}"#;
        let expected = r#"state "game.exe" {
    layout Steam {
        level: u32 at 0x100;
    },
    layout GOG {
        level: u32 at 0x200;
    },
}
onAttach {
    return StateLayout.Steam
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_generic_call_arguments_attached_without_a_turbofish() {
        let source = r#"state "game.exe"{}
whileAttached{let value=process.read<[u8;4]> (0);print<u32> (value [0])}"#;
        let expected = r#"state "game.exe" {}
whileAttached {
    let value = process.read<[u8; 4]>(0);
    print<u32>(value[0])
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn spaces_grouped_operands_after_binary_operators() {
        let source = r#"state "game.exe"{}
fn atMissionPoint(point:MissionPoint,y:f64)->bool{return y==(point.y as f64).roundTo(3)}"#;
        let expected = r#"state "game.exe" {}
fn atMissionPoint(point: MissionPoint, y: f64) -> bool {
    return y == (point.y as f64).roundTo(3)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}
