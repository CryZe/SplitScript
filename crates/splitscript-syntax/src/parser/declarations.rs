//! Top-level declarations and the state/settings domain grammars.

//! Declaration grammar.

use super::{
    Action, ActionKind, CoreTypeId, Diagnostic, EnumDecl, EnumId, EnumReference, EnumVariant,
    FunctionDecl, FunctionId, Parameter, Parser, PointerPath, RecordDecl, RecordField, RecordId,
    SettingChoiceOption, SettingDecl, SettingFileFilter, SettingKind, Span, StateDecl, StateField,
    StateLayoutDecl, StateMemoryDecoder, StateProviderRef, StateSource, TokenKind, TypeRef,
    csharp_numeric_type,
};

impl Parser<'_> {
    pub(super) fn enum_decl(&mut self) -> Result<EnumDecl, Diagnostic> {
        let start = self.expect_ident("enum")?.start;
        let (name, name_span) = self.expect_any_ident("expected an enum name")?;
        let id = EnumId::from_index(self.next_enum_id);
        self.next_enum_id += 1;
        self.expect(TokenKind::LBrace, "expected `{` after the enum name")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated enum declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede an enum variant"));
                break;
            }
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
                    name_span: variant_span,
                    documentation,
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
            documentation: None,
            name_span,
            variants,
            span: Span { start, end },
        })
    }

    pub(super) fn record_decl(&mut self) -> Result<RecordDecl, Diagnostic> {
        let start = self.expect_ident("record")?.start;
        let (name, name_span) = self.expect_any_ident("expected a record name")?;
        let id = RecordId::from_index(self.next_record_id);
        self.next_record_id += 1;
        self.expect(TokenKind::LBrace, "expected `{` after the record name")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated record declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede a record field"));
                break;
            }
            let parsed = (|| {
                let (field_name, field_start) = self.expect_any_ident("expected a field name")?;
                self.expect(TokenKind::Colon, "expected `:` after the field name")?;
                let (ty, type_span) = self.parse_type("expected a field type")?;
                let field = RecordField {
                    id: self.new_record_field_id(),
                    name: field_name,
                    name_span: field_start,
                    documentation,
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
            documentation: None,
            name_span,
            fields,
            span: Span { start, end },
        })
    }

    pub(super) fn function_decl(&mut self) -> Result<FunctionDecl, Diagnostic> {
        let id = FunctionId::from_index(self.next_function_id);
        self.next_function_id += 1;
        let start = self.bump().span.start;
        let (first_name, first_span) = self.expect_any_ident("expected a function name")?;
        let (method_of, name, name_span) = if self.eat(&TokenKind::Dot).is_some() {
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
            let (method, method_span) =
                self.expect_any_ident("expected a method name after `.`")?;
            (Some(receiver), method, method_span)
        } else {
            (None, first_name, first_span)
        };
        self.expect(TokenKind::LParen, "expected `(` after the function name")?;
        let mut params = method_of.map_or_else(Vec::new, |ty| {
            vec![Parameter {
                id: self.new_value_id(),
                name: "self".to_owned(),
                name_span: first_span,
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
            let item_start = self.cursor.position();
            let parsed = (|| {
                let (param_name, param_start) =
                    self.expect_any_ident("expected a parameter name")?;
                let (annotation, type_span) = if self.eat(&TokenKind::Colon).is_some() {
                    let (ty, span) = self.parse_type("expected a parameter type")?;
                    (Some(ty), span)
                } else {
                    (None, param_start)
                };
                if annotation == Some(TypeRef::core(CoreTypeId::Void)) {
                    return Err(Diagnostic::new(
                        "parameters cannot have type `void`",
                        type_span,
                    ));
                }
                let parameter = Parameter {
                    id: self.new_value_id(),
                    name: param_name,
                    name_span: param_start,
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
        let (return_annotation, return_is_async, return_async_span, return_annotation_span) =
            if self.eat(&TokenKind::Minus).is_some() {
                self.expect(TokenKind::Gt, "expected `>` in the return arrow `->`")?;
                let async_span = self.eat_ident("async");
                let (ty, span) = self.parse_type("expected a return type after `async`")?;
                (Some(ty), async_span.is_some(), async_span, Some(span))
            } else {
                (None, false, None, None)
            };
        let body = self.block()?;
        let span = Span {
            start,
            end: body.span.end,
        };
        Ok(FunctionDecl {
            id,
            name,
            name_span,
            documentation: None,
            debug_only: false,
            method_of,
            params,
            return_annotation,
            return_is_async,
            return_async_span,
            return_annotation_span,
            body,
            span,
        })
    }

    pub(super) fn state_block_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        let (provider, processes) = if matches!(self.current().kind, TokenKind::Ident(_))
            && matches!(self.peek(1).kind, TokenKind::LBrace)
        {
            let (name, span) = self.expect_any_ident("expected a state provider name")?;
            (Some(StateProviderRef { name, span }), Vec::new())
        } else {
            (None, self.process_names()?)
        };
        self.expect(
            TokenKind::LBrace,
            "expected `{` after the process name list",
        )?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut fields = Vec::new();
        let mut layouts = Vec::new();
        let mut layout_variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics.push(
                    self.error("a documentation comment must precede a state field or layout"),
                );
                break;
            }
            if self.at_ident("layout") {
                let parsed = self.state_layout_decl(documentation);
                if let Some((layout, variant)) =
                    self.recover_delimited_item(parsed, item_start, body_depth)
                {
                    layouts.push(layout);
                    layout_variants.push(variant);
                }
            } else {
                let parsed = self.state_field(documentation);
                if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    fields.push(field);
                }
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        if !fields.is_empty() && !layouts.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                "a state declaration cannot mix fields and named layouts",
                Span { start, end },
            ));
        }
        let (layout_enum, layout_value) = if layouts.is_empty() {
            (None, None)
        } else {
            let id = EnumId::from_index(self.next_enum_id);
            self.next_enum_id += 1;
            let name_span = Span {
                start,
                end: start + "state".len(),
            };
            (
                Some(EnumDecl {
                    id,
                    name: "StateLayout".to_owned(),
                    documentation: Some(
                        "The memory layout selected for the attached game build.".to_owned(),
                    ),
                    name_span,
                    variants: layout_variants,
                    span: Span { start, end },
                }),
                Some(self.new_value_id()),
            )
        };
        Ok(StateDecl {
            provider,
            processes,
            fields,
            layouts,
            layout_enum,
            layout_value,
            span: Span { start, end },
        })
    }

    fn state_layout_decl(
        &mut self,
        documentation: Option<String>,
    ) -> Result<(StateLayoutDecl, EnumVariant), Diagnostic> {
        let start = self.expect_ident("layout")?.start;
        let (name, name_span) = self.expect_any_ident("expected a layout name")?;
        let variant = EnumVariant {
            id: self.new_enum_variant_id(),
            name,
            name_span,
            documentation,
            payload: None,
            span: name_span,
        };
        self.expect(TokenKind::LBrace, "expected `{` after the layout name")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state layout");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede a state field"));
                break;
            }
            let parsed = self.state_field(documentation);
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok((
            StateLayoutDecl {
                variant: variant.id,
                fields,
                span: Span { start, end },
            },
            variant,
        ))
    }

    fn state_field(&mut self, documentation: Option<String>) -> Result<StateField, Diagnostic> {
        let (name, field_start) = self.expect_any_ident("expected a state field name")?;
        let annotation = if self.eat(&TokenKind::Colon).is_some() {
            let (ty, type_span) = self.parse_type("expected a state field type")?;
            if !Self::type_can_be_stored_in_state(ty) {
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
            let decoder = if let Some(start) = self.eat_ident("as") {
                let (name, name_span) =
                    self.expect_any_ident("expected a state memory decoder after `as`")?;
                if name != "utf8" {
                    return Err(Diagnostic::new(
                        format!("unknown state memory decoder `{name}`"),
                        name_span,
                    ));
                }
                self.expect(TokenKind::LParen, "expected `(` after `utf8`")?;
                let max_bytes = self.expect_u64("expected a maximum UTF-8 byte count")?;
                let end = self
                    .expect(
                        TokenKind::RParen,
                        "expected `)` after the maximum UTF-8 byte count",
                    )?
                    .end;
                let Ok(max_bytes) = u32::try_from(max_bytes) else {
                    return Err(Diagnostic::new(
                        "the maximum UTF-8 byte count must fit in `u32`",
                        start.join(Span {
                            start: name_span.start,
                            end,
                        }),
                    ));
                };
                Some(StateMemoryDecoder::Utf8 {
                    max_bytes,
                    span: Span {
                        start: start.start,
                        end,
                    },
                })
            } else {
                None
            };
            StateSource::Pointer(PointerPath {
                module,
                offsets,
                decoder,
            })
        };
        let end = self.previous().span.end;
        self.terminator()?;
        Ok(StateField {
            id: self.new_value_id(),
            name,
            documentation,
            annotation,
            source,
            span: Span {
                start: field_start.start,
                end,
            },
        })
    }

    pub(super) fn process_names(&mut self) -> Result<Vec<String>, Diagnostic> {
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

    pub(super) fn settings_block_decl(&mut self) -> Result<Vec<SettingDecl>, Diagnostic> {
        self.expect_ident("settings")?;
        self.expect(TokenKind::LBrace, "expected `{` after `settings`")?;
        let mut settings = Vec::new();
        let mut heading_count = 0;
        self.settings_dsl_entries(&mut settings, 0, &mut heading_count)?;
        if let Err(error) = self.expect(TokenKind::RBrace, "expected `}` after settings") {
            self.record_missing(error);
        }
        Ok(settings)
    }

    pub(super) fn settings_dsl_entries(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let body_depth = self.brace_depth_before(self.cursor.position());
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated settings group");
                return Ok(());
            }
            let item_start = self.cursor.position();
            let parsed = self.settings_dsl_entry(settings, heading_level, heading_count);
            self.recover_delimited_item(parsed, item_start, body_depth);
        }
        Ok(())
    }

    pub(super) fn settings_dsl_entry(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let tooltip = self.take_doc_tooltip();
        if self.at(&TokenKind::RBrace) {
            return Err(self.error("a documentation comment must precede a setting or title"));
        }
        if matches!(self.current().kind, TokenKind::Ident(_))
            && matches!(self.peek(1).kind, TokenKind::Colon)
            && matches!(&self.peek(2).kind, TokenKind::Ident(name) if name == "bool")
            && matches!(self.peek(3).kind, TokenKind::Assign)
            && matches!(&self.peek(4).kind, TokenKind::Ident(value) if value == "true" || value == "false")
        {
            let (name, start) = self.expect_any_ident("expected a setting name")?;
            self.bump();
            self.bump();
            self.bump();
            let default = self.expect_bool("expected a boolean default value")?;
            let description = if self.eat(&TokenKind::Comma).is_some() {
                self.expect_string("expected a quoted setting label after `,`")?
            } else {
                name.clone()
            };
            let span = Span {
                start: start.start,
                end: self.previous().span.end,
            };
            return Err(Diagnostic::new(
                "legacy `name: bool = default, \"label\"` settings syntax is not supported",
                span,
            )
            .with_machine_applicable_fix(
                "rewrite this setting using the settings DSL",
                span,
                format!("{description:?} => {name}: {default}"),
            ));
        }
        let label_token = self.current().clone();
        let TokenKind::String(description) = label_token.kind else {
            return Err(Diagnostic::new(
                "expected a quoted setting label",
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

        self.expect(TokenKind::FatArrow, "expected `=>` after the setting label")?;
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

    pub(super) fn take_doc_tooltip(&mut self) -> Option<String> {
        self.take_doc_comments("\n")
    }

    pub(super) fn take_source_documentation(&mut self) -> Option<String> {
        self.take_doc_comments("\n\n")
    }

    fn take_doc_comments(&mut self, paragraph_break: &str) -> Option<String> {
        let mut tooltip = String::new();
        let mut blank_lines = 0usize;
        for line in self.cursor.take_doc_comments() {
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
                        tooltip.push_str(paragraph_break);
                    }
                }
            }
            blank_lines = 0;
            tooltip.push_str(line);
        }
        (!tooltip.is_empty()).then_some(tooltip)
    }

    pub(super) fn choice_setting(&mut self) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `choice`")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut enumeration: Option<(String, Span)> = None;
        let mut options = Vec::new();
        let mut default_variant = None;
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated choice setting");
                break;
            }
            let item_start = self.cursor.position();
            let parsed = (|| {
                let option_start = self.current().span;
                let description = self.expect_string("expected a choice option description")?;
                self.expect(
                    TokenKind::FatArrow,
                    "expected `=>` after the option description",
                )?;
                let (enum_name, enum_span) = self.expect_any_ident("expected an enum name")?;
                if enumeration
                    .as_ref()
                    .is_some_and(|(previous, _)| previous != &enum_name)
                {
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
                Ok((enum_name, enum_span, description, variant, is_default, span))
            })();
            if let Some((enum_name, enum_span, description, variant, is_default, span)) =
                self.recover_delimited_item(parsed, item_start, body_depth)
            {
                enumeration.get_or_insert((enum_name, enum_span));
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
        let Some((enum_name, enum_span)) = enumeration else {
            return Err(self.error("a choice needs at least one option"));
        };
        let default_variant = default_variant.unwrap_or_else(|| options[0].variant.clone());
        Ok(SettingKind::Choice {
            enumeration: EnumReference {
                name: enum_name,
                span: enum_span,
            },
            default_variant,
            options,
        })
    }

    pub(super) fn file_setting(&mut self) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `file`")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut filters = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated file setting");
                break;
            }
            let item_start = self.cursor.position();
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

    pub(super) fn action_block(&mut self) -> Result<Action, Diagnostic> {
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

    pub(super) fn state_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        self.expect(TokenKind::LParen, "expected `(` after `state`")?;
        let process = self.expect_string("expected a process name string")?;
        self.expect(TokenKind::Comma, "expected `,` after the process name")?;
        self.expect(TokenKind::LBrace, "expected a state object")?;
        let body_depth = self.brace_depth_before(self.cursor.position());
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state object");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede a state field"));
                break;
            }
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
                if !Self::type_can_be_stored_in_state(ty) {
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
                    documentation,
                    annotation: Some(ty),
                    source: StateSource::Pointer(PointerPath {
                        module,
                        offsets,
                        decoder: None,
                    }),
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
            provider: None,
            processes: vec![process],
            fields,
            layouts: Vec::new(),
            layout_enum: None,
            layout_value: None,
            span: Span {
                start,
                end: self.previous().span.end,
            },
        })
    }
}
