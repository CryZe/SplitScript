use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{
        Action, ActionKind, ArrayTypeDecl, ArrayTypeId, AssignmentId, BinaryOp, Block, EnumDecl,
        EnumId, EnumTypeId, EnumVariant, EnumVariantId, Expr, ExprId, ExprKind, FallbackBranch,
        FunctionDecl, FunctionId, InterpolatedPart, MatchArm, MatchPattern, OptionTypeDecl,
        OptionTypeId, Parameter, PatternBinding, PatternId, PointerPath, Program, RecordDecl,
        RecordField, RecordFieldId, RecordId, ResultTypeDecl, ResultTypeId, SettingChoiceOption,
        SettingChoiceOptionId, SettingDecl, SettingFileFilter, SettingKind, Span, StateDecl,
        StateField, StateSource, Stmt, SuspensionBinding, SuspensionMode, TypeNameId, TypeRef,
        UnaryOp, ValueId, VariableDecl,
    },
    lexer::{Token, TokenKind},
    stdlib::{StandardLibrary, StdlibTypeKind},
    syntax::{RecoveryNode, RecoveryNodeKind},
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
    pub diagnostics: Vec<Diagnostic>,
    pub recovery_nodes: Vec<RecoveryNode>,
}

pub fn parse_recovering(source: &str, tokens: Vec<Token>) -> ParseOutput {
    let (named_types, initial_diagnostics) = collect_named_types(&tokens);
    let first_constructed_type_id = named_types.source_type_count() as u32;
    Parser {
        source,
        tokens,
        pos: 0,
        named_types,
        array_types: Vec::new(),
        array_type_ids: HashMap::new(),
        option_types: Vec::new(),
        option_type_ids: HashMap::new(),
        result_types: Vec::new(),
        result_type_ids: HashMap::new(),
        type_names: Vec::new(),
        type_name_ids: HashMap::new(),
        next_constructed_type_id: first_constructed_type_id,
        next_expression_id: 0,
        next_function_id: 0,
        next_value_id: 0,
        next_assignment_id: 0,
        next_record_field_id: 0,
        next_enum_variant_id: 0,
        next_pattern_id: 0,
        next_setting_choice_option_id: 0,
        diagnostics: initial_diagnostics,
        recovery_nodes: Vec::new(),
    }
    .program()
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    named_types: NamedTypeEnvironment,
    array_types: Vec<ArrayTypeDecl>,
    array_type_ids: HashMap<TypeRef, ArrayTypeId>,
    option_types: Vec<OptionTypeDecl>,
    option_type_ids: HashMap<TypeRef, OptionTypeId>,
    result_types: Vec<ResultTypeDecl>,
    result_type_ids: HashMap<TypeRef, ResultTypeId>,
    type_names: Vec<String>,
    type_name_ids: HashMap<String, TypeNameId>,
    next_constructed_type_id: u32,
    next_expression_id: u32,
    next_function_id: u32,
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
            let declaration_start = self.pos;
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
                if !program.settings.is_empty() {
                    Err(self.error("only one `settings(...)` declaration is allowed"))
                } else {
                    let declarations = if matches!(self.peek(1).kind, TokenKind::LParen) {
                        self.settings_decl()
                    } else {
                        self.settings_block_decl()
                    };
                    declarations.map(|declarations| program.settings = declarations)
                }
            } else if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
                if self.at_ident("const") || self.at_ident("var") {
                    self.record_let_keyword_diagnostic();
                }
                self.variable_decl().and_then(|declaration| {
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
                        function.debug_only = true;
                        function.span.start = start;
                        program.functions.push(function);
                    })
                } else if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
                    if self.at_ident("const") || self.at_ident("var") {
                        self.record_let_keyword_diagnostic();
                    }
                    self.variable_decl().and_then(|mut declaration| {
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
                self.function_decl()
                    .map(|function| program.functions.push(function))
            } else if self.at_ident("record") {
                self.record_decl()
                    .map(|record| program.records.push(record))
            } else if self.at_ident("enum") {
                self.enum_decl()
                    .map(|enumeration| program.enums.push(enumeration))
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
                let skipped_start = self.tokens[declaration_start].span.start;
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
        ParseOutput {
            program,
            diagnostics: self.diagnostics,
            recovery_nodes: self.recovery_nodes,
        }
    }

    fn enum_decl(&mut self) -> Result<EnumDecl, Diagnostic> {
        let start = self.expect_ident("enum")?.start;
        let (name, _) = self.expect_any_ident("expected an enum name")?;
        let Some(EnumTypeId::Source(id)) = self.named_types.enumeration(&name) else {
            unreachable!("standard-library enum declarations are rejected during name collection")
        };
        self.expect(TokenKind::LBrace, "expected `{` after the enum name")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated enum declaration");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (variant_name, variant_span) =
                    self.expect_any_ident("expected a variant name")?;
                let payload = if self.eat(&TokenKind::LParen).is_some() {
                    let (ty, _) = self.parse_type("expected a payload type")?;
                    self.expect(TokenKind::RParen, "expected `)` after the payload type")?;
                    Some(ty)
                } else {
                    None
                };
                let variant = EnumVariant {
                    id: self.new_enum_variant_id(),
                    name: variant_name,
                    payload,
                    span: variant_span.join(self.previous().span),
                };
                self.terminator()?;
                Ok(variant)
            })();
            if let Some(variant) = self.recover_delimited_item(parsed, item_start, body_depth) {
                variants.push(variant);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(EnumDecl {
            id,
            name,
            variants,
            span: Span { start, end },
        })
    }

    fn record_decl(&mut self) -> Result<RecordDecl, Diagnostic> {
        let start = self.expect_ident("record")?.start;
        let (name, _) = self.expect_any_ident("expected a record name")?;
        let Some(id) = self.named_types.record(&name) else {
            unreachable!("record declarations are collected before parsing")
        };
        self.expect(TokenKind::LBrace, "expected `{` after the record name")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated record declaration");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (field_name, field_start) = self.expect_any_ident("expected a field name")?;
                self.expect(TokenKind::Colon, "expected `:` after the field name")?;
                let (ty, type_span) = self.parse_type("expected a field type")?;
                let field = RecordField {
                    id: self.new_record_field_id(),
                    name: field_name,
                    ty,
                    span: field_start.join(type_span),
                };
                self.terminator()?;
                Ok(field)
            })();
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(RecordDecl {
            id,
            name,
            fields,
            span: Span { start, end },
        })
    }

    fn function_decl(&mut self) -> Result<FunctionDecl, Diagnostic> {
        let id = FunctionId::from_index(self.next_function_id);
        self.next_function_id += 1;
        let start = self.bump().span.start;
        let (first_name, first_span) = self.expect_any_ident("expected a function name")?;
        let (method_of, name) = if self.eat(&TokenKind::Dot).is_some() {
            let receiver_name = match first_name.as_str() {
                "string" => {
                    self.record_string_type_diagnostic(first_span);
                    "String"
                }
                "TimeSpan" => {
                    self.record_duration_type_diagnostic(first_span);
                    "Duration"
                }
                name => {
                    if let Some(replacement) = csharp_numeric_type(name) {
                        self.record_numeric_type_diagnostic(first_span, name, replacement);
                        replacement
                    } else {
                        &first_name
                    }
                }
            };
            let receiver = self.resolve_type(receiver_name, first_span)?;
            let method = self.expect_any_ident("expected a method name after `.`")?.0;
            (Some(receiver), method)
        } else {
            (None, first_name)
        };
        self.expect(TokenKind::LParen, "expected `(` after the function name")?;
        let mut params = method_of.map_or_else(Vec::new, |ty| {
            vec![Parameter {
                id: self.new_value_id(),
                name: "self".to_owned(),
                annotation: Some(ty),
                span: first_span,
            }]
        });
        let mut missing_closing_parenthesis = false;
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated parameter list");
                missing_closing_parenthesis = true;
                break;
            }
            if self.at(&TokenKind::LBrace) || self.at(&TokenKind::Minus) {
                self.record_missing(Diagnostic::new(
                    "expected `)` after the parameters",
                    self.current().span,
                ));
                missing_closing_parenthesis = true;
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (param_name, param_start) =
                    self.expect_any_ident("expected a parameter name")?;
                let (annotation, type_span) = if self.eat(&TokenKind::Colon).is_some() {
                    let (ty, span) = self.parse_type("expected a parameter type")?;
                    (Some(ty), span)
                } else {
                    (None, param_start)
                };
                if annotation == Some(TypeRef::Void) {
                    return Err(Diagnostic::new(
                        "parameters cannot have type `void`",
                        type_span,
                    ));
                }
                let parameter = Parameter {
                    id: self.new_value_id(),
                    name: param_name,
                    annotation,
                    span: param_start.join(type_span),
                };
                if self.eat(&TokenKind::Comma).is_none()
                    && !self.at(&TokenKind::RParen)
                    && !self.at(&TokenKind::LBrace)
                    && !self.at(&TokenKind::Minus)
                {
                    return Err(self.error("expected `,` between parameters"));
                }
                Ok(parameter)
            })();
            if let Some(parameter) = self.recover_parameter(parsed, item_start) {
                params.push(parameter);
            }
        }
        if !missing_closing_parenthesis {
            self.expect(TokenKind::RParen, "expected `)` after the parameters")?;
        }
        let return_annotation = if self.eat(&TokenKind::Minus).is_some() {
            self.expect(TokenKind::Gt, "expected `>` in the return arrow `->`")?;
            Some(self.parse_type("expected a return type")?.0)
        } else {
            None
        };
        let body = self.block()?;
        let span = Span {
            start,
            end: body.span.end,
        };
        Ok(FunctionDecl {
            id,
            name,
            debug_only: false,
            method_of,
            params,
            return_annotation,
            body,
            span,
        })
    }

    fn state_block_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        let processes = self.process_names()?;
        self.expect(
            TokenKind::LBrace,
            "expected `{` after the process name list",
        )?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state declaration");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (name, field_start) = self.expect_any_ident("expected a state field name")?;
                let annotation = if self.eat(&TokenKind::Colon).is_some() {
                    let (ty, type_span) = self.parse_type("expected a state field type")?;
                    if !type_can_be_stored_in_state(ty) {
                        return Err(Diagnostic::new(
                            format!("`{ty}` cannot be stored in state"),
                            type_span,
                        ));
                    }
                    Some(ty)
                } else {
                    None
                };
                let source = if self.eat(&TokenKind::Assign).is_some() {
                    StateSource::Expression(self.root_expression())
                } else {
                    self.expect_ident("at")?;
                    let module = if matches!(self.current().kind, TokenKind::String(_)) {
                        let module = self.expect_string("expected a module name")?;
                        self.expect(TokenKind::Comma, "expected an offset after the module")?;
                        Some(module)
                    } else {
                        None
                    };
                    let mut offsets = vec![self.expect_u64("expected a pointer offset")?];
                    while self.eat(&TokenKind::Comma).is_some() {
                        offsets.push(self.expect_u64("expected a pointer offset")?);
                    }
                    StateSource::Pointer(PointerPath { module, offsets })
                };
                let end = self.previous().span.end;
                self.terminator()?;
                Ok(StateField {
                    id: self.new_value_id(),
                    name,
                    annotation,
                    source,
                    span: Span {
                        start: field_start.start,
                        end,
                    },
                })
            })();
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(StateDecl {
            processes,
            fields,
            span: Span { start, end },
        })
    }

    fn process_names(&mut self) -> Result<Vec<String>, Diagnostic> {
        if matches!(self.current().kind, TokenKind::String(_)) {
            return Ok(vec![self.expect_string("expected a process name")?]);
        }
        self.expect(TokenKind::LBracket, "expected a process name or `[` list")?;
        let mut names = Vec::new();
        while !self.at(&TokenKind::RBracket) {
            names.push(self.expect_string("expected a process name string")?);
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(TokenKind::RBracket, "expected `]` after process names")?;
        if names.is_empty() {
            return Err(self.error("a process name list cannot be empty"));
        }
        Ok(names)
    }

    fn settings_block_decl(&mut self) -> Result<Vec<SettingDecl>, Diagnostic> {
        self.expect_ident("settings")?;
        self.expect(TokenKind::LBrace, "expected `{` after `settings`")?;
        let body_depth = self.brace_depth_before(self.pos);
        if matches!(
            self.current().kind,
            TokenKind::String(_) | TokenKind::DocComment(_)
        ) {
            let mut settings = Vec::new();
            let mut heading_count = 0;
            self.settings_dsl_entries(&mut settings, 0, &mut heading_count)?;
            if let Err(error) = self.expect(TokenKind::RBrace, "expected `}` after settings") {
                self.record_missing(error);
            }
            return Ok(settings);
        }

        let mut settings = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated settings declaration");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (name, start) = self.expect_any_ident("expected a setting name")?;
                self.expect(TokenKind::Colon, "expected `:` after the setting name")?;
                self.expect_ident("bool")?;
                self.expect(TokenKind::Assign, "expected `=` before the default value")?;
                let default = self.expect_bool("expected a boolean default value")?;
                let description = if self.eat(&TokenKind::Comma).is_some() {
                    self.expect_string("expected a setting description")?
                } else {
                    name.clone()
                };
                let end = self.previous().span.end;
                self.terminator()?;
                Ok(SettingDecl {
                    id: self.new_value_id(),
                    name,
                    description,
                    tooltip: None,
                    kind: SettingKind::Bool { default },
                    span: Span {
                        start: start.start,
                        end,
                    },
                })
            })();
            if let Some(setting) = self.recover_delimited_item(parsed, item_start, body_depth) {
                settings.push(setting);
            }
        }
        self.eat(&TokenKind::RBrace);
        Ok(settings)
    }

    fn settings_dsl_entries(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let body_depth = self.brace_depth_before(self.pos);
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated settings group");
                return Ok(());
            }
            let item_start = self.pos;
            let parsed = self.settings_dsl_entry(settings, heading_level, heading_count);
            self.recover_delimited_item(parsed, item_start, body_depth);
        }
        Ok(())
    }

    fn settings_dsl_entry(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let tooltip = self.take_doc_tooltip();
        if self.at(&TokenKind::RBrace) {
            return Err(self.error("a documentation comment must precede a setting or title"));
        }
        let label_token = self.current().clone();
        let TokenKind::String(description) = label_token.kind else {
            return Err(Diagnostic::new(
                "expected a quoted setting description",
                label_token.span,
            ));
        };
        self.bump();
        if self.at(&TokenKind::Comma) {
            return Err(
                self.error("setting tooltips use `///` documentation comments before the setting")
            );
        }

        if self.eat(&TokenKind::LBrace).is_some() {
            let name = format!("_heading{}", *heading_count);
            *heading_count += 1;
            let title_index = settings.len();
            settings.push(SettingDecl {
                id: self.new_value_id(),
                name,
                description,
                tooltip,
                kind: SettingKind::Title { heading_level },
                span: label_token.span,
            });
            self.settings_dsl_entries(settings, heading_level + 1, heading_count)?;
            let end = self.expect(TokenKind::RBrace, "expected `}` after setting group")?;
            settings[title_index].span = label_token.span.join(end);
            self.eat(&TokenKind::Comma);
            self.eat(&TokenKind::Semicolon);
            return Ok(());
        }

        self.expect(
            TokenKind::FatArrow,
            "expected `=>` after the setting description",
        )?;
        let (name, name_span) = self.expect_any_ident("expected a setting name")?;
        self.expect(TokenKind::Colon, "expected `:` after the setting name")?;
        let (kind_name, kind_span) =
            self.expect_any_ident("expected a setting default, `choice`, or `file`")?;
        let kind = match kind_name.as_str() {
            "true" => SettingKind::Bool { default: true },
            "false" => SettingKind::Bool { default: false },
            "choice" => self.choice_setting()?,
            "file" => self.file_setting()?,
            _ => {
                return Err(Diagnostic::new(
                    "expected `true`, `false`, `choice`, or `file`",
                    kind_span,
                ));
            }
        };
        let end = self.previous().span;
        settings.push(SettingDecl {
            id: self.new_value_id(),
            name,
            description,
            tooltip,
            kind,
            span: label_token.span.join(end).join(name_span),
        });
        if self.eat(&TokenKind::Comma).is_none()
            && self.eat(&TokenKind::Semicolon).is_none()
            && !self.at(&TokenKind::RBrace)
            && !self.line_break_before_current()
        {
            return Err(self.error("expected a line break or `,` between settings"));
        }
        Ok(())
    }

    fn take_doc_tooltip(&mut self) -> Option<String> {
        let mut tooltip = String::new();
        let mut blank_lines = 0usize;
        while let TokenKind::DocComment(line) = self.current().kind.clone() {
            self.bump();
            let line = line.trim();
            if line.is_empty() {
                if !tooltip.is_empty() {
                    blank_lines += 1;
                }
                continue;
            }
            if !tooltip.is_empty() {
                if blank_lines == 0 {
                    tooltip.push(' ');
                } else {
                    for _ in 0..blank_lines {
                        tooltip.push('\n');
                    }
                }
            }
            blank_lines = 0;
            tooltip.push_str(line);
        }
        (!tooltip.is_empty()).then_some(tooltip)
    }

    fn choice_setting(&mut self) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `choice`")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut enumeration = None;
        let mut options = Vec::new();
        let mut default_variant = None;
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated choice setting");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let option_start = self.current().span;
                let description = self.expect_string("expected a choice option description")?;
                self.expect(
                    TokenKind::FatArrow,
                    "expected `=>` after the option description",
                )?;
                let (enum_name, enum_span) = self.expect_any_ident("expected an enum name")?;
                let Some(EnumTypeId::Source(enum_id)) = self.named_types.enumeration(&enum_name)
                else {
                    return Err(Diagnostic::new(
                        format!("choice settings require a source enum, found `{enum_name}`"),
                        enum_span,
                    ));
                };
                if enumeration.is_some_and(|previous| previous != enum_id) {
                    return Err(Diagnostic::new(
                        "all choice options must belong to the same enum",
                        enum_span,
                    ));
                }
                self.expect(TokenKind::Dot, "expected `.` before the enum variant")?;
                let (variant, variant_span) = self.expect_any_ident("expected an enum variant")?;
                let is_default = self.eat_ident("default").is_some();
                if is_default && default_variant.is_some() {
                    return Err(Diagnostic::new(
                        "a choice can only have one default option",
                        variant_span,
                    ));
                }
                let span = option_start.join(self.previous().span);
                if self.eat(&TokenKind::Comma).is_none()
                    && !self.at(&TokenKind::RBrace)
                    && !self.line_break_before_current()
                {
                    return Err(self.error("expected a line break or `,` between choice options"));
                }
                Ok((enum_id, description, variant, is_default, span))
            })();
            if let Some((enum_id, description, variant, is_default, span)) =
                self.recover_delimited_item(parsed, item_start, body_depth)
            {
                enumeration.get_or_insert(enum_id);
                if is_default {
                    default_variant = Some(variant.clone());
                }
                options.push(SettingChoiceOption {
                    id: self.new_setting_choice_option_id(),
                    variant,
                    description,
                    span,
                });
            }
        }
        self.eat(&TokenKind::RBrace);
        let Some(enumeration) = enumeration else {
            return Err(self.error("a choice needs at least one option"));
        };
        let default_variant = default_variant.unwrap_or_else(|| options[0].variant.clone());
        Ok(SettingKind::Choice {
            enumeration,
            default_variant,
            options,
        })
    }

    fn file_setting(&mut self) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `file`")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut filters = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated file setting");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let filter = match self.current().kind.clone() {
                    TokenKind::String(description) => {
                        self.bump();
                        self.expect(TokenKind::FatArrow, "expected `=>` after the filter name")?;
                        let pattern = self.expect_string("expected a file-name pattern")?;
                        SettingFileFilter::Name {
                            description: Some(description),
                            pattern,
                        }
                    }
                    TokenKind::Ident(name) if name == "_" => {
                        self.bump();
                        self.expect(TokenKind::FatArrow, "expected `=>` after `_`")?;
                        let pattern = self.expect_string("expected a file-name pattern")?;
                        SettingFileFilter::Name {
                            description: None,
                            pattern,
                        }
                    }
                    TokenKind::Ident(name) if name == "mime" => {
                        self.bump();
                        self.expect(TokenKind::FatArrow, "expected `=>` after `mime`")?;
                        SettingFileFilter::Mime(self.expect_string("expected a MIME type")?)
                    }
                    _ => {
                        return Err(
                            self.error("expected a named filter, `_` filter, `mime`, or `}`")
                        );
                    }
                };
                if self.eat(&TokenKind::Comma).is_none()
                    && !self.at(&TokenKind::RBrace)
                    && !self.line_break_before_current()
                {
                    return Err(self.error("expected a line break or `,` between file filters"));
                }
                Ok(filter)
            })();
            if let Some(filter) = self.recover_delimited_item(parsed, item_start, body_depth) {
                filters.push(filter);
            }
        }
        self.eat(&TokenKind::RBrace);
        Ok(SettingKind::File { filters })
    }

    fn action_block(&mut self) -> Result<Action, Diagnostic> {
        let (name, name_span) = self.expect_any_ident("expected an action name")?;
        let Some(kind) = ActionKind::parse(&name) else {
            return Err(Diagnostic::new(
                format!("unknown action `{name}`"),
                name_span,
            ));
        };
        let body = self.block()?;
        Ok(Action {
            kind,
            span: name_span.join(body.span),
            body,
        })
    }

    fn state_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        self.expect(TokenKind::LParen, "expected `(` after `state`")?;
        let process = self.expect_string("expected a process name string")?;
        self.expect(TokenKind::Comma, "expected `,` after the process name")?;
        self.expect(TokenKind::LBrace, "expected a state object")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state object");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (name, field_start) = self.expect_any_ident("expected a state field name")?;
                self.expect(TokenKind::Colon, "expected `:` after the field name")?;
                self.expect_ident("memory")?;
                self.expect(TokenKind::Dot, "expected `.` after `memory`")?;
                let (type_name, type_span) = self.expect_any_ident("expected a memory type")?;
                let Some(ty) = TypeRef::parse(&type_name) else {
                    return Err(Diagnostic::new(
                        format!("`memory.{type_name}` is not a supported memory type"),
                        type_span,
                    ));
                };
                if !type_can_be_stored_in_state(ty) {
                    return Err(Diagnostic::new(
                        format!("`{ty}` cannot be read from process memory"),
                        type_span,
                    ));
                }
                self.expect(TokenKind::LParen, "expected `(` after the memory type")?;
                let module = if matches!(self.current().kind, TokenKind::String(_)) {
                    let module = self.expect_string("expected module name")?;
                    self.expect(TokenKind::Comma, "expected an address offset")?;
                    Some(module)
                } else {
                    None
                };
                let mut offsets = Vec::new();
                loop {
                    offsets.push(self.expect_u64("expected a pointer offset")?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                let end = self
                    .expect(TokenKind::RParen, "expected `)` after the pointer path")?
                    .end;
                let field = StateField {
                    id: self.new_value_id(),
                    name,
                    annotation: Some(ty),
                    source: StateSource::Pointer(PointerPath { module, offsets }),
                    span: Span {
                        start: field_start.start,
                        end,
                    },
                };
                if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
                    return Err(self.error("expected `,` between state fields"));
                }
                Ok(field)
            })();
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        self.eat(&TokenKind::RBrace);
        if let Err(error) = self.expect(TokenKind::RParen, "expected `)` after the state object") {
            self.record_missing(error);
        }
        self.eat(&TokenKind::Semicolon);
        Ok(StateDecl {
            processes: vec![process],
            fields,
            span: Span {
                start,
                end: self.previous().span.end,
            },
        })
    }

    fn settings_decl(&mut self) -> Result<Vec<SettingDecl>, Diagnostic> {
        self.expect_ident("settings")?;
        self.expect(TokenKind::LParen, "expected `(` after `settings`")?;
        self.expect(TokenKind::LBrace, "expected a settings object")?;
        let body_depth = self.brace_depth_before(self.pos);
        let mut settings = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated settings object");
                break;
            }
            let item_start = self.pos;
            let parsed = (|| {
                let (name, start) = self.expect_any_ident("expected a setting name")?;
                self.expect(TokenKind::Colon, "expected `:` after the setting name")?;
                self.expect_ident("Setting")?;
                self.expect(TokenKind::Dot, "expected `.` after `Setting`")?;
                self.expect_ident("bool")?;
                self.expect(TokenKind::LParen, "expected `(` after `Setting.bool`")?;
                let default = self.expect_bool("expected a boolean default value")?;
                let description = if self.eat(&TokenKind::Comma).is_some() {
                    self.expect_string("expected a setting description")?
                } else {
                    name.clone()
                };
                let end = self
                    .expect(TokenKind::RParen, "expected `)` after the setting")?
                    .end;
                let setting = SettingDecl {
                    id: self.new_value_id(),
                    name,
                    description,
                    tooltip: None,
                    kind: SettingKind::Bool { default },
                    span: Span {
                        start: start.start,
                        end,
                    },
                };
                if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
                    return Err(self.error("expected `,` between settings"));
                }
                Ok(setting)
            })();
            if let Some(setting) = self.recover_delimited_item(parsed, item_start, body_depth) {
                settings.push(setting);
            }
        }
        self.eat(&TokenKind::RBrace);
        if let Err(error) = self.expect(TokenKind::RParen, "expected `)` after the settings object")
        {
            self.record_missing(error);
        }
        self.eat(&TokenKind::Semicolon);
        Ok(settings)
    }

    fn block(&mut self) -> Result<Block, Diagnostic> {
        let start = self
            .expect(TokenKind::LBrace, "expected `{` to start a block")?
            .start;
        let block_depth = self.brace_depth_before(self.pos);
        let mut statements = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                let error = self.error("unterminated block");
                self.diagnostics.push(error);
                self.recovery_nodes.push(RecoveryNode {
                    kind: RecoveryNodeKind::Missing,
                    span: self.current().span,
                });
                return Ok(Block {
                    statements,
                    span: Span {
                        start,
                        end: self.current().span.end,
                    },
                });
            }
            let statement_start = self.pos;
            match self.statement() {
                Ok(statement) => statements.push(statement),
                Err(error) => {
                    if error.message.starts_with("expected") {
                        self.recovery_nodes.push(RecoveryNode {
                            kind: RecoveryNodeKind::Missing,
                            span: Span {
                                start: error.span.start,
                                end: error.span.start,
                            },
                        });
                    }
                    self.diagnostics.push(error);
                    let skipped_start = self.tokens[statement_start].span.start;
                    self.synchronize_statement(statement_start, block_depth);
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
        }
        let end = self.bump().span.end;
        Ok(Block {
            statements,
            span: Span { start, end },
        })
    }

    fn statement(&mut self) -> Result<Stmt, Diagnostic> {
        if self.eat_ident("debug").is_some() {
            let start = self.previous().span.start;
            if self.at_ident("debug") {
                return Err(self.error("a statement cannot have more than one `debug` modifier"));
            }
            let statement = self.statement()?;
            let end = statement_span(&statement).end;
            return Ok(Stmt::Debug {
                statement: Box::new(statement),
                span: Span { start, end },
            });
        }
        if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
            if self.at_ident("const") || self.at_ident("var") {
                self.record_let_keyword_diagnostic();
            }
            if self.is_suspension_binding() {
                return self.suspension_binding();
            }
            let declaration = self.variable_decl()?;
            self.terminator()?;
            return Ok(Stmt::Variable(declaration));
        }
        if self.eat_ident("if").is_some() {
            let start = self.previous().span.start;
            return self.if_statement(start);
        }
        if self.eat_ident("while").is_some() {
            let start = self.previous().span.start;
            let condition = self.root_expression();
            let body = self.block()?;
            let end = body.span.end;
            return Ok(Stmt::While {
                condition,
                body,
                span: Span { start, end },
            });
        }
        if self.eat_ident("break").is_some() {
            let start = self.previous().span.start;
            self.terminator()?;
            return Ok(Stmt::Break {
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
            });
        }
        if self.eat_ident("continue").is_some() {
            let start = self.previous().span.start;
            self.terminator()?;
            return Ok(Stmt::Continue {
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
            });
        }
        if self.eat_ident("return").is_some() {
            let start = self.previous().span.start;
            let value = if self.at(&TokenKind::Semicolon)
                || self.at(&TokenKind::RBrace)
                || self.line_break_before_current()
            {
                None
            } else {
                Some(self.root_expression())
            };
            self.terminator()?;
            return Ok(Stmt::Return {
                value,
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
            });
        }
        if self.eat_ident("throw").is_some() {
            let start = self.previous().span.start;
            let error = if self.at(&TokenKind::Semicolon)
                || self.at(&TokenKind::RBrace)
                || self.line_break_before_current()
            {
                self.missing_root_expression("expected an error expression after `throw`")
            } else {
                self.root_expression()
            };
            self.terminator()?;
            return Ok(Stmt::Throw {
                error,
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
            });
        }
        if self.at_ident("await") || self.at_ident("retry") {
            let mode = if self.eat_ident("await").is_some() {
                SuspensionMode::Await
            } else {
                self.expect_ident("retry")?;
                SuspensionMode::Retry
            };
            let start = self.previous().span.start;
            let value = self.root_expression();
            self.terminator()?;
            return Ok(Stmt::Suspend {
                mode,
                binding: None,
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
                value,
            });
        }
        if let TokenKind::Ident(name) = &self.current().kind
            && let Some(op) = assignment_operator(&self.peek(1).kind)
        {
            let name = name.clone();
            let start = self.bump().span.start;
            self.bump();
            let value = self.root_expression();
            self.terminator()?;
            return Ok(Stmt::Assign {
                id: self.new_assignment_id(),
                name,
                op,
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
                value,
            });
        }
        let expr = self.root_expression();
        self.terminator()?;
        Ok(Stmt::Expression(expr))
    }

    fn if_statement(&mut self, start: usize) -> Result<Stmt, Diagnostic> {
        let condition = self.root_expression();
        let then_block = self.block()?;
        let else_block = if self.eat_ident("else").is_some() {
            if self.eat_ident("if").is_some() {
                let nested_start = self.previous().span.start;
                let nested = self.if_statement(nested_start)?;
                let span = match &nested {
                    Stmt::If { span, .. } => *span,
                    _ => unreachable!(),
                };
                Some(Block {
                    statements: vec![nested],
                    span,
                })
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map_or(then_block.span.end, |block| block.span.end);
        Ok(Stmt::If {
            condition,
            then_block,
            else_block,
            span: Span { start, end },
        })
    }

    fn is_suspension_binding(&self) -> bool {
        let mut offset = 2;
        while !matches!(self.peek(offset).kind, TokenKind::Assign | TokenKind::Eof) {
            offset += 1;
        }
        matches!(self.peek(offset).kind, TokenKind::Assign)
            && matches!(&self.peek(offset + 1).kind, TokenKind::Ident(name) if name == "await" || name == "retry")
    }

    fn suspension_binding(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump().span.start;
        let (name, name_span) = self.expect_any_ident("expected a variable name")?;
        let annotation = if self.eat(&TokenKind::Colon).is_some() {
            Some(self.parse_type("expected a type name")?.0)
        } else {
            None
        };
        self.expect(TokenKind::Assign, "expected `=` in variable declaration")?;
        let mode = if self.eat_ident("await").is_some() {
            SuspensionMode::Await
        } else {
            self.expect_ident("retry")?;
            SuspensionMode::Retry
        };
        let value = self.root_expression();
        self.terminator()?;
        let span = Span {
            start,
            end: self.previous().span.end,
        };
        Ok(Stmt::Suspend {
            mode,
            binding: Some(SuspensionBinding {
                id: self.new_value_id(),
                name,
                annotation,
                span: Span {
                    start,
                    end: value.span.end.max(name_span.end),
                },
            }),
            value,
            span,
        })
    }

    fn variable_decl(&mut self) -> Result<VariableDecl, Diagnostic> {
        let keyword = self.bump().clone();
        let (name, name_span) = self.expect_any_ident("expected a variable name")?;
        let annotation = if self.eat(&TokenKind::Colon).is_some() {
            Some(self.parse_type("expected a type name")?.0)
        } else {
            None
        };
        self.expect(TokenKind::Assign, "expected `=` in variable declaration")?;
        let value = self.root_expression();
        Ok(VariableDecl {
            id: self.new_value_id(),
            name,
            mutable: true,
            debug_only: false,
            annotation,
            span: Span {
                start: keyword.span.start,
                end: value.span.end.max(name_span.end),
            },
            value,
        })
    }

    fn expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let mut left = self.prefix()?;
        let mut saw_comparison = false;
        loop {
            if self.eat(&TokenKind::Dot).is_some() {
                let (name, name_span) = self.expect_any_ident("expected a field name after `.`")?;
                let span = left.span.join(name_span);
                left = self.new_expr(
                    ExprKind::Member {
                        receiver: Box::new(left),
                        name,
                        name_span,
                    },
                    span,
                );
                continue;
            }
            if let Some(question) = self.eat(&TokenKind::Question) {
                let span = left.span.join(question);
                left = self.new_expr(ExprKind::Propagate(Box::new(left)), span);
                continue;
            }
            const FALLBACK_PRECEDENCE: u8 = 0;
            if self.at_ident("else") {
                if FALLBACK_PRECEDENCE < min_precedence {
                    break;
                }
                self.bump();
                let fallback = if self.eat_ident("return").is_some() {
                    let return_span = self.previous().span;
                    let value = if self.at(&TokenKind::Semicolon)
                        || self.at(&TokenKind::RBrace)
                        || self.at(&TokenKind::Eof)
                        || self.line_break_before_current()
                    {
                        None
                    } else {
                        Some(Box::new(self.expression(FALLBACK_PRECEDENCE)?))
                    };
                    let span = value
                        .as_ref()
                        .map_or(return_span, |value| return_span.join(value.span));
                    FallbackBranch::Return { value, span }
                } else if self.eat_ident("break").is_some() {
                    FallbackBranch::Break {
                        span: self.previous().span,
                    }
                } else if self.eat_ident("continue").is_some() {
                    FallbackBranch::Continue {
                        span: self.previous().span,
                    }
                } else {
                    FallbackBranch::Value(Box::new(self.expression(FALLBACK_PRECEDENCE)?))
                };
                let end = match &fallback {
                    FallbackBranch::Value(value) => value.span,
                    FallbackBranch::Return { span, .. } => *span,
                    FallbackBranch::Break { span } | FallbackBranch::Continue { span } => *span,
                };
                let span = left.span.join(end);
                left = self.new_expr(
                    ExprKind::Fallback {
                        value: Box::new(left),
                        fallback,
                    },
                    span,
                );
                continue;
            }
            const CAST_PRECEDENCE: u8 = 10;
            if self.at_ident("as") {
                if CAST_PRECEDENCE < min_precedence {
                    break;
                }
                self.bump();
                let (target, target_span) = self.parse_type("expected a type after `as`")?;
                let span = left.span.join(target_span);
                left = self.new_expr(
                    ExprKind::Cast {
                        expr: Box::new(left),
                        target,
                    },
                    span,
                );
                continue;
            }
            let Some((precedence, op)) = self.binary_operator() else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            let is_comparison = matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            );
            if is_comparison && saw_comparison {
                return Err(self.error(
                    "comparison operators cannot be chained; use parentheses to disambiguate",
                ));
            }
            self.bump();
            let right = self.required_expression(precedence + 1)?;
            let span = left.span.join(right.span);
            left = self.new_expr(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            );
            saw_comparison |= is_comparison;
        }
        Ok(left)
    }

    fn prefix(&mut self) -> Result<Expr, Diagnostic> {
        if self.eat_ident("if").is_some() {
            let start = self.previous().span;
            return self.if_expression(start);
        }
        if self.eat_ident("match").is_some() {
            let start = self.previous().span;
            let value = self.required_expression(0)?;
            self.expect(TokenKind::LBrace, "expected `{` after the matched value")?;
            let body_depth = self.brace_depth_before(self.pos);
            let mut arms = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                if self.at(&TokenKind::Eof) {
                    self.record_missing_closing("unterminated match expression");
                    break;
                }
                let item_start = self.pos;
                let parsed = self.match_arm();
                if let Some(arm) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    arms.push(arm);
                }
            }
            let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
            return Ok(self.new_expr(
                ExprKind::Match {
                    value: Box::new(value),
                    arms,
                },
                start.join(end),
            ));
        }
        if self.eat(&TokenKind::Bang).is_some() {
            let start = self.previous().span;
            let expr = self.required_prefix()?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.eat(&TokenKind::Minus).is_some() {
            let start = self.previous().span;
            let expr = self.required_prefix()?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.eat(&TokenKind::LParen).is_some() {
            let start = self.previous().span;
            let target_depth = self.delimiter_depth_before(self.pos);
            let mut expr = self.required_expression(0)?;
            let end = if let Some(end) = self.eat(&TokenKind::RParen) {
                end
            } else {
                let error = self.error("expected `)` after expression");
                self.record_recovery_diagnostic(error);
                let skipped_start = self.current().span.start;
                self.synchronize_delimited_expression(&TokenKind::RParen, target_depth);
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::RParen).unwrap_or(expr.span)
            };
            expr.span = start.join(end);
            return Ok(expr);
        }
        if self.eat(&TokenKind::LBracket).is_some() {
            let start = self.previous().span;
            let (elements, end) = self.expression_list(
                TokenKind::RBracket,
                "expected `]` after array elements",
                true,
            );
            return Ok(self.new_expr(ExprKind::Array(elements), start.join(end)));
        }
        if self.eat(&TokenKind::TemplateStart).is_some() {
            let start = self.previous().span;
            let mut parts = Vec::new();
            let mut has_expression = false;
            loop {
                match self.current().kind.clone() {
                    TokenKind::TemplateChunk(value) => {
                        self.bump();
                        parts.push(InterpolatedPart::Text(value));
                    }
                    TokenKind::TemplateExprStart => {
                        self.bump();
                        let expression_start = self.pos;
                        if let Some(value) = self.interpolated_expression(expression_start) {
                            has_expression = true;
                            parts.push(InterpolatedPart::Expr(value));
                        }
                    }
                    TokenKind::TemplateEnd => {
                        let end = self.bump().span;
                        if has_expression {
                            return Ok(
                                self.new_expr(ExprKind::InterpolatedString(parts), start.join(end))
                            );
                        }
                        let value = parts
                            .into_iter()
                            .map(|part| match part {
                                InterpolatedPart::Text(value) => value,
                                InterpolatedPart::Expr(_) => unreachable!(),
                            })
                            .collect();
                        return Ok(self.new_expr(ExprKind::String(value), start.join(end)));
                    }
                    _ => {
                        return Err(self.error(
                            "expected template text, an interpolation, or a closing backtick",
                        ));
                    }
                }
            }
        }

        let token = self.current().clone();
        match token.kind {
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_none_keyword_diagnostic(token.span);
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(true), token.span))
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(false), token.span))
            }
            TokenKind::Ident(mut first) => {
                self.bump();
                if self.at(&TokenKind::Dot) {
                    match first.as_str() {
                        "string" => {
                            self.record_string_type_diagnostic(token.span);
                            first = "String".to_owned();
                        }
                        "TimeSpan" => {
                            self.record_duration_type_diagnostic(token.span);
                            first = "Duration".to_owned();
                        }
                        _ => {}
                    }
                }
                if first == "sig" {
                    let signature_span = self.current().span;
                    let value = self.expect_string("expected a quoted pattern after `sig`")?;
                    return Ok(
                        self.new_expr(ExprKind::Signature(value), token.span.join(signature_span))
                    );
                }
                if let Some(record) = self.named_types.record(&first)
                    && self.eat(&TokenKind::LBrace).is_some()
                {
                    let body_depth = self.brace_depth_before(self.pos);
                    let mut fields = Vec::new();
                    while !self.at(&TokenKind::RBrace) {
                        if self.at(&TokenKind::Eof) {
                            self.record_missing_closing("unterminated record literal");
                            break;
                        }
                        let item_start = self.pos;
                        let parsed = self.record_literal_field();
                        if let Some(field) =
                            self.recover_delimited_item(parsed, item_start, body_depth)
                        {
                            fields.push(field);
                            if self.eat(&TokenKind::Comma).is_some() {
                                continue;
                            }
                            if self.at(&TokenKind::RBrace) {
                                continue;
                            }
                            self.record_missing(Diagnostic::new(
                                "expected `,` between record fields",
                                self.current().span,
                            ));
                            if matches!(self.current().kind, TokenKind::Ident(_)) {
                                continue;
                            }
                            self.synchronize_delimited_item(item_start, body_depth);
                        }
                    }
                    let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
                    return Ok(
                        self.new_expr(ExprKind::Record { record, fields }, token.span.join(end))
                    );
                }
                let mut path = vec![first];
                let mut name_span = token.span;
                while self.eat(&TokenKind::Dot).is_some() {
                    let (name, span) = self.expect_any_ident("expected a name after `.`")?;
                    path.push(name);
                    name_span = span;
                }
                if self.eat(&TokenKind::LParen).is_some() {
                    let (args, end) = self.expression_list(
                        TokenKind::RParen,
                        "expected `)` after arguments",
                        false,
                    );
                    if let [enum_name, variant] = path.as_slice()
                        && let Some(enumeration) = self.named_types.enumeration(enum_name)
                    {
                        if args.len() > 1 {
                            return Err(Diagnostic::new(
                                "enum constructors accept at most one payload",
                                token.span.join(end),
                            ));
                        }
                        return Ok(self.new_expr(
                            ExprKind::Enum {
                                enumeration,
                                variant: variant.clone(),
                                payload: args.into_iter().next().map(Box::new),
                            },
                            token.span.join(end),
                        ));
                    }
                    Ok(self.new_expr(
                        ExprKind::Call {
                            callee: path,
                            name_span,
                            args,
                        },
                        token.span.join(end),
                    ))
                } else {
                    let end = self.previous().span;
                    if let [enum_name, variant] = path.as_slice()
                        && let Some(enumeration) = self.named_types.enumeration(enum_name)
                    {
                        return Ok(self.new_expr(
                            ExprKind::Enum {
                                enumeration,
                                variant: variant.clone(),
                                payload: None,
                            },
                            token.span.join(end),
                        ));
                    }
                    Ok(self.new_expr(ExprKind::Path(path), token.span.join(end)))
                }
            }
            TokenKind::Int(text) => {
                let (value, suffix) =
                    parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
                self.bump();
                Ok(self.new_expr(ExprKind::Int { value, suffix }, token.span))
            }
            TokenKind::Float(text) => {
                let value = text
                    .replace('_', "")
                    .parse()
                    .map_err(|_| Diagnostic::new("invalid floating-point literal", token.span))?;
                self.bump();
                Ok(self.new_expr(ExprKind::Float(value), token.span))
            }
            TokenKind::String(value) => {
                self.bump();
                Ok(self.new_expr(ExprKind::String(value), token.span))
            }
            _ => Err(Diagnostic::new("expected an expression", token.span)),
        }
    }

    fn record_literal_field(&mut self) -> Result<(String, Expr), Diagnostic> {
        let (name, _) = self.expect_any_ident("expected a record field name")?;
        self.expect(TokenKind::Colon, "expected `:` after the field name")?;
        let value = self.expression(0)?;
        Ok((name, value))
    }

    fn interpolated_expression(&mut self, expression_start: usize) -> Option<Expr> {
        match self.expression(0) {
            Ok(value) => {
                if self.eat(&TokenKind::TemplateExprEnd).is_none() {
                    let error = self.error("expected `}` after the interpolated expression");
                    self.record_recovery_diagnostic(error);
                    let skipped_start = self.current().span.start;
                    self.synchronize_interpolation();
                    self.record_error_region(skipped_start, self.current().span.start);
                    self.eat(&TokenKind::TemplateExprEnd);
                }
                Some(value)
            }
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[expression_start].span.start;
                self.synchronize_interpolation();
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::TemplateExprEnd);
                None
            }
        }
    }

    fn synchronize_interpolation(&mut self) {
        let mut nested_interpolations = 0u32;
        loop {
            match self.current().kind {
                TokenKind::Eof => return,
                TokenKind::TemplateExprEnd if nested_interpolations == 0 => return,
                TokenKind::TemplateExprStart => {
                    nested_interpolations += 1;
                    self.bump();
                }
                TokenKind::TemplateExprEnd => {
                    nested_interpolations -= 1;
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn recover_required_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
    ) -> Result<Expr, Diagnostic> {
        match parsed {
            Ok(expression) => Ok(expression),
            Err(error) if self.is_expression_recovery_boundary() => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[expression_start].span.start;
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                Ok(self.new_expr(ExprKind::Error, span))
            }
            Err(error) => Err(error),
        }
    }

    fn root_expression(&mut self) -> Expr {
        let expression_start = self.pos;
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(0)
        };
        self.recover_root_expression(parsed, expression_start)
    }

    fn missing_root_expression(&mut self, message: &'static str) -> Expr {
        let expression_start = self.pos;
        let error = self.error(message);
        self.recover_root_expression(Err(error), expression_start)
    }

    fn recover_root_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
    ) -> Expr {
        match parsed {
            Ok(expression) => expression,
            Err(error) => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[expression_start].span.start;
                self.synchronize_root_expression(expression_start);
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                self.new_expr(ExprKind::Error, span)
            }
        }
    }

    fn synchronize_root_expression(&mut self, expression_start: usize) {
        let target_depth = self.delimiter_depth_before(expression_start);
        let mut depth = self.delimiter_depth_before(self.pos);
        loop {
            let at_same_brace_depth = depth.braces == target_depth.braces;
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::Semicolon) && at_same_brace_depth)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || (self.at(&TokenKind::RBrace) && depth.braces <= target_depth.braces)
                || (self.at(&TokenKind::RParen)
                    && at_same_brace_depth
                    && depth.parentheses <= target_depth.parentheses)
                || (self.at(&TokenKind::RBracket)
                    && at_same_brace_depth
                    && depth.brackets <= target_depth.brackets)
                || (at_same_brace_depth
                    && self.line_break_before_current()
                    && (self.pos > expression_start || self.is_statement_start()))
                || (self.pos > expression_start
                    && depth == target_depth
                    && self.is_top_level_start())
            {
                return;
            }
            let kind = self.bump().kind.clone();
            depth.update(&kind);
        }
    }

    fn required_expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let expression_start = self.pos;
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(min_precedence)
        };
        self.recover_required_expression(parsed, expression_start)
    }

    fn required_prefix(&mut self) -> Result<Expr, Diagnostic> {
        let expression_start = self.pos;
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.prefix()
        };
        self.recover_required_expression(parsed, expression_start)
    }

    fn expression_is_missing_before_statement(&self) -> bool {
        if !self.line_break_before_current() {
            return false;
        }
        match &self.current().kind {
            TokenKind::Ident(name) => {
                matches!(
                    name.as_str(),
                    "debug"
                        | "let"
                        | "const"
                        | "var"
                        | "while"
                        | "break"
                        | "continue"
                        | "return"
                        | "throw"
                        | "await"
                        | "retry"
                ) || assignment_operator(&self.peek(1).kind).is_some()
            }
            _ => false,
        }
    }

    fn is_expression_recovery_boundary(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Eof
                | TokenKind::LBrace
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::TemplateExprEnd
        ) || self.at_ident("else")
            || (self.line_break_before_current() && self.is_statement_start())
    }

    fn synchronize_delimited_expression(
        &mut self,
        closing: &TokenKind,
        target_depth: DelimiterDepth,
    ) {
        let mut depth = self.delimiter_depth_before(self.pos);
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.at(&TokenKind::TemplateExprEnd)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || self.at_ident("else")
                || self.is_expression_list_boundary(closing, depth, target_depth)
                || (depth == target_depth
                    && self.line_break_before_current()
                    && self.is_statement_start())
            {
                return;
            }
            let kind = self.bump().kind.clone();
            depth.update(&kind);
        }
    }

    fn match_arm(&mut self) -> Result<MatchArm, Diagnostic> {
        let token = self.current().clone();
        let pattern_start = token.span;
        let pattern = match token.kind {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                MatchPattern::Wildcard
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                MatchPattern::Bool(true)
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                MatchPattern::Bool(false)
            }
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_none_keyword_diagnostic(token.span);
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name)
                if matches!(name.as_str(), "Some" | "Ok" | "Err")
                    && matches!(self.peek(1).kind, TokenKind::LParen) =>
            {
                self.bump();
                self.bump();
                let binding_name = self
                    .expect_any_ident("expected a binding or `_` in the wrapper pattern")?
                    .0;
                self.expect(TokenKind::RParen, "expected `)` after the wrapper binding")?;
                let binding = (binding_name != "_").then(|| PatternBinding {
                    id: self.new_value_id(),
                    name: binding_name,
                });
                match name.as_str() {
                    "Some" => MatchPattern::OptionSome(binding),
                    "Ok" => MatchPattern::ResultSuccess(binding),
                    "Err" => MatchPattern::ResultError(binding),
                    _ => unreachable!(),
                }
            }
            TokenKind::Ident(enum_name) => {
                self.bump();
                if self.eat(&TokenKind::Dot).is_some() {
                    let Some(enumeration) = self.named_types.enumeration(&enum_name) else {
                        return Err(Diagnostic::new(
                            format!("unknown enum `{enum_name}` in match pattern"),
                            pattern_start,
                        ));
                    };
                    let (variant, _) = self.expect_any_ident("expected a variant name")?;
                    let binding = if self.eat(&TokenKind::LParen).is_some() {
                        let name = self.expect_any_ident("expected a payload binding")?.0;
                        self.expect(TokenKind::RParen, "expected `)` after the payload binding")?;
                        Some(PatternBinding {
                            id: self.new_value_id(),
                            name,
                        })
                    } else {
                        None
                    };
                    MatchPattern::Enum {
                        enumeration,
                        variant,
                        binding,
                    }
                } else {
                    return Err(Diagnostic::new(
                        format!(
                            "bare binding `{enum_name}` would match every value; use `Some({enum_name})` or `Ok({enum_name})` to match a wrapper payload"
                        ),
                        pattern_start,
                    ));
                }
            }
            TokenKind::Int(text) => {
                self.bump();
                let (value, suffix) = parse_integer(&text)
                    .map_err(|message| Diagnostic::new(message, pattern_start))?;
                MatchPattern::Int { value, suffix }
            }
            _ => {
                return Err(Diagnostic::new(
                    "expected an enum variant, integer, boolean, `None`, `Some(value)`, `Ok(value)`, `Err(error)`, or `_` pattern",
                    pattern_start,
                ));
            }
        };
        let guard = if self.eat_ident("if").is_some() {
            Some(self.expression(0)?)
        } else {
            None
        };
        self.expect(TokenKind::FatArrow, "expected `=>` after the pattern")?;
        let value = self.expression(0)?;
        let span = pattern_start.join(value.span);
        let arm = MatchArm {
            pattern_id: self.new_pattern_id(),
            pattern,
            guard,
            value,
            span,
        };
        if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
            return Err(self.error("expected `,` between match arms"));
        }
        Ok(arm)
    }

    fn expression_list(
        &mut self,
        closing: TokenKind,
        missing_closing_message: &'static str,
        allow_trailing_comma: bool,
    ) -> (Vec<Expr>, Span) {
        let target_depth = self.delimiter_depth_before(self.pos);
        let mut expressions = Vec::new();
        loop {
            let depth = self.delimiter_depth_before(self.pos);
            if self.at(&closing) && depth == target_depth {
                return (expressions, self.bump().span);
            }
            if self.is_expression_list_boundary(&closing, depth, target_depth) {
                self.record_missing(Diagnostic::new(
                    missing_closing_message,
                    self.current().span,
                ));
                return (expressions, self.previous().span);
            }

            let item_start = self.pos;
            let parsed = self.expression(0);
            if let Some(expression) =
                self.recover_expression_list_item(parsed, item_start, target_depth, &closing)
            {
                expressions.push(expression);
                if self.eat(&TokenKind::Comma).is_some() {
                    if !allow_trailing_comma
                        && self.at(&closing)
                        && self.delimiter_depth_before(self.pos) == target_depth
                    {
                        self.record_missing(Diagnostic::new(
                            "expected an expression after `,`",
                            self.current().span,
                        ));
                    }
                    continue;
                }
                if self.at(&closing) && self.delimiter_depth_before(self.pos) == target_depth {
                    continue;
                }
                self.record_missing(Diagnostic::new(
                    "expected `,` between expressions",
                    self.current().span,
                ));
                if self.is_expression_start() {
                    continue;
                }
                self.synchronize_expression_list_item(target_depth, &closing);
            }
        }
    }

    fn recover_expression_list_item(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        item_start: usize,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) -> Option<Expr> {
        match parsed {
            Ok(expression) => Some(expression),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[item_start].span.start;
                self.synchronize_expression_list_item(target_depth, closing);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    fn synchronize_expression_list_item(
        &mut self,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) {
        let mut depth = self.delimiter_depth_before(self.pos);
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.is_expression_list_boundary(closing, depth, target_depth)
            {
                return;
            }
            let kind = self.bump().kind.clone();
            if kind == TokenKind::Comma && depth == target_depth {
                return;
            }
            depth.update(&kind);
        }
    }

    fn is_expression_list_boundary(
        &self,
        closing: &TokenKind,
        depth: DelimiterDepth,
        target: DelimiterDepth,
    ) -> bool {
        if self.at(&TokenKind::Eof) || (self.at(&TokenKind::Semicolon) && depth == target) {
            return true;
        }
        match self.current().kind {
            TokenKind::RParen => {
                *closing != TokenKind::RParen && depth.parentheses <= target.parentheses
            }
            TokenKind::RBracket => {
                *closing != TokenKind::RBracket && depth.brackets <= target.brackets
            }
            TokenKind::RBrace => depth.braces <= target.braces,
            _ => false,
        }
    }

    fn delimiter_depth_before(&self, position: usize) -> DelimiterDepth {
        self.tokens[..position].iter().fold(
            DelimiterDepth {
                parentheses: 0,
                brackets: 0,
                braces: 0,
            },
            |mut depth, token| {
                depth.update(&token.kind);
                depth
            },
        )
    }

    fn if_expression(&mut self, start: Span) -> Result<Expr, Diagnostic> {
        let condition = self.required_expression(0)?;
        let then_expr = self.braced_expression("expected `{` after the `if` condition")?;
        let else_expr = if self.eat_ident("else").is_none() {
            let error = Diagnostic::new(
                "an `if` expression needs an `else` branch",
                self.current().span,
            );
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            self.new_expr(ExprKind::Error, span)
        } else if self.eat_ident("if").is_some() {
            let nested_start = self.previous().span;
            self.if_expression(nested_start)?
        } else {
            self.braced_expression("expected `{` after `else`")?
        };
        let span = start.join(else_expr.span);
        Ok(self.new_expr(
            ExprKind::If {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            span,
        ))
    }

    fn braced_expression(&mut self, message: &'static str) -> Result<Expr, Diagnostic> {
        let Some(start) = self.eat(&TokenKind::LBrace) else {
            let error = self.error(message);
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            return Ok(self.new_expr(ExprKind::Error, span));
        };
        let target_depth = self.delimiter_depth_before(self.pos);
        let expression_start = self.pos;
        let parsed = self.expression(0);
        let mut expression = match parsed {
            Ok(expression) => expression,
            Err(error) => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[expression_start].span.start;
                self.synchronize_delimited_expression(&TokenKind::RBrace, target_depth);
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                self.new_expr(ExprKind::Error, span)
            }
        };
        let end = if let Some(end) = self.eat(&TokenKind::RBrace) {
            end
        } else {
            let error = self.error("expected `}` after the branch expression");
            self.record_missing(error);
            let skipped_start = self.current().span.start;
            self.synchronize_delimited_expression(&TokenKind::RBrace, target_depth);
            self.record_error_region(skipped_start, self.current().span.start);
            self.eat(&TokenKind::RBrace).unwrap_or(expression.span)
        };
        expression.span = start.join(end);
        Ok(expression)
    }

    fn binary_operator(&self) -> Option<(u8, BinaryOp)> {
        Some(match self.current().kind {
            TokenKind::OrOr => (1, BinaryOp::Or),
            TokenKind::AndAnd => (2, BinaryOp::And),
            TokenKind::EqEq => (3, BinaryOp::Eq),
            TokenKind::BangEq => (3, BinaryOp::Ne),
            TokenKind::Lt => (3, BinaryOp::Lt),
            TokenKind::Le => (3, BinaryOp::Le),
            TokenKind::Gt => (3, BinaryOp::Gt),
            TokenKind::Ge => (3, BinaryOp::Ge),
            TokenKind::Or => (4, BinaryOp::BitOr),
            TokenKind::Caret => (5, BinaryOp::BitXor),
            TokenKind::And => (6, BinaryOp::BitAnd),
            TokenKind::Shl => (7, BinaryOp::Shl),
            TokenKind::Shr => (7, BinaryOp::Shr),
            TokenKind::Plus => (8, BinaryOp::Add),
            TokenKind::Minus => (8, BinaryOp::Sub),
            TokenKind::Star => (9, BinaryOp::Mul),
            TokenKind::Slash => (9, BinaryOp::Div),
            TokenKind::Percent => (9, BinaryOp::Rem),
            _ => return None,
        })
    }

    fn resolve_type(&mut self, name: &str, span: Span) -> Result<TypeRef, Diagnostic> {
        if let Some(core) = self.named_types.core(name) {
            return Ok(core);
        }
        if !self.named_types.contains(name) {
            return Err(Diagnostic::new(format!("unknown type `{name}`"), span));
        }
        let id = if let Some(id) = self.type_name_ids.get(name).copied() {
            id
        } else {
            let id = TypeNameId::from_index(self.type_names.len() as u32);
            self.type_names.push(name.to_owned());
            self.type_name_ids.insert(name.to_owned(), id);
            id
        };
        Ok(TypeRef::Named(id))
    }

    fn parse_type(&mut self, message: &'static str) -> Result<(TypeRef, Span), Diagnostic> {
        let (mut name, start) = self.expect_any_ident(message)?;
        if name == "string" {
            self.record_string_type_diagnostic(start);
            name = "String".to_owned();
        } else if name == "TimeSpan" {
            self.record_duration_type_diagnostic(start);
            name = "Duration".to_owned();
        } else if let Some(replacement) = csharp_numeric_type(&name) {
            self.record_numeric_type_diagnostic(start, &name, replacement);
            name = replacement.to_owned();
        }
        let (mut ty, mut end) = if name == "Array" {
            self.expect(TokenKind::Lt, "expected `<` after `Array`")?;
            let (element, _) = self.parse_type("expected an array element type")?;
            let end = self.expect(TokenKind::Gt, "expected `>` after the array element type")?;
            let id = if let Some(&id) = self.array_type_ids.get(&element) {
                id
            } else {
                let id = ArrayTypeId::from_index(self.next_constructed_type_id);
                self.next_constructed_type_id += 1;
                self.array_types.push(ArrayTypeDecl { id, element });
                self.array_type_ids.insert(element, id);
                id
            };
            (TypeRef::Array(id), end)
        } else {
            (self.resolve_type(&name, start)?, start)
        };

        if let Some(suffix) = self.eat(&TokenKind::Question) {
            let id = if let Some(&id) = self.option_type_ids.get(&ty) {
                id
            } else {
                let id = OptionTypeId::from_index(self.next_constructed_type_id);
                self.next_constructed_type_id += 1;
                self.option_types.push(OptionTypeDecl { id, value: ty });
                self.option_type_ids.insert(ty, id);
                id
            };
            ty = TypeRef::Option(id);
            end = suffix;
        } else if let Some(suffix) = self.eat(&TokenKind::Bang) {
            let id = if let Some(&id) = self.result_type_ids.get(&ty) {
                id
            } else {
                let id = ResultTypeId::from_index(self.next_constructed_type_id);
                self.next_constructed_type_id += 1;
                self.result_types.push(ResultTypeDecl { id, value: ty });
                self.result_type_ids.insert(ty, id);
                id
            };
            ty = TypeRef::Result(id);
            end = suffix;
        }

        if self.at(&TokenKind::Question) || self.at(&TokenKind::Bang) {
            let repeated = self.current().span;
            return Err(self
                .error(
                    "repeated optional/result postfixes are not supported; use only one `?` or `!`",
                )
                .with_primary_label("this second wrapper postfix is not allowed")
                .with_note("a type can be optional or fallible, but wrapper postfixes cannot be combined or repeated")
                .with_machine_applicable_fix("remove the repeated postfix", repeated, ""));
        }

        Ok((ty, start.join(end)))
    }

    fn terminator(&mut self) -> Result<(), Diagnostic> {
        if self.eat(&TokenKind::Semicolon).is_some()
            || self.at(&TokenKind::RBrace)
            || self.at(&TokenKind::Eof)
            || self.line_break_before_current()
        {
            Ok(())
        } else {
            Err(self.error("expected `;` or a line break after the statement"))
        }
    }

    fn line_break_before_current(&self) -> bool {
        let previous_end = self.previous().span.end;
        let current_start = self.current().span.start;
        self.source[previous_end.min(self.source.len())..current_start.min(self.source.len())]
            .contains(['\n', '\r'])
    }

    fn expect_u64(&mut self, message: &'static str) -> Result<u64, Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::Int(text) = &token.kind {
            let (value, suffix) =
                parse_integer(text).map_err(|error| Diagnostic::new(error, token.span))?;
            if suffix.is_some_and(|ty| !ty.is_integer()) {
                return Err(Diagnostic::new(message, token.span));
            }
            self.bump();
            Ok(value)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    fn expect_bool(&mut self, message: &'static str) -> Result<bool, Diagnostic> {
        let token = self.current().clone();
        match &token.kind {
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                Ok(true)
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                Ok(false)
            }
            _ => Err(Diagnostic::new(message, token.span)),
        }
    }

    fn expect_string(&mut self, message: &'static str) -> Result<String, Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::String(value) = token.kind {
            self.bump();
            Ok(value)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    fn expect_ident(&mut self, expected: &'static str) -> Result<Span, Diagnostic> {
        let token = self.current().clone();
        if matches!(&token.kind, TokenKind::Ident(name) if name == expected) {
            self.bump();
            Ok(token.span)
        } else {
            Err(Diagnostic::new(
                format!("expected `{expected}`"),
                token.span,
            ))
        }
    }

    fn expect_any_ident(&mut self, message: &'static str) -> Result<(String, Span), Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::Ident(name) = token.kind {
            self.bump();
            Ok((name, token.span))
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &'static str) -> Result<Span, Diagnostic> {
        let token = self.current().clone();
        if token.kind == kind {
            self.bump();
            Ok(token.span)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    fn eat_ident(&mut self, expected: &str) -> Option<Span> {
        if self.at_ident(expected) {
            Some(self.bump().span)
        } else {
            None
        }
    }

    fn at_ident(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Ident(name) if name == expected)
    }

    fn eat(&mut self, kind: &TokenKind) -> Option<Span> {
        if self.at(kind) {
            Some(self.bump().span)
        } else {
            None
        }
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.current().kind == *kind
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.pos.saturating_sub(1)]
    }

    fn peek(&self, offset: usize) -> &Token {
        &self.tokens[(self.pos + offset).min(self.tokens.len() - 1)]
    }

    fn bump(&mut self) -> &Token {
        let index = self.pos;
        if !matches!(self.tokens[index].kind, TokenKind::Eof) {
            self.pos += 1;
        }
        &self.tokens[index]
    }

    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(message, self.current().span)
    }

    fn record_let_keyword_diagnostic(&mut self) {
        let span = self.current().span;
        let TokenKind::Ident(keyword) = &self.current().kind else {
            unreachable!("the familiar declaration keyword is an identifier")
        };
        let keyword = keyword.clone();
        self.diagnostics.push(
            Diagnostic::new(
                format!("SplitScript uses `let` instead of `{keyword}` for variable declarations"),
                span,
            )
            .with_primary_label("replace this familiar declaration keyword")
            .with_machine_applicable_fix(
                format!("replace `{keyword}` with `let`"),
                span,
                "let",
            ),
        );
    }

    fn record_none_keyword_diagnostic(&mut self, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "SplitScript uses `None` instead of `null` for absent optional values",
                span,
            )
            .with_primary_label("replace this JavaScript-style value")
            .with_machine_applicable_fix("replace `null` with `None`", span, "None"),
        );
    }

    fn record_fn_keyword_diagnostic(&mut self) {
        let span = self.current().span;
        let TokenKind::Ident(keyword) = &self.current().kind else {
            unreachable!("the familiar function keyword is an identifier")
        };
        let keyword = keyword.clone();
        self.diagnostics.push(
            Diagnostic::new(
                format!("SplitScript uses `fn` instead of `{keyword}` for functions"),
                span,
            )
            .with_primary_label("replace this familiar function keyword")
            .with_machine_applicable_fix(
                format!("replace `{keyword}` with `fn`"),
                span,
                "fn",
            ),
        );
    }

    fn record_string_type_diagnostic(&mut self, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "SplitScript uses `String` instead of `string` for the string type",
                span,
            )
            .with_primary_label("type names are case-sensitive")
            .with_machine_applicable_fix(
                "replace `string` with `String`",
                span,
                "String",
            ),
        );
    }

    fn record_duration_type_diagnostic(&mut self, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "SplitScript uses `Duration` instead of `TimeSpan` for timer durations",
                span,
            )
            .with_primary_label("replace this C# type name")
            .with_machine_applicable_fix(
                "replace `TimeSpan` with `Duration`",
                span,
                "Duration",
            ),
        );
    }

    fn record_numeric_type_diagnostic(
        &mut self,
        span: Span,
        csharp_name: &str,
        splitscript_name: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::new(
                format!(
                    "SplitScript uses `{splitscript_name}` instead of `{csharp_name}` for this numeric type"
                ),
                span,
            )
            .with_primary_label("replace this C# numeric type name")
            .with_machine_applicable_fix(
                format!("replace `{csharp_name}` with `{splitscript_name}`"),
                span,
                splitscript_name,
            ),
        );
    }

    fn current_action_kind(&self) -> Option<ActionKind> {
        let TokenKind::Ident(name) = &self.current().kind else {
            return None;
        };
        ActionKind::parse(name)
    }

    fn is_top_level_start(&self) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::Ident(name)
                if matches!(name.as_str(), "state" | "settings" | "let" | "const" | "var" | "debug" | "fn" | "func" | "function" | "record" | "enum")
                    || ActionKind::parse(name).is_some()
        )
    }

    fn synchronize_top_level(&mut self, declaration_start: usize) {
        let mut brace_depth = self.brace_depth_before(self.pos);

        if self.pos > declaration_start && brace_depth == 0 && self.is_top_level_start() {
            return;
        }

        while !self.at(&TokenKind::Eof) {
            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
            if brace_depth == 0 && self.is_top_level_start() {
                return;
            }
        }
    }

    fn synchronize_statement(&mut self, statement_start: usize, block_depth: u32) {
        let mut brace_depth = self.brace_depth_before(self.pos);
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::RBrace) && brace_depth == block_depth)
            {
                return;
            }
            if self.pos > statement_start
                && brace_depth == block_depth
                && self.line_break_before_current()
                && self.is_statement_start()
            {
                return;
            }

            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::Semicolon if brace_depth == block_depth => return,
                _ => {}
            }
        }
    }

    fn recover_delimited_item<T>(
        &mut self,
        parsed: Result<T, Diagnostic>,
        item_start: usize,
        body_depth: u32,
    ) -> Option<T> {
        match parsed {
            Ok(item) => Some(item),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[item_start].span.start;
                self.synchronize_delimited_item(item_start, body_depth);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    fn recover_parameter<T>(
        &mut self,
        parsed: Result<T, Diagnostic>,
        item_start: usize,
    ) -> Option<T> {
        match parsed {
            Ok(parameter) => Some(parameter),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.tokens[item_start].span.start;
                self.synchronize_parameter(item_start);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    fn record_recovery_diagnostic(&mut self, error: Diagnostic) {
        if error.message.starts_with("expected") {
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Missing,
                span: Span {
                    start: error.span.start,
                    end: error.span.start,
                },
            });
        }
        self.diagnostics.push(error);
    }

    fn record_error_region(&mut self, start: usize, end: usize) {
        let end = end.max(start);
        if end != start {
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Error,
                span: Span { start, end },
            });
        }
    }

    fn synchronize_parameter(&mut self, item_start: usize) {
        loop {
            if self.at(&TokenKind::Eof)
                || self.at(&TokenKind::RParen)
                || self.at(&TokenKind::LBrace)
                || self.at(&TokenKind::Minus)
            {
                return;
            }
            if self.pos > item_start
                && self.line_break_before_current()
                && matches!(self.current().kind, TokenKind::Ident(_))
            {
                return;
            }
            if matches!(self.bump().kind, TokenKind::Comma) {
                return;
            }
        }
    }

    fn synchronize_delimited_item(&mut self, item_start: usize, body_depth: u32) {
        let mut brace_depth = self.brace_depth_before(self.pos);
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::RBrace) && brace_depth == body_depth)
            {
                return;
            }
            if self.pos > item_start
                && brace_depth == body_depth
                && self.line_break_before_current()
                && matches!(
                    self.current().kind,
                    TokenKind::Ident(_)
                        | TokenKind::Int(_)
                        | TokenKind::String(_)
                        | TokenKind::DocComment(_)
                )
            {
                return;
            }

            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::Comma | TokenKind::Semicolon if brace_depth == body_depth => return,
                _ => {}
            }
        }
    }

    fn record_missing_closing(&mut self, message: &'static str) {
        let error = self.error(message);
        self.record_missing(error);
    }

    fn record_missing(&mut self, error: Diagnostic) {
        let position = error.span.start;
        self.diagnostics.push(error);
        self.recovery_nodes.push(RecoveryNode {
            kind: RecoveryNodeKind::Missing,
            span: Span {
                start: position,
                end: position,
            },
        });
    }

    fn brace_depth_before(&self, position: usize) -> u32 {
        self.tokens[..position]
            .iter()
            .fold(0u32, |depth, token| match token.kind {
                TokenKind::LBrace => depth + 1,
                TokenKind::RBrace => depth.saturating_sub(1),
                _ => depth,
            })
    }

    fn is_statement_start(&self) -> bool {
        match &self.current().kind {
            TokenKind::Ident(name) => name != "else",
            _ => self.is_expression_start(),
        }
    }

    fn is_expression_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Ident(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::TemplateStart
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::Bang
                | TokenKind::Minus
        )
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Debug { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
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

fn type_can_be_stored_in_state(ty: TypeRef) -> bool {
    ty != TypeRef::Void
        && ty.standard_type().is_none_or(|standard| {
            StandardLibrary::new()
                .type_decl(standard)
                .value_usage
                .state_field
        })
}

#[derive(Clone, Copy)]
enum NamedTypeDeclaration {
    Core(TypeRef),
    Record(RecordId),
    Enum(EnumTypeId),
    RecordAndEnum(RecordId, EnumTypeId),
    Standard,
}

#[derive(Default)]
struct NamedTypeEnvironment {
    declarations: HashMap<String, NamedTypeDeclaration>,
}

impl NamedTypeEnvironment {
    fn core(&self, name: &str) -> Option<TypeRef> {
        match self.declarations.get(name) {
            Some(NamedTypeDeclaration::Core(ty)) => Some(*ty),
            _ => None,
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.declarations.contains_key(name)
    }

    fn record(&self, name: &str) -> Option<RecordId> {
        match self.declarations.get(name) {
            Some(NamedTypeDeclaration::Record(id))
            | Some(NamedTypeDeclaration::RecordAndEnum(id, _)) => Some(*id),
            _ => None,
        }
    }

    fn enumeration(&self, name: &str) -> Option<EnumTypeId> {
        match self.declarations.get(name) {
            Some(NamedTypeDeclaration::Enum(id))
            | Some(NamedTypeDeclaration::RecordAndEnum(_, id)) => Some(*id),
            _ => None,
        }
    }

    fn source_type_count(&self) -> usize {
        self.declarations
            .values()
            .map(|declaration| match declaration {
                NamedTypeDeclaration::Record(_)
                | NamedTypeDeclaration::Enum(EnumTypeId::Source(_)) => 1,
                NamedTypeDeclaration::RecordAndEnum(_, EnumTypeId::Source(_)) => 2,
                NamedTypeDeclaration::RecordAndEnum(_, EnumTypeId::Standard(_)) => 1,
                NamedTypeDeclaration::Enum(EnumTypeId::Standard(_))
                | NamedTypeDeclaration::Core(_)
                | NamedTypeDeclaration::Standard => 0,
            })
            .sum()
    }
}

fn collect_named_types(tokens: &[Token]) -> (NamedTypeEnvironment, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let records = collect_top_level_names(tokens, "record", 0)
        .into_iter()
        .map(|(name, id)| (name, RecordId::from_index(id)))
        .collect::<HashMap<_, _>>();
    let enums = collect_top_level_names(tokens, "enum", records.len() as u32)
        .into_iter()
        .map(|(name, id)| (name, EnumTypeId::Source(EnumId::from_index(id))))
        .collect::<HashMap<_, _>>();
    let mut environment = NamedTypeEnvironment::default();
    for core in StandardLibrary::new().core_types() {
        environment.declarations.insert(
            core.name.to_owned(),
            NamedTypeDeclaration::Core(
                TypeRef::parse(core.name).expect("every declared core type has source syntax"),
            ),
        );
    }
    environment.declarations.insert(
        "Address".to_owned(),
        NamedTypeDeclaration::Core(TypeRef::Address),
    );
    for (name, id) in &records {
        if environment.contains(name) {
            diagnostics.push(Diagnostic::new(
                format!("`{name}` is a core type and cannot be redeclared as a record"),
                Span::default(),
            ));
        }
        environment
            .declarations
            .insert(name.clone(), NamedTypeDeclaration::Record(*id));
    }
    for (name, id) in &enums {
        if let Some(NamedTypeDeclaration::Record(record)) =
            environment.declarations.get(name).copied()
        {
            diagnostics.push(Diagnostic::new(
                format!("duplicate named type `{name}`"),
                Span::default(),
            ));
            environment.declarations.insert(
                name.clone(),
                NamedTypeDeclaration::RecordAndEnum(record, *id),
            );
        } else {
            if environment.contains(name) {
                diagnostics.push(Diagnostic::new(
                    format!("`{name}` is a core type and cannot be redeclared as an enum"),
                    Span::default(),
                ));
            }
            environment
                .declarations
                .insert(name.clone(), NamedTypeDeclaration::Enum(*id));
        }
    }
    for ty in StandardLibrary::new().types() {
        if records.contains_key(ty.name) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "`{}` is a standard-library type and cannot be redeclared as a record",
                    ty.name
                ),
                Span::default(),
            ));
        }
        if enums.contains_key(ty.name) {
            let kind = if ty.kind == StdlibTypeKind::Enum {
                "enum"
            } else {
                "type"
            };
            diagnostics.push(Diagnostic::new(
                format!(
                    "`{}` is a standard-library {kind} and cannot be redeclared as an enum",
                    ty.name,
                ),
                Span::default(),
            ));
        }
        if !environment.contains(ty.name) {
            let declaration = if ty.kind == StdlibTypeKind::Enum {
                NamedTypeDeclaration::Enum(EnumTypeId::Standard(ty.id))
            } else {
                NamedTypeDeclaration::Standard
            };
            environment
                .declarations
                .insert(ty.name.to_owned(), declaration);
        }
    }
    (environment, diagnostics)
}

