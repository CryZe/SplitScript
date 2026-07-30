//! Compiler-owned semantic highlighting shared by the LSP and future editors.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    ast::{
        Action, EnumTypeId, Expr, ExprKind, FunctionDecl, MatchPattern, Program, SettingKind, Span,
        Stmt, TypeRef, ValueId, VariableDecl,
    },
    lexer::{Lexeme, Token, TokenKind, TriviaKind},
    semantic::{ResolvedCall, ResolvedValue, SemanticModel},
    stdlib::StandardLibrary,
    syntax::SourceDocument,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SemanticTokenKind {
    Keyword,
    Type,
    Struct,
    Enum,
    EnumMember,
    Function,
    Method,
    Variable,
    Parameter,
    Property,
    Namespace,
    String,
    Number,
    Operator,
    Comment,
    Setting,
    SettingTitle,
    StateField,
    Lifecycle,
    Signature,
    Debug,
}

impl SemanticTokenKind {
    pub const ALL: [Self; 21] = [
        Self::Keyword,
        Self::Type,
        Self::Struct,
        Self::Enum,
        Self::EnumMember,
        Self::Function,
        Self::Method,
        Self::Variable,
        Self::Parameter,
        Self::Property,
        Self::Namespace,
        Self::String,
        Self::Number,
        Self::Operator,
        Self::Comment,
        Self::Setting,
        Self::SettingTitle,
        Self::StateField,
        Self::Lifecycle,
        Self::Signature,
        Self::Debug,
    ];

    pub const fn index(self) -> u32 {
        self as u32
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::EnumMember => "enumMember",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Namespace => "namespace",
            Self::String => "string",
            Self::Number => "number",
            Self::Operator => "operator",
            Self::Comment => "comment",
            Self::Setting => "setting",
            Self::SettingTitle => "settingTitle",
            Self::StateField => "stateField",
            Self::Lifecycle => "lifecycle",
            Self::Signature => "signature",
            Self::Debug => "debug",
        }
    }
}

pub const MODIFIER_DECLARATION: u32 = 1 << 0;
pub const MODIFIER_READONLY: u32 = 1 << 1;
pub const MODIFIER_DEFAULT_LIBRARY: u32 = 1 << 2;
pub const MODIFIER_DEBUG: u32 = 1 << 3;

