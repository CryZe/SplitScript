use std::collections::HashMap;

mod declarations;
mod expressions;
mod recovery;
mod statements;
mod types;

use crate::{
    Token, TokenCursor, TokenKind,
    ast::{
        Action, ActionKind, ArrayTypeDecl, ArrayTypeId, AssignmentId, AsyncTypeDecl, AsyncTypeId,
        BinaryOp, Block, ConstructedTypeIdAllocator, EnumDecl, EnumId, EnumReference, EnumVariant,
        EnumVariantId, Expr, ExprId, ExprKind, FallbackBranch, ForBinding, FunctionDecl,
        FunctionId, InterpolatedPart, MatchArm, MatchPattern, OptionTypeDecl, OptionTypeId,
        Parameter, PatternBinding, PatternId, PointerPath, PointerPathBase, Program, RecordDecl,
        RecordField, RecordFieldId, RecordId, ResultTypeDecl, ResultTypeId, SettingChoiceOption,
        SettingChoiceOptionId, SettingDecl, SettingExternalKey, SettingFamilyDecl,
        SettingFileFilter, SettingKind, SettingTextPart, SettingTextPattern, Span, StateDecl,
        StateField, StateLayoutDecl, StateMemoryDecoder, StateProviderRef, StateSource,
        StateTransform, Stmt, SuspensionMode, TickRateDecl, TickRateValue, TypeApplicationDecl,
        TypeApplicationId, TypeApplicationOccurrence, TypeNameId, TypeRef, UnaryOp, ValueId,
        VariableDecl,
    },
    diagnostic::Diagnostic,
    migration::{ASL_TIMER_CONTROL_DIAGNOSTIC, DUPLICATE_STATE_DIAGNOSTIC},
    source::{RecoveryNode, RecoveryNodeKind},
};

pub fn parse(source: &str, tokens: Vec<Token>) -> Result<Program, Diagnostic> {
    let output = parse_recovering(source, tokens);
    match output.diagnostics.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(output.program),
    }
}

pub struct ParseOutput {
    pub program: Program,
    /// Grammar and recovery diagnostics produced while constructing syntax.
    pub diagnostics: Vec<Diagnostic>,
    pub recovery_nodes: Vec<RecoveryNode>,
}

pub fn parse_recovering(source: &str, tokens: Vec<Token>) -> ParseOutput {
    Parser {
        source,
        cursor: TokenCursor::new(tokens),
        array_types: Vec::new(),
        array_type_ids: HashMap::new(),
        option_types: Vec::new(),
        option_type_ids: HashMap::new(),
        result_types: Vec::new(),
        result_type_ids: HashMap::new(),
        async_types: Vec::new(),
        async_type_ids: HashMap::new(),
        type_applications: Vec::new(),
        type_application_ids: HashMap::new(),
        type_names: Vec::new(),
        type_name_spans: Vec::new(),
        type_name_occurrences: Vec::new(),
        type_name_ids: HashMap::new(),
        constructed_type_ids: ConstructedTypeIdAllocator::starting_at(0),
        next_expression_id: 0,
        next_function_id: 0,
        next_record_id: 0,
        next_enum_id: 0,
        next_value_id: 0,
        next_assignment_id: 0,
        next_record_field_id: 0,
        next_enum_variant_id: 0,
        next_pattern_id: 0,
        next_setting_choice_option_id: 0,
        diagnostics: Vec::new(),
        recovery_nodes: Vec::new(),
    }
    .program()
}

struct Parser<'a> {
    source: &'a str,
    cursor: TokenCursor,
    array_types: Vec<ArrayTypeDecl>,
    array_type_ids: HashMap<(TypeRef, Option<u32>), ArrayTypeId>,
    option_types: Vec<OptionTypeDecl>,
    option_type_ids: HashMap<TypeRef, OptionTypeId>,
    result_types: Vec<ResultTypeDecl>,
    result_type_ids: HashMap<TypeRef, ResultTypeId>,
    async_types: Vec<AsyncTypeDecl>,
    async_type_ids: HashMap<TypeRef, AsyncTypeId>,
    type_applications: Vec<TypeApplicationDecl>,
    type_application_ids: HashMap<(TypeNameId, Vec<TypeRef>), TypeApplicationId>,
    type_names: Vec<String>,
    type_name_spans: Vec<Span>,
    type_name_occurrences: Vec<Vec<Span>>,
    type_name_ids: HashMap<String, TypeNameId>,
    constructed_type_ids: ConstructedTypeIdAllocator,
    next_expression_id: u32,
    next_function_id: u32,
    next_record_id: u32,
    next_enum_id: u32,
    next_value_id: u32,
    next_assignment_id: u32,
    next_record_field_id: u32,
    next_enum_variant_id: u32,
    next_pattern_id: u32,
    next_setting_choice_option_id: u32,
    diagnostics: Vec<Diagnostic>,
    recovery_nodes: Vec<RecoveryNode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DelimiterDepth {
    parentheses: u32,
    brackets: u32,
    braces: u32,
}

impl DelimiterDepth {
    fn update(&mut self, kind: &TokenKind) {
        match kind {
            TokenKind::LParen => self.parentheses += 1,
            TokenKind::RParen => self.parentheses = self.parentheses.saturating_sub(1),
            TokenKind::LBracket => self.brackets += 1,
            TokenKind::RBracket => self.brackets = self.brackets.saturating_sub(1),
            TokenKind::LBrace => self.braces += 1,
            TokenKind::RBrace => self.braces = self.braces.saturating_sub(1),
            _ => {}
        }
    }
}

