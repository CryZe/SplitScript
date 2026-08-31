//! Identity-preserving source rename validation.

use std::{collections::HashSet, fmt, sync::Arc};

use crate::{
    Diagnostic,
    ast::{Expr, ExprKind, Span},
    diagnostic::TextEdit,
    language::LanguageCatalog,
    lexer::TokenKind,
    semantic::{ResolvedRecordFieldId, SemanticModel},
    stdlib::{StandardLibrary, StdlibOwner},
    visit::{self, Visitor},
};

use super::{CompilerDatabase, DefinitionTarget, SemanticQueryResult, SourceDefinitionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    pub id: SourceDefinitionId,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    pub new_name: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    Diagnostics(Arc<[Diagnostic]>),
    NotRenameable,
    InvalidIdentifier,
    ReservedIdentifier,
    ConflictingBinding,
}

#[derive(Debug, Clone)]
struct RecordShorthandReference {
    span: Span,
    name: String,
    field: SourceDefinitionId,
    value: SourceDefinitionId,
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
        let offset = self.caret_query_offset(offset)?;
        let definitions = self.definition_index()?;
        if let Some(reference) = definitions.reference_at(offset) {
            let target = definitions.get(reference.target).and_then(|definition| {
                (!matches!(
                    definition.id,
                    SourceDefinitionId::State | SourceDefinitionId::Settings
                ))
                .then(|| RenameTarget {
                    id: definition.id,
                    name: definition.name.clone(),
                    span: reference.span,
                })
            });
            if target
                .as_ref()
                .is_some_and(|target| self.is_generated_layout_symbol(target.id))
            {
                return Ok(None);
            }
            return Ok(target);
        }
        let target = match self.definition_at_query_offset(offset)? {
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
        };
        Ok(target.filter(|target| !self.is_generated_layout_symbol(target.id)))
    }

    /// Validates an identity-preserving source rename and returns its complete
    /// text-edit plan. The rebuilt candidate must type-check and all existing
    /// source references must retain their stable declaration IDs. Declarations
    /// whose local spelling also supplies an external identity may add a
    /// zero-width preservation edit.
    pub fn rename_at(&mut self, offset: usize, new_name: &str) -> Result<RenamePlan, RenameError> {
        self.check().map_err(RenameError::Diagnostics)?;
        let target = self
            .rename_target_at(offset)
            .map_err(RenameError::Diagnostics)?
            .ok_or(RenameError::NotRenameable)?;
        if !splitscript_syntax::is_identifier(new_name) {
            return Err(RenameError::InvalidIdentifier);
        }
        if is_reserved_source_identifier(self.context.standard_library(), new_name) {
            return Err(RenameError::ReservedIdentifier);
        }

        let definitions = self.definition_index().map_err(RenameError::Diagnostics)?;
        let shorthand_references = self.record_shorthand_references()?;
        let target_ids = self.logical_rename_ids(target.id)?;
        let mut spans = target_ids
            .iter()
            .flat_map(|id| definitions.references_to(*id))
            .map(|reference| reference.span)
            .collect::<Vec<_>>();
        spans.sort_by_key(|span| (span.start, span.end));
        spans.dedup();
        let mut expanded_shorthands = Vec::new();
        let mut edits = spans
            .iter()
            .copied()
            .map(|span| {
                let shorthand = shorthand_references.iter().find(|reference| {
                    reference.span == span
                        && (target_ids.contains(&reference.field)
                            || target_ids.contains(&reference.value))
                });
                let replacement = shorthand.map_or_else(
                    || new_name.to_owned(),
                    |reference| {
                        expanded_shorthands.push(reference.clone());
                        if target_ids.contains(&reference.field) {
                            format!("{new_name}: {}", reference.name)
                        } else {
                            format!("{}: {new_name}", reference.name)
                        }
                    },
                );
                TextEdit { span, replacement }
            })
            .collect::<Vec<_>>();
        if new_name == target.name {
            return Ok(RenamePlan {
                new_name: new_name.to_owned(),
                edits,
            });
        }

        if let Some(preservation) = self.external_identity_preservation(target.id, &target.name)? {
            edits.push(preservation);
        }
        edits.sort_by_key(|edit| (edit.span.start, edit.span.end));

        let candidate_source = apply_edits(self.source(), &edits);
        let mut candidate = Self::with_context(self.context.clone(), candidate_source);
        candidate
            .check()
            .map_err(|_| RenameError::ConflictingBinding)?;
        let candidate_definitions = candidate
            .definition_index()
            .map_err(|_| RenameError::ConflictingBinding)?;
        for reference in definitions.syntax_references() {
            if expanded_shorthands
                .iter()
                .any(|shorthand| shorthand.span == reference.span)
            {
                continue;
            }
            let mapped = remap_span(reference.span, &edits);
            if !candidate_definitions
                .syntax_references()
                .iter()
                .any(|candidate_reference| {
                    candidate_reference.span == mapped
                        && candidate_reference.target == reference.target
                })
            {
                return Err(RenameError::ConflictingBinding);
            }
        }
        for shorthand in &expanded_shorthands {
            let mapped = remap_span(shorthand.span, &edits);
            let renamed_field = target_ids.contains(&shorthand.field);
            let (field_name, value_name) = if renamed_field {
                (new_name, shorthand.name.as_str())
            } else {
                (shorthand.name.as_str(), new_name)
            };
            let field_span = Span {
                start: mapped.start,
                end: mapped.start + field_name.len(),
            };
            let value_span = Span {
                start: field_span.end + 2,
                end: field_span.end + 2 + value_name.len(),
            };
            if !candidate_definitions
                .reference_at(field_span.start)
                .is_some_and(|reference| {
                    reference.span == field_span && reference.target == shorthand.field
                })
                || !candidate_definitions
                    .reference_at(value_span.start)
                    .is_some_and(|reference| {
                        reference.span == value_span && reference.target == shorthand.value
                    })
            {
                return Err(RenameError::ConflictingBinding);
            }
        }
        Ok(RenamePlan {
            new_name: new_name.to_owned(),
            edits,
        })
    }

    fn record_shorthand_references(
        &mut self,
    ) -> Result<Vec<RecordShorthandReference>, RenameError> {
        let checked = self.check().map_err(RenameError::Diagnostics)?;
        let mut collector = RecordShorthandCollector {
            semantics: checked.semantics(),
            references: Vec::new(),
        };
        collector.visit_program(checked.syntax());
        Ok(collector.references)
    }

    /// Preserves an external identity that would otherwise be inherited from
    /// the declaration's source-facing name. Each domain keeps its own syntax
    /// and cardinality: settings have one stable `key`, managed declarations
    /// have one or more lookup names introduced by `from`.
    fn external_identity_preservation(
        &mut self,
        target: SourceDefinitionId,
        old_name: &str,
    ) -> Result<Option<TextEdit>, RenameError> {
        let parsed = self.parse().map_err(RenameError::Diagnostics)?;
        let syntax = parsed.syntax();
        let edit = match target {
            SourceDefinitionId::Value(id) => syntax
                .settings
                .iter()
                .find(|setting| {
                    setting.id == id && setting.source_visible && setting.external_key.is_none()
                })
                .map(|setting| TextEdit {
                    span: Span {
                        start: setting.name_span.end,
                        end: setting.name_span.end,
                    },
                    replacement: format!(" key \"{old_name}\""),
                }),
            SourceDefinitionId::ManagedClass(id) => syntax
                .managed_class_declarations()
                .into_iter()
                .find(|class| class.id == id && class.metadata_names.keyword_span.is_none())
                .map(|class| TextEdit {
                    span: Span {
                        start: class.name_span.end,
                        end: class.name_span.end,
                    },
                    replacement: format!(" from \"{old_name}\""),
                }),
            SourceDefinitionId::ManagedField(id) => syntax
                .managed_class_declarations()
                .into_iter()
                .flat_map(|class| class.all_fields())
                .find(|field| field.id == id && field.metadata_names.keyword_span.is_none())
                .map(|field| TextEdit {
                    span: Span {
                        start: field.name_span.end,
                        end: field.name_span.end,
                    },
                    replacement: format!(" from [\"{old_name}\", \"<{old_name}>k__BackingField\"]"),
                }),
            _ => None,
        };
        Ok(edit)
    }

    /// Compatible declarations across named layouts expose one shared
    /// snapshot field. Layout-specific declarations keep their own identity,
    /// even when another layout happens to use the same spelling with a
    /// conflicting type.
    fn logical_rename_ids(
        &mut self,
        target: SourceDefinitionId,
    ) -> Result<Vec<SourceDefinitionId>, RenameError> {
        let SourceDefinitionId::Value(target) = target else {
            return Ok(vec![target]);
        };
        let parsed = self.parse().map_err(RenameError::Diagnostics)?;
        let Some(state) = &parsed.syntax().state else {
            return Ok(vec![SourceDefinitionId::Value(target)]);
        };
        if state.layouts.is_empty() {
            return Ok(vec![SourceDefinitionId::Value(target)]);
        }
        let Some(name) = state
            .all_fields()
            .find(|field| field.id == target)
            .map(|field| field.name.as_str())
        else {
            return Ok(vec![SourceDefinitionId::Value(target)]);
        };
        if !state.is_common_field(name) {
            return Ok(vec![SourceDefinitionId::Value(target)]);
        }
        Ok(state
            .all_fields()
            .filter(|field| field.name == name)
            .map(|field| SourceDefinitionId::Value(field.id))
            .collect())
    }

    /// Plans an underscore-prefixed rename for a compiler warning.
    ///
    /// The ordinary rename validator proves that all references retain their
    /// declaration identities after the edit. Additional underscores are
    /// tried when the most natural spelling would collide with an existing
    /// binding.
    pub fn underscore_suppression_at(
        &mut self,
        offset: usize,
    ) -> Result<Option<RenamePlan>, RenameError> {
        let Some(target) = self
            .rename_target_at(offset)
            .map_err(RenameError::Diagnostics)?
        else {
            return Ok(None);
        };
        if target.name.starts_with('_') {
            return Ok(None);
        }

        let occupied = self
            .recovering_parse()
            .map_err(RenameError::Diagnostics)?
            .source_document()
            .tokens()
            .filter_map(|token| match &token.kind {
                TokenKind::Ident(name) => Some(name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let standard_library = self.context.standard_library();
        let mut replacement = format!("_{}", target.name);
        loop {
            while is_reserved_source_identifier(standard_library.clone(), &replacement) {
                replacement.insert(0, '_');
            }
            match self.rename_at(offset, &replacement) {
                Ok(plan) => return Ok(Some(plan)),
                // A spelling that occurs in the finite source token set may
                // genuinely capture another binding. Once the spelling is
                // fresh, another underscore cannot repair a structural rename
                // validation failure, so propagate it instead of looping.
                Err(RenameError::ConflictingBinding) if occupied.contains(&replacement) => {
                    replacement.insert(0, '_');
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn is_generated_layout_symbol(&mut self, id: SourceDefinitionId) -> bool {
        let Ok(parsed) = self.recovering_parse() else {
            return false;
        };
        let Some(state) = &parsed.syntax().state else {
            return false;
        };
        matches!(id, SourceDefinitionId::Value(value) if state.layout_value == Some(value))
            || matches!(id, SourceDefinitionId::Enum(enumeration)
                if state.layout_enum.as_ref().is_some_and(|layout| layout.id == enumeration))
    }
}

struct RecordShorthandCollector<'a> {
    semantics: &'a SemanticModel,
    references: Vec<RecordShorthandReference>,
}

impl<'ast> Visitor<'ast> for RecordShorthandCollector<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Record { fields, .. } = &expression.kind {
            let resolved = self
                .semantics
                .record_literal_fields(expression.id)
                .unwrap_or_default();
            for (field, resolved_field) in fields.iter().zip(resolved) {
                if !field.shorthand {
                    continue;
                }
                let ResolvedRecordFieldId::Source(record_field) = resolved_field else {
                    continue;
                };
                let Some(value) = self
                    .semantics
                    .value(field.value.id)
                    .and_then(|root| root.source_value())
                else {
                    continue;
                };
                self.references.push(RecordShorthandReference {
                    span: field.name_span,
                    name: field.name.clone(),
                    field: SourceDefinitionId::RecordField(*record_field),
                    value: SourceDefinitionId::Value(value),
                });
            }
        }
        visit::walk_expr(self, expression);
    }
}

fn is_reserved_source_identifier(standard_library: StandardLibrary, name: &str) -> bool {
    let language_reserved = LanguageCatalog::new().item_for_source_token(name).is_some();
    let standard_library_reserved = standard_library.namespace_by_name(name).is_some()
        || standard_library.type_by_name(name).is_some()
        || standard_library
            .items()
            .any(|item| item.owner == StdlibOwner::Root && item.name == name);
    language_reserved || standard_library_reserved
}

fn apply_edits(source: &str, edits: &[TextEdit]) -> String {
    let removed = edits
        .iter()
        .map(|edit| edit.span.end - edit.span.start)
        .sum::<usize>();
    let mut result = String::with_capacity(
        source.len() - removed
            + edits
                .iter()
                .map(|edit| edit.replacement.len())
                .sum::<usize>(),
    );
    let mut cursor = 0;
    for edit in edits {
        result.push_str(&source[cursor..edit.span.start]);
        result.push_str(&edit.replacement);
        cursor = edit.span.end;
    }
    result.push_str(&source[cursor..]);
    result
}

fn remap_span(span: Span, edits: &[TextEdit]) -> Span {
    let mut delta = 0isize;
    for edit in edits {
        if edit.span == span {
            let start = span.start.checked_add_signed(delta).unwrap();
            return Span {
                start,
                end: start + edit.replacement.len(),
            };
        }
        if edit.span.end <= span.start {
            delta += edit.replacement.len() as isize - (edit.span.end - edit.span.start) as isize;
        } else {
            break;
        }
    }
    Span {
        start: span.start.checked_add_signed(delta).unwrap(),
        end: span.end.checked_add_signed(delta).unwrap(),
    }
}
