use std::collections::HashMap;

mod declarations;
mod expressions;
mod recovery;
mod statements;
mod types;

use crate::{
    PrimitiveType as CoreTypeId, Token, TokenCursor, TokenKind,
    ast::{
        Action, ActionKind, ArrayTypeDecl, ArrayTypeId, AssignmentId, BinaryOp, Block,
        ConstructedTypeIdAllocator, EnumDecl, EnumId, EnumReference, EnumVariant, EnumVariantId,
        Expr, ExprId, ExprKind, FallbackBranch, ForBinding, FunctionDecl, FunctionId,
        InterpolatedPart, MatchArm, MatchPattern, OptionTypeDecl, OptionTypeId, Parameter,
        PatternBinding, PatternId, PointerPath, Program, RecordDecl, RecordField, RecordFieldId,
        RecordId, ResultTypeDecl, ResultTypeId, SettingChoiceOption, SettingChoiceOptionId,
        SettingDecl, SettingFileFilter, SettingKind, Span, StateDecl, StateField, StateLayoutDecl,
        StateMemoryDecoder, StateProviderRef, StateSource, Stmt, SuspensionBinding, SuspensionMode,
        TypeNameId, TypeRef, UnaryOp, ValueId, VariableDecl,
    },
    diagnostic::Diagnostic,
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
        type_names: Vec::new(),
        type_name_spans: Vec::new(),
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
    type_names: Vec<String>,
    type_name_spans: Vec<Span>,
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
    fn type_can_be_stored_in_state(ty: TypeRef) -> bool {
        // Nominal value-usage rules are resolution/type-checking facts. The
        // parser only rejects the syntactically special non-value type.
        ty != TypeRef::core(CoreTypeId::Void)
    }

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
                if program.state.is_some() {
                    Err(self.error("only one `state(...)` declaration is allowed"))
                } else {
                    let declaration = if matches!(self.peek(1).kind, TokenKind::LParen) {
                        self.state_decl()
                    } else {
                        self.state_block_decl()
                    };
                    declaration.map(|declaration| program.state = Some(declaration))
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
                    declarations.map(|declarations| {
                        program.settings_span = Some(Span {
                            start,
                            end: self.previous().span.end,
                        });
                        program.settings = declarations;
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
            } else if self.current_action_kind().is_some() {
                self.action_block()
                    .map(|action| program.actions.push(action))
            } else {
                Err(self.error(
                    "expected `state`, `settings`, `record`, `enum`, `fn`, a global `let`, or an action block",
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
            self.diagnostics.push(Diagnostic::new(
                "a SplitScript file needs a `state(process, { ... })` declaration",
                Span::default(),
            ));
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Missing,
                span: Span::default(),
            });
        }
        program.array_types = self.array_types;
        program.option_types = self.option_types;
        program.result_types = self.result_types;
        program.type_names = self.type_names;
        program.type_name_spans = self.type_name_spans;
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

fn csharp_numeric_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "sbyte" => "i8",
        "byte" => "u8",
        "short" => "i16",
        "ushort" => "u16",
        "int" => "i32",
        "uint" => "u32",
        "long" => "i64",
        "ulong" => "u64",
        "float" => "f32",
        "double" => "f64",
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
        .map_or((digits.as_str(), 10), |digits| (digits, 16));
    let value = u64::from_str_radix(digits, radix)
        .map_err(|_| "integer literal does not fit in 64 bits".to_owned())?;
    Ok((value, suffix))
}

#[cfg(test)]
mod tests {
    use crate::{SyntaxMode, lex};

    use super::*;

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
            Some(TypeRef::core(CoreTypeId::U32))
        );
        assert_eq!(program.actions[0].kind, ActionKind::Split);
    }

    #[test]
    fn parses_named_state_layouts_and_their_generated_enum() {
        let source = r#"
            state "game.exe" {
                /// Steam build.
                layout Steam { level: u32 at 0x100 }
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
    fn parses_bounded_utf8_state_decoder_without_a_pseudo_type() {
        let source = r#"
            state "game.exe" {
                mapName at "game.dll", 0x1234, 0x20 as utf8(64)
            }
        "#;
        let program = parse(source, lex(source, SyntaxMode::Program).unwrap()).unwrap();
        let field = &program.state.unwrap().fields[0];
        assert_eq!(field.annotation, None);
        let StateSource::Pointer(path) = &field.source else {
            panic!("expected a pointer-backed state field");
        };
        assert_eq!(path.module.as_deref(), Some("game.dll"));
        assert_eq!(path.offsets, [0x1234, 0x20]);
        assert!(matches!(
            path.decoder,
            Some(StateMemoryDecoder::Utf8 { max_bytes: 64, .. })
        ));
    }

    #[test]
    fn array_types_use_brackets_and_compose_with_wrappers() {
        let source = r#"
            state "game.exe" {}
            record Arrays {
                bytes: [u8]
                nested: [[String]]
                optional: [i32]?
                fallibleElements: [u16!]
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
            TypeRef::core(CoreTypeId::U8)
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
        assert_eq!(fixed_bytes.element, TypeRef::core(CoreTypeId::U8));
        assert_eq!(fixed_bytes.length, Some(6));
    }

    #[test]
    fn legacy_array_constructor_syntax_is_not_accepted() {
        let source = "state \"game.exe\" {} record Legacy { values: Array<i32> }";
        assert!(parse(source, lex(source, SyntaxMode::Program).unwrap()).is_err());
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