impl Parser<'_> {
    fn new_expr(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = ExprId::from_index(self.next_expression_id);
        self.next_expression_id += 1;
        Expr::parsed(id, kind, span)
    }

    fn new_value_id(&mut self) -> ValueId {
        let id = ValueId::from_index(self.next_value_id);
        self.next_value_id += 1;
        id
    }

    fn new_assignment_id(&mut self) -> AssignmentId {
        let id = AssignmentId::from_index(self.next_assignment_id);
        self.next_assignment_id += 1;
        id
    }

    fn new_record_field_id(&mut self) -> RecordFieldId {
        let id = RecordFieldId::from_index(self.next_record_field_id);
        self.next_record_field_id += 1;
        id
    }

    fn new_enum_variant_id(&mut self) -> EnumVariantId {
        let id = EnumVariantId::from_index(self.next_enum_variant_id);
        self.next_enum_variant_id += 1;
        id
    }

    fn new_pattern_id(&mut self) -> PatternId {
        let id = PatternId::from_index(self.next_pattern_id);
        self.next_pattern_id += 1;
        id
    }

    fn new_setting_choice_option_id(&mut self) -> SettingChoiceOptionId {
        let id = SettingChoiceOptionId::from_index(self.next_setting_choice_option_id);
        self.next_setting_choice_option_id += 1;
        id
    }

    fn program(mut self) -> ParseOutput {
        let mut program = Program::default();
        while !self.at(&TokenKind::Eof) {
            let declaration_start = self.cursor.position();
            let mut documentation = self.take_source_documentation();
            if documentation.is_some() && self.at(&TokenKind::Eof) {
                self.diagnostics.push(
                    self.error("a documentation comment must precede a documented declaration"),
                );
                break;
            }
            let supports_documentation = self.at_ident("let")
                || self.at_ident("const")
                || self.at_ident("var")
                || self.at_ident("fn")
                || self.at_ident("func")
                || self.at_ident("function")
                || self.at_ident("record")
                || self.at_ident("enum")
                || (self.at_ident("debug")
                    && matches!(&self.peek(1).kind, TokenKind::Ident(name)
                        if matches!(name.as_str(), "let" | "const" | "var" | "fn" | "func" | "function")));
            if documentation.is_some() && !supports_documentation {
                self.diagnostics.push(self.error(
                    "documentation comments are supported on functions, global variables, records, and enums",
                ));
                documentation = None;
            }
            let recognized_start = self.is_top_level_start();
            let result = if self.at_ident("state") {
                if let Some(previous) = program.state.as_ref() {
                    Err(self
                        .migration_diagnostic(DUPLICATE_STATE_DIAGNOSTIC, self.current().span)
                        .with_secondary_label(previous.span, "the first state declaration is here"))
                } else {
                    let declaration = if matches!(self.peek(1).kind, TokenKind::LParen) {
                        self.state_decl()
                    } else {
                        self.state_block_decl()
                    };
                    declaration.map(|declaration| program.state = Some(declaration))
                }
            } else if self.at_ident("tickRate") {
                if program.tick_rate.is_some() {
                    Err(self.error("only one `tickRate` declaration is allowed"))
                } else {
                    self.tick_rate_decl()
                        .map(|declaration| program.tick_rate = Some(declaration))
                }
            } else if self.at_ident("settings") {
                if program.settings_span.is_some() {
                    Err(self.error("only one `settings` declaration is allowed"))
                } else if matches!(self.peek(1).kind, TokenKind::LParen) {
                    Err(self.error(
                        "legacy `settings({ ... })` syntax is not supported; use `settings { ... }`",
                    ))
                } else {
                    let start = self.current().span.start;
                    let declarations = self.settings_block_decl();
                    declarations.map(|(settings, families)| {
                        program.settings_span = Some(Span {
                            start,
                            end: self.previous().span.end,
                        });
                        program.settings = settings;
                        program.setting_families = families;
                    })
                }
            } else if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
                if self.at_ident("const") || self.at_ident("var") {
                    self.record_let_keyword_diagnostic();
                }
                self.variable_decl().and_then(|mut declaration| {
                    declaration.documentation = documentation.take();
                    self.terminator()?;
                    program.globals.push(declaration);
                    Ok(())
                })
            } else if self.at_ident("debug") {
                let start = self.bump().span.start;
                if self.at_ident("fn") || self.at_ident("func") || self.at_ident("function") {
                    if self.at_ident("func") || self.at_ident("function") {
                        self.record_fn_keyword_diagnostic();
                    }
                    self.function_decl().map(|mut function| {
                        function.documentation = documentation.take();
                        function.debug_only = true;
                        function.span.start = start;
                        program.functions.push(function);
                    })
                } else if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
                    if self.at_ident("const") || self.at_ident("var") {
                        self.record_let_keyword_diagnostic();
                    }
                    self.variable_decl().and_then(|mut declaration| {
                        declaration.documentation = documentation.take();
                        declaration.debug_only = true;
                        declaration.span.start = start;
                        self.terminator()?;
                        program.globals.push(declaration);
                        Ok(())
                    })
                } else {
                    Err(self.error("`debug` can only modify top-level `fn` or `let` declarations"))
                }
            } else if self.at_ident("fn") || self.at_ident("func") || self.at_ident("function") {
                if self.at_ident("func") || self.at_ident("function") {
                    self.record_fn_keyword_diagnostic();
                }
                self.function_decl().map(|mut function| {
                    function.documentation = documentation.take();
                    program.functions.push(function);
                })
            } else if self.at_ident("record") {
                self.record_decl().map(|mut record| {
                    record.documentation = documentation.take();
                    program.records.push(record);
                })
            } else if self.at_ident("enum") {
                self.enum_decl().map(|mut enumeration| {
                    enumeration.documentation = documentation.take();
                    program.enums.push(enumeration);
                })
            } else if self.current_action_kind().is_some()
                || self.current_legacy_lifecycle_diagnostic().is_some()
            {
                self.action_block()
                    .map(|action| program.actions.push(action))
            } else {
                Err(self.error(
                    "expected `state`, `tickRate`, `settings`, `record`, `enum`, `fn`, a global `let`, or an action block",
                ))
            };

            if let Err(error) = result {
                if recognized_start && error.message.starts_with("expected") {
                    self.recovery_nodes.push(RecoveryNode {
                        kind: RecoveryNodeKind::Missing,
                        span: Span {
                            start: error.span.start,
                            end: error.span.start,
                        },
                    });
                }
                self.diagnostics.push(error);
                let skipped_start = self.cursor.tokens()[declaration_start].span.start;
                self.synchronize_top_level(declaration_start);
                let skipped_end = self.current().span.start.max(skipped_start);
                if skipped_end != skipped_start {
                    self.recovery_nodes.push(RecoveryNode {
                        kind: RecoveryNodeKind::Error,
                        span: Span {
                            start: skipped_start,
                            end: skipped_end,
                        },
                    });
                }
            }
        }
        if program.state.is_none() {
            self.diagnostics.push(
                Diagnostic::new(
                    "a SplitScript autosplitter needs one attachment `state` declaration",
                    Span::default(),
                )
                .with_primary_label("no attachment provider is declared")
                .with_note(
                    "use `state \"game.exe\" { ... }` for a native process or `state GBA { ... }` for a supported provider",
                )
                .with_note(
                    "SplitScript currently compiles one executable autosplitter per file; a state-less helper module is not a supported compilation unit",
                ),
            );
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Missing,
                span: Span::default(),
            });
        }
        program.array_types = self.array_types;
        program.option_types = self.option_types;
        program.result_types = self.result_types;
        program.async_types = self.async_types;
        program.type_applications = self.type_applications;
        program.type_names = self.type_names;
        program.type_name_spans = self.type_name_spans;
        program.type_name_occurrences = self.type_name_occurrences;
        ParseOutput {
            program,
            diagnostics: self.diagnostics,
            recovery_nodes: self.recovery_nodes,
        }
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Debug { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::StateAssign { span, .. }
        | Stmt::IndexAssign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Return { span, .. }
        | Stmt::Throw { span, .. }
        | Stmt::Suspend { span, .. } => *span,
        Stmt::Variable(variable) => variable.span,
        Stmt::Expression(expression) => expression.span,
    }
}

