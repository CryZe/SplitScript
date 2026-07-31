//! Identity-preserving source rename validation.

use std::{fmt, sync::Arc};

use crate::{
    Diagnostic,
    ast::Span,
    language::LanguageCatalog,
    stdlib::{StandardLibrary, StdlibOwner},
};

use super::{CompilerDatabase, DefinitionTarget, SemanticQueryResult, SourceDefinitionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    pub id: SourceDefinitionId,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    Diagnostics(Arc<[Diagnostic]>),
    NotRenameable,
    InvalidIdentifier,
    ReservedIdentifier,
    ConflictingBinding,
}

impl fmt::Display for RenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Diagnostics(_) => formatter.write_str("the source has compiler diagnostics"),
            Self::NotRenameable => formatter.write_str("the selected symbol cannot be renamed"),
            Self::InvalidIdentifier => {
                formatter.write_str("the new name is not a valid identifier")
            }
            Self::ReservedIdentifier => formatter.write_str("the new name is reserved"),
            Self::ConflictingBinding => formatter
                .write_str("the rename would capture a reference or change declaration identity"),
        }
    }
}

impl std::error::Error for RenameError {}

impl CompilerDatabase {
    /// Returns the source declaration that can be renamed at `offset`.
    /// Language and standard-library catalog symbols are intentionally not
    /// renameable source declarations.
    pub fn rename_target_at(&mut self, offset: usize) -> SemanticQueryResult<Option<RenameTarget>> {
        let definitions = self.definition_index()?;
        if let Some(reference) = definitions.reference_at(offset) {
            return Ok(definitions.get(reference.target).and_then(|definition| {
                (!matches!(
                    definition.id,
                    SourceDefinitionId::State | SourceDefinitionId::Settings
                ))
                .then(|| RenameTarget {
                    id: definition.id,
                    name: definition.name.clone(),
                    span: reference.span,
                })
            }));
        }
        Ok(match self.definition_at(offset)? {
            Some(DefinitionTarget::Source(definition)) => {
                if matches!(
                    definition.id,
                    SourceDefinitionId::State | SourceDefinitionId::Settings
                ) {
                    return Ok(None);
                }
                let span = self
                    .token_at(offset)?
                    .map_or(definition.span, |token| token.span);
                Some(RenameTarget {
                    id: definition.id,
                    name: definition.name,
                    span,
                })
            }
            Some(
                DefinitionTarget::StandardLibrary(_)
                | DefinitionTarget::StandardLibrarySymbol(_)
                | DefinitionTarget::Language(_),
            )
            | None => None,
        })
    }

    /// Validates an identity-preserving source rename and returns every exact
    /// identifier span to edit. The rebuilt candidate must type-check and all
    /// existing source references must retain their stable declaration IDs.
    pub fn rename_at(&mut self, offset: usize, new_name: &str) -> Result<Vec<Span>, RenameError> {
        self.check().map_err(RenameError::Diagnostics)?;
        let target = self
            .rename_target_at(offset)
            .map_err(RenameError::Diagnostics)?
            .ok_or(RenameError::NotRenameable)?;
        if !is_source_identifier(new_name) {
            return Err(RenameError::InvalidIdentifier);
        }
        if is_reserved_source_identifier(self.context.standard_library(), new_name) {
            return Err(RenameError::ReservedIdentifier);
        }

        let definitions = self.definition_index().map_err(RenameError::Diagnostics)?;
        let spans = definitions
            .references_to(target.id)
            .map(|reference| reference.span)
            .collect::<Vec<_>>();
        if new_name == target.name {
            return Ok(spans);
        }

        let candidate_source = replace_spans(self.source(), &spans, new_name);
        let mut candidate = Self::with_context(self.context.clone(), candidate_source);
        candidate
            .check()
            .map_err(|_| RenameError::ConflictingBinding)?;
        let candidate_definitions = candidate
            .definition_index()
            .map_err(|_| RenameError::ConflictingBinding)?;
        for reference in definitions.syntax_references() {
            let mapped = remap_span(reference.span, &spans, new_name.len());
            if !candidate_definitions
                .reference_at(mapped.start)
                .is_some_and(|candidate_reference| {
                    candidate_reference.span == mapped
                        && candidate_reference.target == reference.target
                })
            {
                return Err(RenameError::ConflictingBinding);
            }
        }
        Ok(spans)
    }
}

fn is_source_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    matches!(first, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
}

fn is_reserved_source_identifier(standard_library: StandardLibrary, name: &str) -> bool {
    let language_reserved = LanguageCatalog::new().item_for_source_token(name).is_some();
    let standard_library_reserved = standard_library.namespace_by_name(name).is_some()
        || standard_library.type_by_name(name).is_some()
        || standard_library
            .items()
            .iter()
            .any(|item| item.owner == StdlibOwner::Root && item.name == name);
    language_reserved || standard_library_reserved
}

fn replace_spans(source: &str, spans: &[Span], replacement: &str) -> String {
    let removed = spans
        .iter()
        .map(|span| span.end - span.start)
        .sum::<usize>();
    let mut result = String::with_capacity(
        source.len() - removed + spans.len().saturating_mul(replacement.len()),
    );
    let mut cursor = 0;
    for span in spans {
        result.push_str(&source[cursor..span.start]);
        result.push_str(replacement);
        cursor = span.end;
    }
    result.push_str(&source[cursor..]);
    result
}

fn remap_span(span: Span, replacements: &[Span], replacement_len: usize) -> Span {
    let mut delta = 0isize;
    for replacement in replacements {
        if *replacement == span {
            let start = span.start.checked_add_signed(delta).unwrap();
            return Span {
                start,
                end: start + replacement_len,
            };
        }
        if replacement.end <= span.start {
            delta += replacement_len as isize - (replacement.end - replacement.start) as isize;
        } else {
            break;
        }
    }
    Span {
        start: span.start.checked_add_signed(delta).unwrap(),
        end: span.end.checked_add_signed(delta).unwrap(),
    }
}