pub const SEMANTIC_TOKEN_MODIFIERS: [&str; 4] =
    ["declaration", "readonly", "defaultLibrary", "debug"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticHighlight {
    pub span: Span,
    pub kind: SemanticTokenKind,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticHighlightIndex {
    highlights: Vec<SemanticHighlight>,
}

impl SemanticHighlightIndex {
    pub(crate) fn build(
        document: &SourceDocument,
        syntax: &Program,
        semantics: Option<&SemanticModel>,
    ) -> Self {
        let mut value_kinds = ValueKindCollector::default();
        value_kinds.visit_program(syntax);
        let record_names = syntax
            .records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<HashSet<_>>();
        let enum_names = syntax
            .enums
            .iter()
            .map(|enumeration| enumeration.name.as_str())
            .collect::<HashSet<_>>();
        let function_names = syntax
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<HashSet<_>>();
        let mut collector = HighlightCollector {
            document,
            syntax,
            semantics,
            entries: BTreeMap::new(),
            value_kinds: value_kinds.kinds,
            debug_ranges: value_kinds.debug_ranges,
        };
        collector.base_tokens(&record_names, &enum_names, &function_names);
        collector.visit_program(syntax);
        collector.apply_debug_modifiers();
        Self {
            highlights: collector.entries.into_values().collect(),
        }
    }

    pub fn highlights(&self) -> &[SemanticHighlight] {
        &self.highlights
    }
}

#[derive(Default)]
struct ValueKindCollector {
    kinds: HashMap<ValueId, SemanticTokenKind>,
    debug_ranges: Vec<Span>,
}

impl<'ast> Visitor<'ast> for ValueKindCollector {
    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.kinds.insert(field.id, SemanticTokenKind::StateField);
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        self.kinds.insert(setting.id, SemanticTokenKind::Setting);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        if function.debug_only {
            self.debug_ranges.push(function.span);
        }
        for parameter in &function.params {
            self.kinds
                .insert(parameter.id, SemanticTokenKind::Parameter);
        }
        visit::walk_function(self, function);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        self.kinds.insert(variable.id, SemanticTokenKind::Variable);
        if variable.debug_only {
            self.debug_ranges.push(variable.span);
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.kinds.insert(binding.id, SemanticTokenKind::Variable);
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        let binding = match &arm.pattern {
            MatchPattern::Enum {
                binding: Some(binding),
                ..
            }
            | MatchPattern::OptionSome(Some(binding))
            | MatchPattern::ResultSuccess(Some(binding))
            | MatchPattern::ResultError(Some(binding)) => Some(binding),
            _ => None,
        };
        if let Some(binding) = binding {
            self.kinds.insert(binding.id, SemanticTokenKind::Variable);
        }
        visit::walk_match_arm(self, arm);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Debug { span, .. } = statement {
            self.debug_ranges.push(*span);
        }
        visit::walk_stmt(self, statement);
    }
}

struct HighlightCollector<'a> {
    document: &'a SourceDocument,
    syntax: &'a Program,
    semantics: Option<&'a SemanticModel>,
    entries: BTreeMap<usize, SemanticHighlight>,
    value_kinds: HashMap<ValueId, SemanticTokenKind>,
    debug_ranges: Vec<Span>,
}

impl HighlightCollector<'_> {
    fn base_tokens(
        &mut self,
        record_names: &HashSet<&str>,
        enum_names: &HashSet<&str>,
        function_names: &HashSet<&str>,
    ) {
        let mut previous_token: Option<&Token> = None;
        for lexeme in self.document.lexemes() {
            match lexeme {
                Lexeme::Trivia(trivia)
                    if matches!(
                        trivia.kind,
                        TriviaKind::LineComment | TriviaKind::BlockComment
                    ) =>
                {
                    self.insert(trivia.span, SemanticTokenKind::Comment, 0);
                }
                Lexeme::Trivia(_) => {}
                Lexeme::Token(token) => {
                    let kind = match &token.kind {
                        TokenKind::Ident(name) if name == "debug" => Some(SemanticTokenKind::Debug),
                        TokenKind::Ident(name) if name == "sig" => {
                            Some(SemanticTokenKind::Signature)
                        }
                        TokenKind::Ident(name) if is_keyword(name) => {
                            Some(SemanticTokenKind::Keyword)
                        }
                        TokenKind::Ident(name) if is_builtin_type(name) => {
                            Some(SemanticTokenKind::Type)
                        }
                        TokenKind::Ident(name) if record_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Struct)
                        }
                        TokenKind::Ident(name) if enum_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Enum)
                        }
                        TokenKind::Ident(name) if function_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Function)
                        }
                        TokenKind::Ident(name) if is_namespace(name) => {
                            Some(SemanticTokenKind::Namespace)
                        }
                        TokenKind::String(_)
                            if previous_token.is_some_and(|previous| {
                                matches!(&previous.kind, TokenKind::Ident(name) if name == "sig")
                            }) => Some(SemanticTokenKind::Signature),
                        TokenKind::String(_)
                        | TokenKind::TemplateStart
                        | TokenKind::TemplateChunk(_)
                        | TokenKind::TemplateEnd => Some(SemanticTokenKind::String),
                        TokenKind::DocComment(_) => Some(SemanticTokenKind::Comment),
                        TokenKind::Int(_) | TokenKind::Float(_) => Some(SemanticTokenKind::Number),
                        kind if is_operator(kind) => Some(SemanticTokenKind::Operator),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        self.insert(token.span, kind, 0);
                    }
                    if token.kind != TokenKind::Eof {
                        previous_token = Some(token);
                    }
                }
            }
        }
    }

    fn insert(&mut self, span: Span, kind: SemanticTokenKind, modifiers: u32) {
        if span.start != span.end {
            self.entries.insert(
                span.start,
                SemanticHighlight {
                    span,
                    kind,
                    modifiers,
                },
            );
        }
    }

    fn mark_ident(&mut self, span: Span, name: &str, kind: SemanticTokenKind, modifiers: u32) {
        if let Some(token) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
        }) {
            self.insert(token.span, kind, modifiers);
        }
    }

    fn mark_last_ident_before(
        &mut self,
        span: Span,
        end: usize,
        name: &str,
        kind: SemanticTokenKind,
        modifiers: u32,
    ) {
        if let Some(token) = self
            .document
            .tokens()
            .filter(|token| {
                span.start <= token.span.start
                    && token.span.end <= end
                    && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
            })
            .last()
        {
            self.insert(token.span, kind, modifiers);
        }
    }

    fn mark_first_string(&mut self, span: Span, kind: SemanticTokenKind) {
        if let Some(token) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && matches!(token.kind, TokenKind::String(_))
        }) {
            self.insert(token.span, kind, MODIFIER_DECLARATION | MODIFIER_READONLY);
        }
    }

    fn mark_path(&mut self, expression: &Expr, names: &[String], call: bool) {
        let mut tokens = self.document.tokens().filter(|token| {
            expression.span.start <= token.span.start && token.span.end <= expression.span.end
        });
        let spans = names
            .iter()
            .filter_map(|name| {
                tokens.find_map(|token| {
                    matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
                        .then_some(token.span)
                })
            })
            .collect::<Vec<_>>();
        if spans.is_empty() {
            return;
        }

        if let Some(value) = self.semantics.and_then(|model| model.value(expression.id)) {
            match value {
                ResolvedValue::Variable(id) => {
                    self.insert(spans[0], self.value_kind(id), 0);
                }
                ResolvedValue::CurrentState(_) | ResolvedValue::OldState(_) => {
                    self.insert(spans[0], SemanticTokenKind::Namespace, MODIFIER_READONLY);
                    if let Some(span) = spans.get(1) {
                        self.insert(*span, SemanticTokenKind::StateField, MODIFIER_READONLY);
                    }
                }
                ResolvedValue::Setting(id) | ResolvedValue::OldSetting(id) => {
                    self.insert(spans[0], SemanticTokenKind::Namespace, MODIFIER_READONLY);
                    if let Some(span) = spans.get(1) {
                        self.insert(*span, self.value_kind(id), MODIFIER_READONLY);
                    }
                }
            }
        } else if matches!(names.first().map(String::as_str), Some("current" | "old")) {
            self.insert(spans[0], SemanticTokenKind::Namespace, MODIFIER_READONLY);
            if let Some(span) = spans.get(1) {
                self.insert(*span, SemanticTokenKind::StateField, MODIFIER_READONLY);
            }
        } else if matches!(
            names.first().map(String::as_str),
            Some("settings" | "oldSettings")
        ) {
            self.insert(spans[0], SemanticTokenKind::Namespace, MODIFIER_READONLY);
            if let Some(span) = spans.get(1) {
                self.insert(*span, SemanticTokenKind::Setting, MODIFIER_READONLY);
            }
        }

        for span in spans.iter().skip(1) {
            self.entries.entry(span.start).or_insert(SemanticHighlight {
                span: *span,
                kind: SemanticTokenKind::Property,
                modifiers: MODIFIER_READONLY,
            });
        }
        if call {
            let target = *spans.last().unwrap();
            let resolution = self
                .semantics
                .and_then(|semantics| semantics.call(expression.id));
            let kind = match resolution {
                Some(
                    ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. }
                    | ResolvedCall::ResultError { .. },
                ) => SemanticTokenKind::EnumMember,
                Some(ResolvedCall::UserMethod { .. }) => SemanticTokenKind::Method,
                _ if spans.len() == 1 => SemanticTokenKind::Function,
                _ => SemanticTokenKind::Method,
            };
            let modifiers = if matches!(resolution, Some(ResolvedCall::StandardLibrary { .. })) {
                MODIFIER_DEFAULT_LIBRARY
            } else {
                0
            };
            self.insert(target, kind, modifiers);
        }
    }

    fn value_kind(&self, id: ValueId) -> SemanticTokenKind {
        self.value_kinds
            .get(&id)
            .copied()
            .unwrap_or(SemanticTokenKind::Variable)
    }

    fn apply_debug_modifiers(&mut self) {
        for highlight in self.entries.values_mut() {
            if self
                .debug_ranges
                .iter()
                .any(|range| range.start <= highlight.span.start && highlight.span.end <= range.end)
            {
                highlight.modifiers |= MODIFIER_DEBUG;
            }
        }
    }
}