fn assignment_operator(token: &TokenKind) -> Option<Option<BinaryOp>> {
    Some(match token {
        TokenKind::Assign => None,
        TokenKind::PlusAssign => Some(BinaryOp::Add),
        TokenKind::MinusAssign => Some(BinaryOp::Sub),
        TokenKind::StarAssign => Some(BinaryOp::Mul),
        TokenKind::SlashAssign => Some(BinaryOp::Div),
        TokenKind::PercentAssign => Some(BinaryOp::Rem),
        TokenKind::OrAssign => Some(BinaryOp::BitOr),
        TokenKind::AndAssign => Some(BinaryOp::BitAnd),
        TokenKind::CaretAssign => Some(BinaryOp::BitXor),
        TokenKind::ShlAssign => Some(BinaryOp::Shl),
        TokenKind::ShrAssign => Some(BinaryOp::Shr),
        _ => return None,
    })
}

pub(crate) fn parse_integer(text: &str) -> Result<(u64, Option<TypeRef>), String> {
    const SUFFIXES: [&str; 8] = ["u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8"];
    let (digits, suffix) = SUFFIXES
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix).map(|digits| (digits, *suffix)))
        .map_or((text, None), |(digits, suffix)| (digits, Some(suffix)));
    let suffix = suffix.and_then(TypeRef::parse);
    let digits = digits.replace('_', "");
    let (digits, radix) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
        .map_or_else(
            || {
                digits
                    .strip_prefix("0b")
                    .or_else(|| digits.strip_prefix("0B"))
                    .map_or((digits.as_str(), 10), |digits| (digits, 2))
            },
            |digits| (digits, 16),
        );
    let value = u64::from_str_radix(digits, radix).map_err(|error| match error.kind() {
        std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow => {
            "integer literal does not fit in 64 bits".to_owned()
        }
        std::num::IntErrorKind::Empty => "integer literal requires at least one digit".to_owned(),
        std::num::IntErrorKind::InvalidDigit => {
            let (invalid_index, invalid) = digits
                .char_indices()
                .find(|(_, digit)| digit.to_digit(radix).is_none())
                .expect("invalid integer literals contain an invalid digit");
            if invalid.is_ascii_digit() {
                let base = match radix {
                    2 => "binary",
                    16 => "hexadecimal",
                    _ => "decimal",
                };
                format!("digit `{invalid}` is not valid in a {base} integer literal")
            } else {
                format!("unknown integer type suffix `{}`", &digits[invalid_index..])
            }
        }
        _ => "invalid integer literal".to_owned(),
    })?;
    Ok((value, suffix))
}

#[cfg(test)]
mod tests {
    use crate::{PrimitiveType, SyntaxMode, lex};

    use super::*;

    #[test]
    fn parses_binary_integer_literals() {
        assert_eq!(parse_integer("0b1010"), Ok((10, None)));
        assert_eq!(parse_integer("0B1111_0000"), Ok((0xf0, None)));
        assert_eq!(
            parse_integer("0b1000u16"),
            Ok((8, Some(TypeRef::core(PrimitiveType::U16))))
        );
    }

    #[test]
    fn diagnoses_invalid_and_overflowing_integer_literals_accurately() {
        assert_eq!(
            parse_integer("0b102"),
            Err("digit `2` is not valid in a binary integer literal".to_owned())
        );
        assert_eq!(
            parse_integer("0b101usize"),
            Err("unknown integer type suffix `usize`".to_owned())
        );
        assert_eq!(
            parse_integer("18446744073709551616"),
            Err("integer literal does not fit in 64 bits".to_owned())
        );
        assert_eq!(
            parse_integer(
                "0b1_00000000_00000000_00000000_00000000_00000000_00000000_00000000_00000000"
            ),
            Err("integer literal does not fit in 64 bits".to_owned())
        );
    }