fn collect_top_level_names(
    tokens: &[Token],
    keyword_to_find: &str,
    first_id: u32,
) -> HashMap<String, u32> {
    let mut names = HashMap::new();
    let mut brace_depth = 0u32;
    for (index, token) in tokens.iter().enumerate() {
        if brace_depth == 0
            && matches!(&token.kind, TokenKind::Ident(keyword) if keyword == keyword_to_find)
            && let Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) = tokens.get(index + 1)
        {
            let next_id = first_id + names.len() as u32;
            names.entry(name.clone()).or_insert(next_id);
        }
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
    }
    names
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

fn parse_integer(text: &str) -> Result<(u64, Option<TypeRef>), String> {
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
    use crate::lexer;

    use super::*;

    #[test]
    fn parses_domain_shaped_autosplitter() {
        let source = r#"
            state "game.exe" {
                level: u32 at "game.exe", 0x1234, 0x20
            }
            settings { splitLevels: bool = true, "Split levels" }
            split {
                let changed = current.level != old.level;
                return settings.splitLevels && changed;
            }
        "#;
        let program = parse(source, lexer::lex(source).unwrap()).unwrap();
        assert_eq!(
            program.state.unwrap().fields[0].annotation,
            Some(TypeRef::U32)
        );
        assert_eq!(program.actions[0].kind, ActionKind::Split);
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
        let program = parse(source, lexer::lex(source).unwrap()).unwrap();
        assert_eq!(
            program.settings[0].tooltip.as_deref(),
            Some("First line of the tooltip continues on this line.\nA second paragraph.")
        );
    }
}