impl<'ast> Visitor<'ast> for HighlightCollector<'_> {
    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.mark_ident(
            field.span,
            &field.name,
            SemanticTokenKind::StateField,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        );
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        if matches!(setting.kind, SettingKind::Title { .. }) {
            self.mark_first_string(setting.span, SemanticTokenKind::SettingTitle);
        } else {
            self.mark_first_string(setting.span, SemanticTokenKind::Setting);
            self.mark_ident(
                setting.span,
                &setting.name,
                SemanticTokenKind::Setting,
                MODIFIER_DECLARATION,
            );
            if let SettingKind::Choice {
                enumeration,
                options,
                ..
            } = &setting.kind
                && let Some(enumeration) = self
                    .syntax
                    .enums
                    .iter()
                    .find(|candidate| candidate.id == *enumeration)
            {
                for option in options {
                    self.mark_ident(option.span, &enumeration.name, SemanticTokenKind::Enum, 0);
                    self.mark_ident(
                        option.span,
                        &option.variant,
                        SemanticTokenKind::EnumMember,
                        0,
                    );
                }
            }
        }
    }

    fn visit_record(&mut self, record: &'ast crate::ast::RecordDecl) {
        self.mark_ident(
            record.span,
            &record.name,
            SemanticTokenKind::Struct,
            MODIFIER_DECLARATION,
        );
        for field in &record.fields {
            self.mark_ident(
                field.span,
                &field.name,
                SemanticTokenKind::Property,
                MODIFIER_DECLARATION | MODIFIER_READONLY,
            );
        }
        visit::walk_record(self, record);
    }

    fn visit_enum(&mut self, enumeration: &'ast crate::ast::EnumDecl) {
        self.mark_ident(
            enumeration.span,
            &enumeration.name,
            SemanticTokenKind::Enum,
            MODIFIER_DECLARATION,
        );
        for variant in &enumeration.variants {
            self.mark_ident(
                variant.span,
                &variant.name,
                SemanticTokenKind::EnumMember,
                MODIFIER_DECLARATION | MODIFIER_READONLY,
            );
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        self.mark_last_ident_before(
            function.span,
            function.body.span.start,
            &function.name,
            if function.method_of.is_some() {
                SemanticTokenKind::Method
            } else {
                SemanticTokenKind::Function
            },
            MODIFIER_DECLARATION,
        );
        for parameter in &function.params {
            self.mark_ident(
                parameter.span,
                &parameter.name,
                SemanticTokenKind::Parameter,
                MODIFIER_DECLARATION,
            );
        }
        visit::walk_function(self, function);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.mark_ident(
            action.span,
            action.kind.name(),
            SemanticTokenKind::Lifecycle,
            MODIFIER_DECLARATION,
        );
        self.visit_block(&action.body);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        self.mark_ident(
            Span {
                start: variable.span.start,
                end: variable.value.span.start,
            },
            &variable.name,
            SemanticTokenKind::Variable,
            MODIFIER_DECLARATION,
        );
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.mark_ident(
            binding.span,
            &binding.name,
            SemanticTokenKind::Variable,
            MODIFIER_DECLARATION,
        );
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        match &arm.pattern {
            MatchPattern::Enum {
                enumeration,
                variant,
                binding,
            } => {
                if let Some(name) = enum_name(self.syntax, *enumeration) {
                    self.mark_ident(arm.span, name, SemanticTokenKind::Enum, 0);
                }
                self.mark_ident(arm.span, variant, SemanticTokenKind::EnumMember, 0);
                if let Some(binding) = binding {
                    self.mark_ident(
                        arm.span,
                        &binding.name,
                        SemanticTokenKind::Variable,
                        MODIFIER_DECLARATION,
                    );
                }
            }
            MatchPattern::OptionSome(Some(binding))
            | MatchPattern::ResultSuccess(Some(binding))
            | MatchPattern::ResultError(Some(binding)) => self.mark_ident(
                arm.span,
                &binding.name,
                SemanticTokenKind::Variable,
                MODIFIER_DECLARATION,
            ),
            _ => {}
        }
        visit::walk_match_arm(self, arm);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match &expression.kind {
            ExprKind::Path(names) => self.mark_path(expression, names, false),
            ExprKind::Member { name_span, .. } => {
                self.insert(*name_span, SemanticTokenKind::Property, MODIFIER_READONLY)
            }
            ExprKind::Call { callee, .. } => self.mark_path(expression, callee, true),
            ExprKind::Enum {
                enumeration,
                variant,
                ..
            } => {
                if let Some(name) = enum_name(self.syntax, *enumeration) {
                    self.mark_ident(expression.span, name, SemanticTokenKind::Enum, 0);
                }
                self.mark_ident(expression.span, variant, SemanticTokenKind::EnumMember, 0);
            }
            ExprKind::Record { record, fields } => {
                if let Some(record) = self
                    .syntax
                    .records
                    .iter()
                    .find(|candidate| candidate.id == *record)
                {
                    self.mark_ident(expression.span, &record.name, SemanticTokenKind::Struct, 0);
                }
                for (name, value) in fields {
                    self.mark_last_ident_before(
                        expression.span,
                        value.span.start,
                        name,
                        SemanticTokenKind::Property,
                        MODIFIER_READONLY,
                    );
                }
            }
            _ => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn enum_name(program: &Program, enumeration: EnumTypeId) -> Option<&str> {
    match enumeration {
        EnumTypeId::Source(id) => program
            .enums
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|declaration| declaration.name.as_str()),
        EnumTypeId::Standard(id) => Some(StandardLibrary::new().type_decl(id).name),
    }
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "state"
            | "settings"
            | "record"
            | "enum"
            | "fn"
            | "let"
            | "if"
            | "else"
            | "while"
            | "break"
            | "continue"
            | "return"
            | "throw"
            | "await"
            | "retry"
            | "match"
            | "as"
            | "at"
            | "default"
            | "choice"
            | "file"
            | "mime"
            | "true"
            | "false"
            | "None"
            | "Some"
            | "Ok"
            | "Err"
    )
}

fn is_builtin_type(name: &str) -> bool {
    TypeRef::parse(name).is_some() || matches!(name, "void" | "Signature" | "Array")
}

fn is_namespace(name: &str) -> bool {
    matches!(name, "current" | "old" | "settings" | "oldSettings")
        || StandardLibrary::new().namespace_by_name(name).is_some()
}

fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
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
            | TokenKind::Bang
            | TokenKind::Question
            | TokenKind::Tilde
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
    use crate::database::CompilerDatabase;

    fn contains(
        source: &str,
        index: &SemanticHighlightIndex,
        spelling: &str,
        kind: SemanticTokenKind,
        modifiers: u32,
    ) -> bool {
        index.highlights().iter().any(|highlight| {
            &source[highlight.span.start..highlight.span.end] == spelling
                && highlight.kind == kind
                && highlight.modifiers & modifiers == modifiers
        })
    }

    #[test]
    fn compiler_highlights_domain_constructs_and_resolved_references() {
        let source = r#"
enum Mode {
    Active
}

state "game.exe" {
    level = process.read.i32(0)
}

settings {
    "General" {
        "Enabled" => enabled: true
    }
}

debug fn inspect(mode: Mode) {
    debug print(mode as String)
}

whileAttached {
    let mode = Mode.Active
    let marker = await process.scan(0, 1, sig"48 ??")
    if current.level == 1 {
        inspect(mode)
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let first = database
            .semantic_highlights()
            .expect("highlighting should survive incomplete semantic information");
        let second = database
            .semantic_highlights()
            .expect("highlighting should be cached");
        assert!(std::sync::Arc::ptr_eq(&first, &second));

        assert!(contains(
            source,
            &first,
            "\"General\"",
            SemanticTokenKind::SettingTitle,
            MODIFIER_DECLARATION | MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &first,
            "enabled",
            SemanticTokenKind::Setting,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "level",
            SemanticTokenKind::StateField,
            MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &first,
            "whileAttached",
            SemanticTokenKind::Lifecycle,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "Mode",
            SemanticTokenKind::Enum,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "Active",
            SemanticTokenKind::EnumMember,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "\"48 ??\"",
            SemanticTokenKind::Signature,
            0
        ));
        assert!(contains(
            source,
            &first,
            "sig",
            SemanticTokenKind::Signature,
            0
        ));
        assert!(first.highlights().iter().any(|highlight| {
            highlight.modifiers & MODIFIER_DEBUG != 0
                && &source[highlight.span.start..highlight.span.end] == "print"
        }));
        assert!(contains(
            source,
            &first,
            "scan",
            SemanticTokenKind::Method,
            MODIFIER_DEFAULT_LIBRARY
        ));
    }

    #[test]
    fn syntax_recovery_keeps_lexical_highlighting_available() {
        let source = "// still editing\nstate \"game.exe\" {";
        let mut database = CompilerDatabase::new(source);
        let index = database
            .semantic_highlights()
            .expect("recoverable syntax errors should retain highlighting");
        assert!(contains(
            source,
            &index,
            "// still editing",
            SemanticTokenKind::Comment,
            0
        ));
        assert!(contains(
            source,
            &index,
            "state",
            SemanticTokenKind::Keyword,
            0
        ));
    }
}