    #[test]
    fn parses_domain_shaped_autosplitter() {
        let source = r#"
            state "game.exe" {
                level: u32 at "game.exe", 0x1234, 0x20
            }
            settings { "Split levels" => splitLevels: true }
            split {
                let changed = current.level != old.level;
                return settings.splitLevels && changed;
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(
            program.state.unwrap().fields[0].annotation,
            Some(TypeRef::core(PrimitiveType::U32))
        );
        assert_eq!(program.actions[0].kind, ActionKind::Split);
    }

    #[test]
    fn parses_setup_as_a_lifecycle_block() {
        let source = r#"
            state "game.exe" {}
            setup { setTickRate(60.0) }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.actions[0].kind, ActionKind::Setup);
    }

    #[test]
    fn parses_declarative_tick_rate_overrides() {
        let source = r#"
            state "game.exe" {}
            tickRate {
                attached: 60,
                detached: 2.5,
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let policy = program.tick_rate.expect("tick-rate policy");
        assert_eq!(policy.attached.unwrap().value, 60.0);
        assert_eq!(policy.detached.unwrap().value, 2.5);
        assert_eq!(program.attached_tick_rate(), 60.0);
        assert_eq!(program.detached_tick_rate(), 2.5);
    }

    #[test]
    fn tick_rate_defaults_and_invalid_values_are_explicit() {
        let source = r#"state "game.exe" {}"#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.attached_tick_rate(), 120.0);
        assert_eq!(program.detached_tick_rate(), 1.0);

        for invalid in ["0", "-1", "1e309"] {
            let source = format!(
                r#"
                    state "game.exe" {{}}
                    tickRate {{ attached: {invalid} }}
                "#
            );
            let diagnostic = parse(&source, lex(&source, SyntaxMode::Program).unwrap())
                .expect_err("the invalid tick rate should be rejected");
            assert_eq!(
                diagnostic.message,
                "a tick rate must be finite and greater than zero"
            );
        }
    }

    #[test]
    fn parses_on_state_ready_as_a_lifecycle_block() {
        let source = r#"
            state "game.exe" { level: u32 at 0x100 }
            onStateReady { print(current.level) }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.actions[0].kind, ActionKind::OnStateReady);
    }

