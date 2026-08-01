//! Source type-expression parsing and constructed syntax-type interning.

use super::{
    ArrayTypeDecl, Diagnostic, OptionTypeDecl, Parser, ResultTypeDecl, Span, TokenKind, TypeNameId,
    TypeRef, csharp_numeric_type,
};

impl Parser<'_> {
    pub(super) fn resolve_type(&mut self, name: &str, span: Span) -> Result<TypeRef, Diagnostic> {
        if let Some(core) = TypeRef::parse(name) {
            return Ok(core);
        }
        let id = if let Some(id) = self.type_name_ids.get(name).copied() {
            id
        } else {
            let id = TypeNameId::from_index(self.type_names.len() as u32);
            self.type_names.push(name.to_owned());
            self.type_name_spans.push(span);
            self.type_name_ids.insert(name.to_owned(), id);
            id
        };
        Ok(TypeRef::Named(id))
    }

    pub(super) fn parse_type(
        &mut self,
        message: &'static str,
    ) -> Result<(TypeRef, Span), Diagnostic> {
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
            (self.resolve_type(&name, start)?, start, start)
        };

        if let Some(suffix) = self.eat(&TokenKind::Question) {
            let id = if let Some(&id) = self.option_type_ids.get(&ty) {
                id
            } else {
                let id = self.constructed_type_ids.option();
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
                let id = self.constructed_type_ids.result();
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
}
