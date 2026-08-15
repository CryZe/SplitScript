//! Source type-expression parsing and constructed syntax-type interning.

use super::{
    ArrayTypeDecl, AsyncTypeDecl, Diagnostic, OptionTypeDecl, Parser, ResultTypeDecl, Span,
    TokenKind, TypeApplicationDecl, TypeApplicationOccurrence, TypeNameId, TypeRef,
};
use crate::migration::ForeignSpellingContext;

impl Parser<'_> {
    pub(super) fn resolve_type(&mut self, name: &str, span: Span) -> Result<TypeRef, Diagnostic> {
        if let Some(core) = TypeRef::parse(name) {
            return Ok(core);
        }
        let id = if let Some(id) = self.type_name_ids.get(name).copied() {
            self.type_name_occurrences[id.index()].push(span);
            id
        } else {
            let id = TypeNameId::from_index(self.type_names.len() as u32);
            self.type_names.push(name.to_owned());
            self.type_name_spans.push(span);
            self.type_name_occurrences.push(vec![span]);
            self.type_name_ids.insert(name.to_owned(), id);
            id
        };
        Ok(TypeRef::Named(id))
    }

    pub(super) fn parse_type(
        &mut self,
        message: &'static str,
    ) -> Result<(TypeRef, Span), Diagnostic> {
        self.parse_type_with_assignment_boundary(message, false)
    }

    pub(super) fn parse_type_before_assignment(
        &mut self,
        message: &'static str,
    ) -> Result<(TypeRef, Span), Diagnostic> {
        self.parse_type_with_assignment_boundary(message, true)
    }

    fn parse_type_with_assignment_boundary(
        &mut self,
        message: &'static str,
        allow_joined_result_suffix: bool,
    ) -> Result<(TypeRef, Span), Diagnostic> {
        if let Some(start) = self.eat_ident("async") {
            let (value, end) = self.parse_type_with_assignment_boundary(
                "expected a type after `async`",
                allow_joined_result_suffix,
            )?;
            if matches!(value, TypeRef::Async(_)) {
                return Err(Diagnostic::new(
                    "an asynchronous value cannot be wrapped in `async` again",
                    start.join(end),
                ));
            }
            let id = if let Some(&id) = self.async_type_ids.get(&value) {
                id
            } else {
                let id = self.constructed_type_ids.async_value();
                self.async_types.push(AsyncTypeDecl { id, value });
                self.async_type_ids.insert(value, id);
                id
            };
            return Ok((TypeRef::Async(id), start.join(end)));
        }
        let (mut ty, start, mut end) = if let Some(start) = self.eat(&TokenKind::LBracket) {
            let (element, _) = self.parse_type("expected an array element type")?;
            let length = if self.eat(&TokenKind::Semicolon).is_some() {
                let value = self.expect_u64("expected a fixed array length after `;`")?;
                Some(u32::try_from(value).map_err(|_| {
                    Diagnostic::new(
                        "a fixed array length must fit in `u32`",
                        self.previous().span,
                    )
                })?)
            } else {
                None
            };
            let end = self.expect(TokenKind::RBracket, "expected `]` after the array type")?;
            let key = (element, length);
            let id = if let Some(&id) = self.array_type_ids.get(&key) {
                id
            } else {
                let id = self.constructed_type_ids.array();
                self.array_types.push(ArrayTypeDecl {
                    id,
                    element,
                    length,
                });
                self.array_type_ids.insert(key, id);
                id
            };
            (TypeRef::Array(id), start, end)
        } else {
            let (mut name, start) = self.expect_any_ident(message)?;
            if let Some(replacement) =
                self.record_foreign_spelling_diagnostic(start, &name, ForeignSpellingContext::Type)
            {
                name = replacement.to_owned();
            }
            let named = self.resolve_type(&name, start)?;
            if let Some(opening) = self.eat(&TokenKind::Lt) {
                let TypeRef::Named(constructor) = named else {
                    return Err(Diagnostic::new(
                        "core types cannot be used as generic type constructors",
                        start,
                    ));
                };
                let mut arguments = Vec::new();
                loop {
                    let (argument, _) = self.parse_type("expected a generic type argument")?;
                    arguments.push(argument);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                    if self.at_generic_close() {
                        break;
                    }
                }
                let end = self.expect_generic_close("expected `>` after generic type arguments")?;
                let occurrence = TypeApplicationOccurrence {
                    span: start.join(end),
                    constructor: start,
                    opening,
                    closing: end,
                };
                let key = (constructor, arguments.clone());
                let id = if let Some(&id) = self.type_application_ids.get(&key) {
                    self.type_applications
                        .iter_mut()
                        .find(|application| application.id == id)
                        .expect("interned type applications retain their declaration")
                        .occurrences
                        .push(occurrence);
                    id
                } else {
                    let id = self.constructed_type_ids.application();
                    self.type_applications.push(TypeApplicationDecl {
                        id,
                        constructor,
                        arguments,
                        occurrences: vec![occurrence],
                    });
                    self.type_application_ids.insert(key, id);
                    id
                };
                (TypeRef::Application(id), start, end)
            } else {
                (named, start, start)
            }
        };

        let mut previous_suffix = None;
        loop {
            let (suffix, is_option) = if let Some(suffix) = self.eat(&TokenKind::Question) {
                (suffix, true)
            } else if let Some(suffix) = self.eat(&TokenKind::Bang).or_else(|| {
                allow_joined_result_suffix
                    .then(|| self.eat_fallible_type_suffix())
                    .flatten()
            }) {
                (suffix, false)
            } else {
                break;
            };

            if previous_suffix == Some(is_option) {
                let spelling = if is_option { "?" } else { "!" };
                return Err(Diagnostic::new(
                    format!("a type cannot have two adjacent `{spelling}` wrappers"),
                    suffix,
                )
                .with_primary_label("this wrapper duplicates the preceding wrapper")
                .with_note("mixed wrappers are valid: `T!?` is an optional result and `T?!` is a fallible option")
                .with_machine_applicable_fix("remove the duplicate wrapper", suffix, ""));
            }

            ty = if is_option {
                TypeRef::Option(self.intern_option_type(ty, suffix))
            } else {
                TypeRef::Result(self.intern_result_type(ty, suffix))
            };
            end = suffix;
            previous_suffix = Some(is_option);
        }

        Ok((ty, start.join(end)))
    }

    fn intern_option_type(&mut self, value: TypeRef, occurrence: Span) -> super::OptionTypeId {
        if let Some(&id) = self.option_type_ids.get(&value) {
            self.option_types
                .iter_mut()
                .find(|option| option.id == id)
                .expect("interned option types retain their declaration")
                .occurrences
                .push(occurrence);
            id
        } else {
            let id = self.constructed_type_ids.option();
            self.option_types.push(OptionTypeDecl {
                id,
                value,
                occurrences: vec![occurrence],
            });
            self.option_type_ids.insert(value, id);
            id
        }
    }

    fn intern_result_type(&mut self, value: TypeRef, occurrence: Span) -> super::ResultTypeId {
        if let Some(&id) = self.result_type_ids.get(&value) {
            self.result_types
                .iter_mut()
                .find(|result| result.id == id)
                .expect("interned result types retain their declaration")
                .occurrences
                .push(occurrence);
            id
        } else {
            let id = self.constructed_type_ids.result();
            self.result_types.push(ResultTypeDecl {
                id,
                value,
                occurrences: vec![occurrence],
            });
            self.result_type_ids.insert(value, id);
            id
        }
    }
}