    #[test]
    fn parses_on_detach_as_a_lifecycle_block() {
        let source = r#"
            state "game.exe" {}
            onDetach { timer.pauseGameTime() }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.actions[0].kind, ActionKind::OnDetach);
    }

    #[test]
    fn parses_named_state_layouts_and_their_generated_enum() {
        let source = r#"
            state "game.exe" {
                /// Steam build.
                layout Steam { level: u32 at 0x100 },
                layout GOG { level: u32 at 0x200 }
            }
            onAttach { return StateLayout.Steam }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let state = program.state.unwrap();
        assert!(state.fields.is_empty());
        assert_eq!(state.layouts.len(), 2);
        assert_eq!(state.layouts[0].fields[0].name, "level");
        let enumeration = state.layout_enum.unwrap();
        assert_eq!(enumeration.name, "StateLayout");
        assert_eq!(enumeration.variants[0].name, "Steam");
        assert_eq!(
            enumeration.variants[0].documentation.as_deref(),
            Some("Steam build.")
        );
        assert_eq!(program.actions[0].kind, ActionKind::OnAttach);
    }

    #[test]
    fn parses_bounded_native_string_decoders_without_pseudo_types() {
        let source = r#"
            state "game.exe" {
                mapName at "game.dll", 0x1234, 0x20 as utf8(64);
                chapterName at 0x2345 as utf16le(32)
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let field = &program.state.as_ref().unwrap().fields[0];
        assert_eq!(field.annotation, None);
        let StateSource::Pointer(path) = &field.source else {
            panic!("expected a pointer-backed state field");
        };
        assert_eq!(
            path.base,
            PointerPathBase::Module {
                name: "game.dll".to_owned(),
                offset: 0x1234,
            }
        );
        assert_eq!(path.offsets, [0x20]);
        assert!(matches!(
            path.decoder,
            Some(StateMemoryDecoder::Utf8 { max_bytes: 64, .. })
        ));
        let wide_field = &program.state.as_ref().unwrap().fields[1];
        assert_eq!(wide_field.annotation, None);
        let StateSource::Pointer(wide_path) = &wide_field.source else {
            panic!("expected a pointer-backed UTF-16LE state field");
        };
        assert_eq!(wide_path.base, PointerPathBase::Absolute(0x2345));
        assert!(wide_path.offsets.is_empty());
        assert!(matches!(
            wide_path.decoder,
            Some(StateMemoryDecoder::Utf16Le { max_units: 32, .. })
        ));
    }

    #[test]
    fn pointer_paths_keep_unsigned_roots_and_signed_offsets_distinct() {
        let source = r#"
            state "game.exe" {
                high: i32 at 0xffff_ffff_ffff_fff0;
                module: i32 at "game.dll", -0x20, -0x8000_0000_0000_0000
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let fields = &program.state.as_ref().unwrap().fields;
        let StateSource::Pointer(high) = &fields[0].source else {
            panic!("expected an absolute pointer path");
        };
        assert_eq!(high.base, PointerPathBase::Absolute(0xffff_ffff_ffff_fff0));
        assert!(high.offsets.is_empty());
        let StateSource::Pointer(module) = &fields[1].source else {
            panic!("expected a module-relative pointer path");
        };
        assert_eq!(
            module.base,
            PointerPathBase::Module {
                name: "game.dll".to_owned(),
                offset: -0x20,
            }
        );
        assert_eq!(module.offsets, [i64::MIN]);

        for source in [
            "state \"game.exe\" { value: i32 at \"game.dll\", 0x8000_0000_0000_0000 }",
            "state \"game.exe\" { value: i32 at 0x1000, -0x8000_0000_0000_0001 }",
        ] {
            let error = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap_err();
            assert!(
                error.message.contains("must fit in signed 64 bits"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn array_types_use_brackets_and_compose_with_wrappers() {
        let source = r#"
            state "game.exe" {}
            record Arrays {
                bytes: [u8],
                nested: [[String]],
                optional: [i32]?,
                fallibleElements: [u16!],
                fixedBytes: [u8; 6]
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let fields = &program.records[0].fields;

        let TypeRef::Array(bytes) = fields[0].ty else {
            panic!("expected [u8]");
        };
        assert_eq!(
            program
                .array_types
                .iter()
                .find(|array| array.id == bytes)
                .unwrap()
                .element,
            TypeRef::core(PrimitiveType::U8)
        );

        let TypeRef::Array(nested) = fields[1].ty else {
            panic!("expected [[String]]");
        };
        assert!(matches!(
            program
                .array_types
                .iter()
                .find(|array| array.id == nested)
                .unwrap()
                .element,
            TypeRef::Array(_)
        ));
        assert!(matches!(fields[2].ty, TypeRef::Option(_)));

        let TypeRef::Array(fallible_elements) = fields[3].ty else {
            panic!("expected [u16!]");
        };
        assert!(matches!(
            program
                .array_types
                .iter()
                .find(|array| array.id == fallible_elements)
                .unwrap()
                .element,
            TypeRef::Result(_)
        ));
        let TypeRef::Array(fixed_bytes) = fields[4].ty else {
            panic!("expected [u8; 6]");
        };
        let fixed_bytes = program
            .array_types
            .iter()
            .find(|array| array.id == fixed_bytes)
            .unwrap();
        assert_eq!(fixed_bytes.element, TypeRef::core(PrimitiveType::U8));
        assert_eq!(fixed_bytes.length, Some(6));
    }

    #[test]
    fn parses_for_in_with_a_scoped_binding() {
        let source = r#"
            state "game.exe" {}
            whileAttached {
                for value in [1, 2, 3] {
                    print(value as String)
                }
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let Stmt::For {
            binding,
            iterable,
            body,
            ..
        } = &program.actions[0].body.statements[0]
        else {
            panic!("expected a for statement")
        };
        assert_eq!(binding.name, "value");
        assert!(matches!(iterable.kind, ExprKind::Array(_)));
        assert_eq!(body.statements.len(), 1);
    }

    #[test]
    fn postfix_calls_bind_more_tightly_than_unary_operators() {
        let source = r#"
            state "game.exe" {}
            whileAttached {
                let absent = !"short".contains("long")
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let Stmt::Variable(variable) = &program.actions[0].body.statements[0] else {
            panic!("expected a variable declaration")
        };
        let ExprKind::Unary {
            op: UnaryOp::Not,
            expr,
        } = &variable.value.kind
        else {
            panic!("expected unary negation outside the call")
        };
        assert!(matches!(expr.kind, ExprKind::Call { .. }));
    }

    #[test]
    fn array_indexes_are_postfix_expressions_and_can_be_chained() {
        let source = r#"
            state "game.exe" {}
            whileAttached {
                let value = matrix[outer + 1][inner]
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let Stmt::Variable(variable) = &program.actions[0].body.statements[0] else {
            panic!("expected a variable declaration")
        };
        let ExprKind::Index {
            receiver, index, ..
        } = &variable.value.kind
        else {
            panic!("expected the second postfix index")
        };
        assert!(matches!(index.kind, ExprKind::Path(_)));
        let ExprKind::Index {
            receiver: matrix,
            index: outer,
            ..
        } = &receiver.kind
        else {
            panic!("expected the first postfix index")
        };
        assert!(matches!(matrix.kind, ExprKind::Path(_)));
        assert!(matches!(outer.kind, ExprKind::Binary { .. }));
    }

    #[test]
    fn builds_multiline_setting_tooltips_from_doc_comments() {
        let source = r#"
            state "game.exe" {}
            settings {
                /// First line of the tooltip
                /// continues on this line.
                ///
                /// A second paragraph.
                "Enabled" => enabled: true
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(
            program.settings[0].tooltip.as_deref(),
            Some("First line of the tooltip continues on this line.\nA second paragraph.")
        );
    }

    #[test]
    fn parses_stable_string_setting_keys() {
        let source = r#"
            state "game.exe" {}
            settings {
                "Mission" => mission key "42": true,
                "Boss" => boss key "final-boss": false,
                "Ordinary" => ordinary: true
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert!(matches!(
            &program.settings[0].external_key,
            Some(SettingExternalKey { value, .. }) if value == "42"
        ));
        assert!(matches!(
            &program.settings[1].external_key,
            Some(SettingExternalKey { value, .. }) if value == "final-boss"
        ));
        assert!(program.settings[2].external_key.is_none());
        assert_eq!(program.settings[0].runtime_key(), "42");
        assert_eq!(program.settings[1].runtime_key(), "final-boss");
        assert_eq!(program.settings[2].runtime_key(), "ordinary");
    }

    #[test]
    fn expands_compile_time_boolean_setting_families() {
        let source = r#"
            state "game.exe" {}
            settings {
                "Levels" {
                    /// Controls this level split.
                    for level in 2..=4 {
                        `Level {level}` key `{level}`: true
                    },
                },
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.setting_families.len(), 1);
        let family = &program.setting_families[0];
        assert_eq!(family.binding, "level");
        assert_eq!((family.start, family.end_inclusive), (2, 4));
        let concrete = program
            .settings
            .iter()
            .filter(|setting| !setting.source_visible)
            .collect::<Vec<_>>();
        assert_eq!(concrete.len(), 3);
        assert_eq!(
            concrete
                .iter()
                .map(|setting| (setting.description.as_str(), setting.runtime_key()))
                .collect::<Vec<_>>(),
            [("Level 2", "2"), ("Level 3", "3"), ("Level 4", "4")]
        );
        assert!(concrete.iter().all(|setting| {
            setting.tooltip.as_deref() == Some("Controls this level split.")
                && matches!(setting.kind, SettingKind::Bool { default: true })
        }));
    }

    #[test]
    fn declaration_lists_are_comma_delimited_and_accept_trailing_commas() {
        let valid = r#"
            enum Mode { First, Second, }
            record Pair { left: i32, right: i32, }
            settings {
                "Mode" => mode: choice {
                    "First" => Mode.First,
                    "Second" => Mode.Second default,
                },
                "Input" => input: file {
                    "Text" => "*.txt",
                    mime => "text/plain",
                },
            }
            state "game.exe" {
                left: i32 at 0x100;
                right: i32 at 0x104;
            }
            fn pair(left, right,) { print(left + right) }
            whileAttached {
                print(min(1, 2,))
                process.read<i32,>(0x100,)
                let values = [1, 2,]
                let pair = Pair { left: 1, right: 2, }
                print(match mode { Mode.First => 1, Mode.Second => 2, })
            }
        "#;
        parse(valid, lex(valid, SyntaxMode::Program).unwrap())
            .expect("every comma-separated construct should accept a trailing comma");

        for source in [
            "record Pair { left: i32 right: i32 }",
            "record Pair { left: i32\nright: i32 }",
            "enum Mode { First Second }",
            "enum Mode { First\nSecond }",
            "state \"game.exe\" { left: i32 at 0x100\nright: i32 at 0x104 }",
            "state \"game.exe\" {} settings { \"A\" => a: true\n\"B\" => b: true }",
        ] {
            let error = parse(source, lex(source, SyntaxMode::Program).unwrap())
                .expect_err("a line break must not substitute for a comma");
            assert!(
                error.message.starts_with("expected `,` between ")
                    || error.message == "expected `;` between state fields"
            );
        }
    }

    #[test]
    fn interns_generic_type_applications_but_retains_every_source_occurrence() {
        let source = r#"
            state "game.exe" {}
            fn visit(first: Set<String>, second: Set<String,>) {}
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(program.type_applications.len(), 1);
        let application = &program.type_applications[0];
        assert_eq!(program.type_name(application.constructor), "Set");
        assert_eq!(application.arguments.len(), 1);
        assert_eq!(application.occurrences.len(), 2);
        for occurrence in &application.occurrences {
            assert_eq!(
                &source[occurrence.opening.start..occurrence.opening.end],
                "<"
            );
            assert_eq!(
                &source[occurrence.closing.start..occurrence.closing.end],
                ">"
            );
        }
    }

    #[test]
    fn parses_adjacent_nested_generic_closers_and_assignments() {
        let source = r#"
            state "game.exe" {}
            fn example() {
                let first: Set<String>=Set.new<String>()
                let nested: Set<Set<String>>=Set.new<Set<String>>()
                let deep: Set<Set<Set<String>>>=Set.new<Set<Set<String>>>()
                let optional: Set<String>? = None
                let fallible: Set<String>! = unavailable()
                make<Set<String>>()
                make < Set<Set<String>> > ()
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap())
            .expect("generic closers must not require whitespace before `=` or another `>`");

        let nested = program
            .type_applications
            .iter()
            .find(|application| {
                application.arguments.iter().any(|argument| {
                    matches!(
                        argument,
                        TypeRef::Application(inner)
                            if program.type_name(program.type_applications[inner.index()].constructor)
                                == "Set"
                    )
                })
            })
            .expect("nested Set application should be retained");
        let occurrence = nested
            .occurrences
            .first()
            .expect("nested application should retain its source occurrence");
        assert_eq!(
            &source[occurrence.closing.start..occurrence.closing.end],
            ">"
        );
        assert_eq!(
            source.as_bytes()[occurrence.closing.start - 1],
            b'>',
            "the inner and outer closers should occupy distinct bytes of `>>`"
        );
        assert_eq!(
            source.as_bytes()[occurrence.closing.end],
            b'=',
            "the assignment should remain after the outer generic closer"
        );
    }

    #[test]
    fn keeps_comparison_and_shift_operators_maximally_munched_in_expressions() {
        let source = r#"
            state "game.exe" {}
            fn operators(left, right) {
                let compared = left >= right
                let less = left < right
                let shifted = left >> right
                left >>= right
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let statements = &program.functions[0].body.statements;
        let Stmt::Variable(compared) = &statements[0] else {
            panic!("expected comparison variable")
        };
        assert!(matches!(
            compared.value.kind,
            ExprKind::Binary {
                op: BinaryOp::Ge,
                ..
            }
        ));
        let Stmt::Variable(less) = &statements[1] else {
            panic!("expected less-than variable")
        };
        assert!(matches!(
            less.value.kind,
            ExprKind::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
        let Stmt::Variable(shifted) = &statements[2] else {
            panic!("expected shift variable")
        };
        assert!(matches!(
            shifted.value.kind,
            ExprKind::Binary {
                op: BinaryOp::Shr,
                ..
            }
        ));
        assert!(matches!(
            statements[3],
            Stmt::Assign {
                op: Some(BinaryOp::Shr),
                ..
            }
        ));
    }

    #[test]
    fn generic_cast_targets_end_before_equality_expressions() {
        for source in [
            "state \"game.exe\" {} fn compare(expr, foo) { return expr as List<u32> == foo }",
            "state \"game.exe\" {} fn compare(expr, foo) { return expr as List<u32>==foo }",
            "state \"game.exe\" {} fn compare(expr, foo) { return expr as List<List<u32>>==foo }",
        ] {
            let program = parse(source, lex(source, SyntaxMode::Program).unwrap())
                .expect("a generic cast target should leave the following equality operator");
            let Stmt::Return {
                value: Some(value), ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("expected a returned expression")
            };
            assert!(matches!(
                value.kind,
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    ref left,
                    ..
                } if matches!(left.kind, ExprKind::Cast { .. })
            ));
        }
    }

    #[test]
    fn bang_eq_after_a_cast_is_inequality_not_a_result_postfix() {
        let source = r#"
            state "game.exe" {}
            fn compare(expr, foo) {
                let ordinary = expr as T!=foo
                let fallible = expr as T! == foo
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let Stmt::Variable(ordinary) = &program.functions[0].body.statements[0] else {
            panic!("expected an ordinary comparison")
        };
        assert!(matches!(
            ordinary.value.kind,
            ExprKind::Binary {
                op: BinaryOp::Ne,
                ref left,
                ..
            } if matches!(left.kind, ExprKind::Cast { target: TypeRef::Named(_), .. })
        ));

        let Stmt::Variable(fallible) = &program.functions[0].body.statements[1] else {
            panic!("expected a fallible cast comparison")
        };
        assert!(matches!(
            fallible.value.kind,
            ExprKind::Binary {
                op: BinaryOp::Eq,
                ref left,
                ..
            } if matches!(left.kind, ExprKind::Cast { target: TypeRef::Result(_), .. })
        ));
    }

    #[test]
    fn mixed_cast_type_wrappers_compose_and_propagation_requires_parentheses() {
        for (expression, outer_is_option) in [("value as T!?", true), ("value as T?!", false)] {
            let source =
                format!("state \"game.exe\" {{}} fn propagate(value) {{ return {expression} }}");
            let program = parse(&source, lex(&source, SyntaxMode::Program).unwrap())
                .unwrap_or_else(|diagnostic| {
                    panic!("failed to parse `{expression}`: {diagnostic:?}")
                });
            let Stmt::Return {
                value: Some(value), ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("expected a returned cast")
            };
            let ExprKind::Cast { target, .. } = value.kind else {
                panic!("`{expression}` should remain one cast expression")
            };
            let nested = match (target, outer_is_option) {
                (TypeRef::Option(id), true) => {
                    program
                        .option_types
                        .iter()
                        .find(|option| option.id == id)
                        .expect("outer option type")
                        .value
                }
                (TypeRef::Result(id), false) => {
                    program
                        .result_types
                        .iter()
                        .find(|result| result.id == id)
                        .expect("outer result type")
                        .value
                }
                _ => panic!("`{expression}` has the wrong outer wrapper"),
            };
            assert!(matches!(
                (nested, outer_is_option),
                (TypeRef::Result(_), true) | (TypeRef::Option(_), false)
            ));
        }

        for expression in ["value as T??", "value as T!!"] {
            let source =
                format!("state \"game.exe\" {{}} fn invalid(value) {{ return {expression} }}");
            let error = parse(&source, lex(&source, SyntaxMode::Program).unwrap())
                .expect_err("identical adjacent wrappers must be rejected");
            assert!(error.message.contains("two adjacent"));
        }

        for (expression, expected_target) in [
            ("(value as T?)?", "option"),
            ("(value as T!)?", "result"),
            ("(value as T)?", "plain"),
        ] {
            let source =
                format!("state \"game.exe\" {{}} fn propagate(value) {{ return {expression} }}");
            let program = parse(&source, lex(&source, SyntaxMode::Program).unwrap())
                .unwrap_or_else(|diagnostic| {
                    panic!("failed to parse `{expression}`: {diagnostic:?}")
                });
            let Stmt::Return {
                value: Some(value), ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("expected a propagated cast")
            };
            let ExprKind::Propagate(cast) = &value.kind else {
                panic!("`{expression}` should apply postfix `?` to the cast")
            };
            assert!(matches!(
                (&cast.kind, expected_target),
                (
                    ExprKind::Cast {
                        target: TypeRef::Option(_),
                        ..
                    },
                    "option"
                ) | (
                    ExprKind::Cast {
                        target: TypeRef::Result(_),
                        ..
                    },
                    "result"
                ) | (
                    ExprKind::Cast {
                        target: TypeRef::Named(_),
                        ..
                    },
                    "plain"
                )
            ));
        }

        for annotation in ["T??", "T!!"] {
            let declaration = format!(
                "state \"game.exe\" {{}} fn invalid() {{ let value: {annotation} = None }}"
            );
            let error = parse(
                &declaration,
                lex(&declaration, SyntaxMode::Program).unwrap(),
            )
            .expect_err("declarations must still reject duplicate type wrappers");
            assert!(error.message.contains("two adjacent"));
        }
    }

    #[test]
    fn cast_type_boundaries_preserve_the_following_binary_operator() {
        let cases = [
            ("value as T==other", BinaryOp::Eq),
            ("value as T>other", BinaryOp::Gt),
            ("value as T>=other", BinaryOp::Ge),
            ("value as T>>other", BinaryOp::Shr),
            ("value as T<<other", BinaryOp::Shl),
            ("value as T<=other", BinaryOp::Le),
            ("value as Box<u32>==other", BinaryOp::Eq),
            ("value as Box<u32>!=other", BinaryOp::Ne),
            ("value as Box<u32>>other", BinaryOp::Gt),
            ("value as Box<u32>>=other", BinaryOp::Ge),
            ("value as Box<u32>>>other", BinaryOp::Shr),
            ("value as Box<u32><other", BinaryOp::Lt),
            ("value as Box<u32><=other", BinaryOp::Le),
            ("value as Box<u32><<other", BinaryOp::Shl),
            ("value as Box<Box<u32>>==other", BinaryOp::Eq),
            ("value as Box<Box<u32>>>other", BinaryOp::Gt),
            ("value as Box<Box<u32>>>>other", BinaryOp::Shr),
            ("value as T!=other", BinaryOp::Ne),
            ("value as T! == other", BinaryOp::Eq),
            ("value as T!!=other", BinaryOp::Ne),
            ("value as T?==other", BinaryOp::Eq),
            ("value as Box<u32>?==other", BinaryOp::Eq),
            ("value as Box<u32>!!=other", BinaryOp::Ne),
            ("value as [u32; 4]==other", BinaryOp::Eq),
        ];

        for (expression, expected) in cases {
            let source = format!(
                "state \"game.exe\" {{}} fn compare(value, other) {{ return {expression} }}"
            );
            let program = parse(&source, lex(&source, SyntaxMode::Program).unwrap())
                .unwrap_or_else(|diagnostic| {
                    panic!("failed to parse `{expression}`: {diagnostic:?}")
                });
            let Stmt::Return {
                value: Some(value), ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("expected `{expression}` to remain a returned binary expression")
            };
            assert!(
                matches!(value.kind, ExprKind::Binary { op, .. } if op == expected),
                "`{expression}` did not retain {expected:?}: {:?}",
                value.kind
            );
        }
    }

    #[test]
    fn less_than_after_a_named_cast_requires_parentheses() {
        let ambiguous =
            "state \"game.exe\" {} fn compare(value, other) { return value as T < other }";
        let error = parse(ambiguous, lex(ambiguous, SyntaxMode::Program).unwrap())
            .expect_err("a bare `<` begins generic type arguments after a named cast");
        assert_eq!(error.message, "expected `>` after generic type arguments");

        let explicit =
            "state \"game.exe\" {} fn compare(value, other) { return (value as T) < other }";
        let program = parse(explicit, lex(explicit, SyntaxMode::Program).unwrap()).unwrap();
        let Stmt::Return {
            value: Some(value), ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("expected a parenthesized cast comparison")
        };
        assert!(matches!(
            value.kind,
            ExprKind::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
    }

    #[test]
    fn strict_inequality_after_a_cast_remains_migration_syntax() {
        for (expression, operator, replacement) in [
            ("value as T!==other", "!==", "!=`"),
            ("value as Box<T>===other", "===", "==`"),
            ("value as Box<Box<T>>===other", "===", "==`"),
        ] {
            let source = format!(
                "state \"game.exe\" {{}} fn compare(value, other) {{ return {expression} }}"
            );
            let recovered = parse_recovering(&source, lex(&source, SyntaxMode::Program).unwrap());
            assert_eq!(recovered.diagnostics.len(), 1, "{expression}");
            let diagnostic = &recovered.diagnostics[0];
            assert_eq!(
                &source[diagnostic.span.start..diagnostic.span.end],
                operator
            );
            assert!(diagnostic.message.contains(replacement));
        }
    }

    #[test]
    fn state_fields_use_semicolons_because_pointer_paths_use_commas() {
        let valid = r#"
            state "game.exe" {
                map at "game.exe", 0x100, 0x20;
                level: u32 at 0x200
            }
        "#;
        parse(valid, lex(valid, SyntaxMode::Program).unwrap())
            .expect("the final state-field semicolon should be optional");

        let invalid = "state \"game.exe\" { first at 0x100, second at 0x200 }";
        let error = parse(invalid, lex(invalid, SyntaxMode::Program).unwrap())
            .expect_err("a comma must not separate state fields");
        assert_eq!(error.message, "expected `;` between state fields");
        assert_eq!(error.fixes[0].title, "replace `,` with `;`");
        assert_eq!(error.fixes[0].edits[0].replacement, ";");
    }

    #[test]
    fn rejects_non_string_setting_keys() {
        let source = r#"
            state "game.exe" {}
            settings {
                "Mission" => mission key 42: true
            }
        "#;
        let error = parse(source, lex(source, SyntaxMode::Program).unwrap())
            .expect_err("host settings-map keys are always strings");
        assert_eq!(error.message, "expected a string setting key");
    }

    #[test]
    fn attaches_multiline_documentation_to_source_declarations() {
        let source = r#"
            state "game.exe" {
                /// Current stage.
                stage: i32 at 0x100
            }
            /// Global first line.
            ///
            /// Global second paragraph.
            let total = 0
            /// A point in game memory.
            record Point {
                /// Horizontal position.
                x: i32
            }
            /// Current run mode.
            enum Mode {
                /// Active gameplay.
                Active
            }
            /// Describes the current point.
            fn describe(point: Point) -> String {
                return point.x as String
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        assert_eq!(
            program.globals[0].documentation.as_deref(),
            Some("Global first line.\n\nGlobal second paragraph.")
        );
        assert_eq!(
            program.state.as_ref().unwrap().fields[0]
                .documentation
                .as_deref(),
            Some("Current stage.")
        );
        assert_eq!(
            program.records[0].documentation.as_deref(),
            Some("A point in game memory.")
        );
        assert_eq!(
            program.records[0].fields[0].documentation.as_deref(),
            Some("Horizontal position.")
        );
        assert_eq!(
            program.enums[0].documentation.as_deref(),
            Some("Current run mode.")
        );
        assert_eq!(
            program.enums[0].variants[0].documentation.as_deref(),
            Some("Active gameplay.")
        );
        assert_eq!(
            program.functions[0].documentation.as_deref(),
            Some("Describes the current point.")
        );
    }
}
