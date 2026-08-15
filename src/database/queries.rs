//! Revisioned compiler-stage and editor query orchestration.

use std::{cmp::Reverse, sync::Arc};

use crate::{
    CheckedProgram, CompilerContext, Diagnostic, LoweredProgram, ParsedProgram, RecoveredCheck,
    RecoveredParse, WarningPolicy,
    ast::{AssignmentId, ExprId, ExprKind, FunctionId, Span, ValueId},
    highlight::SemanticHighlightIndex,
    hir::{Declaration, TypedExpressionKind},
    language::{LanguageCatalog, LanguageItemId},
    lexer::{Token, TokenKind},
    semantic::ResolvedCall,
    stdlib::StdlibSymbolId,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

use super::{
    DefinitionIndex, DefinitionTarget, PositionAnalysis, QueryResult, ReferenceIndex, ResolvedPath,
    SemanticQueryResult, SemanticSnapshot,
    cache::QueryCache,
    definition_for_resolution,
    position::{
        expression_segments, syntax_expression_at, syntax_expression_by_id,
        syntax_expression_resolution, syntax_expression_segments,
    },
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevision(u64);

impl SourceRevision {
    pub const fn index(self) -> u64 {
        self.0
    }
}

/// Caches compiler products for one SplitScript source buffer.
#[derive(Debug)]
pub struct CompilerDatabase {
    pub(super) context: CompilerContext,
    warning_policy: WarningPolicy,
    source_name: String,
    source: String,
    revision: SourceRevision,
    cache: QueryCache,
}

impl CompilerDatabase {
    pub fn new(source: impl Into<String>) -> Self {
        Self::with_context_and_source_name(
            CompilerContext::default(),
            crate::IN_MEMORY_SOURCE_NAME,
            source,
        )
    }

    pub fn with_context(context: CompilerContext, source: impl Into<String>) -> Self {
        Self::with_context_and_source_name(context, crate::IN_MEMORY_SOURCE_NAME, source)
    }

    pub fn with_source_name(source_name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::with_context_and_source_name(CompilerContext::default(), source_name, source)
    }

    pub fn with_context_and_source_name(
        context: CompilerContext,
        source_name: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            context,
            warning_policy: WarningPolicy::default(),
            source_name: source_name.into(),
            source: source.into(),
            revision: SourceRevision::default(),
            cache: QueryCache::default(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub const fn warning_policy(&self) -> WarningPolicy {
        self.warning_policy
    }

    /// Changes only the diagnostic presentation policy. Semantic compiler
    /// products remain valid even when a warning is denied.
    pub fn set_warning_policy(&mut self, warning_policy: WarningPolicy) -> bool {
        if warning_policy == self.warning_policy {
            return false;
        }
        self.warning_policy = warning_policy;
        self.cache.diagnostics = None;
        true
    }

    pub const fn revision(&self) -> SourceRevision {
        self.revision
    }

    /// Replaces the source and returns whether the query revision changed.
    pub fn set_source(&mut self, source: impl Into<String>) -> bool {
        let source = source.into();
        if source == self.source {
            return false;
        }
        self.source = source;
        self.revision.0 = self
            .revision
            .0
            .checked_add(1)
            .expect("source revision counter exhausted");
        self.cache = QueryCache::default();
        true
    }

    pub fn recovering_parse(&mut self) -> QueryResult<RecoveredParse> {
        if self.cache.recovered.is_none() {
            self.cache.recovered = Some(
                crate::parse_recovering_named_with_context(
                    self.context.clone(),
                    self.source_name.clone(),
                    &self.source,
                )
                .map(Arc::new)
                .map_err(Arc::from),
            );
        }
        self.cache.recovered.as_ref().unwrap().clone()
    }

    pub fn parse(&mut self) -> QueryResult<ParsedProgram> {
        if self.cache.parsed.is_none() {
            let parsed = match self.recovering_parse() {
                Ok(recovered) if recovered.diagnostics().is_empty() => {
                    Ok(Arc::new(ParsedProgram {
                        context: recovered.context(),
                        source_name: recovered.source_name().to_owned(),
                        document: recovered.source_document().clone(),
                        syntax: recovered.syntax().clone(),
                        resolution_diagnostics: recovered.resolution_diagnostics().to_vec(),
                    }))
                }
                Ok(recovered) => Err(Arc::from(recovered.diagnostics().to_vec())),
                Err(errors) => Err(errors),
            };
            self.cache.parsed = Some(parsed);
        }
        self.cache.parsed.as_ref().unwrap().clone()
    }

    pub fn format(&mut self) -> QueryResult<String> {
        if self.cache.formatted.is_none() {
            self.cache.formatted = Some(match self.parse() {
                Ok(parsed) => Ok(Arc::new(crate::formatter::format_parsed(&parsed))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.formatted.as_ref().unwrap().clone()
    }

    pub fn semantic_highlights(&mut self) -> QueryResult<SemanticHighlightIndex> {
        if self.cache.highlights.is_none() {
            let semantics = self
                .semantic_snapshot()
                .ok()
                .map(|snapshot| snapshot.semantics().clone());
            self.cache.highlights = Some(match self.recovering_parse() {
                Ok(parsed) => Ok(Arc::new(SemanticHighlightIndex::build(
                    parsed.source_document(),
                    parsed.syntax(),
                    semantics.as_ref(),
                    self.context.standard_library(),
                ))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.highlights.as_ref().unwrap().clone()
    }

    pub fn document_symbols(&mut self) -> QueryResult<Vec<crate::symbols::DocumentSymbol>> {
        if self.cache.document_symbols.is_none() {
            self.cache.document_symbols = Some(match self.recovering_parse() {
                Ok(parsed) => Ok(Arc::new(crate::symbols::document_symbols(
                    parsed.source_document(),
                    parsed.syntax(),
                ))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.document_symbols.as_ref().unwrap().clone()
    }

    pub fn selection_ranges(&mut self, offset: usize) -> SemanticQueryResult<Vec<Span>> {
        let parsed = self.recovering_parse()?;
        Ok(crate::selection_ranges::selection_ranges(
            parsed.source_document(),
            parsed.syntax(),
            offset,
        ))
    }

    pub fn recovering_lower(&mut self) -> QueryResult<LoweredProgram> {
        if self.cache.recovering_lowered.is_none() {
            self.cache.recovering_lowered = Some(match self.recovering_parse() {
                Ok(recovered) => {
                    let syntax = recovered.syntax().clone();
                    let mut compilation_syntax = syntax.clone();
                    if recovered.diagnostics().is_empty()
                        && let Some(augmented) = crate::stdlib::augment_program_with_library_bodies(
                            recovered.source_document().source(),
                            &recovered.context().standard_library(),
                        )
                        .expect("validated standard-library bodies parse in valid user source")
                    {
                        compilation_syntax = augmented;
                    }
                    let mut resolution_diagnostics = recovered.resolution_diagnostics().to_vec();
                    let mut resolutions = crate::resolution::ProgramResolutions::default();
                    resolution_diagnostics.extend(crate::resolution::resolve_program(
                        &compilation_syntax,
                        &recovered.context().standard_library(),
                        &mut resolutions,
                    ));
                    Ok(Arc::new(LoweredProgram {
                        context: recovered.context(),
                        source_name: recovered.source_name().to_owned(),
                        document: recovered.source_document().clone(),
                        hir: crate::hir::DeclarationIndex::lower(&syntax),
                        compilation_syntax,
                        syntax,
                        resolutions,
                        syntax_diagnostics: recovered.diagnostics().to_vec(),
                        resolution_diagnostics,
                    }))
                }
                Err(errors) => Err(errors),
            });
        }
        self.cache.recovering_lowered.as_ref().unwrap().clone()
    }

    pub fn lower(&mut self) -> QueryResult<LoweredProgram> {
        if self.cache.lowered.is_none() {
            self.cache.lowered = Some(match self.parse() {
                Ok(parsed) => Ok(Arc::new(crate::lower((*parsed).clone()))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.lowered.as_ref().unwrap().clone()
    }

    pub fn check(&mut self) -> QueryResult<CheckedProgram> {
        if self.cache.checked.is_none() {
            self.cache.checked = Some(match self.lower() {
                Ok(lowered) => crate::check((*lowered).clone())
                    .map(Arc::new)
                    .map_err(Arc::from),
                Err(errors) => Err(errors),
            });
        }
        self.cache.checked.as_ref().unwrap().clone()
    }

    pub fn recovering_check(&mut self) -> QueryResult<RecoveredCheck> {
        if self.cache.recovering_checked.is_none() {
            // Editor semantics must start from the recovered syntax tree. Using
            // strict lowering here makes one parser diagnostic discard every
            // otherwise valid type, definition, and reference in the file.
            self.cache.recovering_checked = Some(match self.recovering_lower() {
                Ok(lowered) => Ok(Arc::new(crate::check_recovering((*lowered).clone()))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.recovering_checked.as_ref().unwrap().clone()
    }

    /// Returns the best semantic facts available for editor queries without
    /// allowing recovery placeholders to enter strict compilation.
    pub fn semantic_snapshot(&mut self) -> QueryResult<SemanticSnapshot> {
        if self.cache.semantic_snapshot.is_none() {
            self.cache.semantic_snapshot = Some(match self.check() {
                Ok(checked) => Ok(Arc::new(SemanticSnapshot::Checked(checked))),
                Err(_) => self
                    .length_preserving_parse_repair_snapshot()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.recovering_check()
                            .map(|checked| Arc::new(SemanticSnapshot::Recovered(checked)))
                    }),
            });
        }
        self.cache.semantic_snapshot.as_ref().unwrap().clone()
    }

    /// Rechecks a source with parser-supplied, machine-applicable repairs when
    /// every edit preserves byte offsets. This lets semantic editor features
    /// survive substitutions such as `,` to `;` without publishing semantic
    /// spans that no longer line up with the user's buffer.
    fn length_preserving_parse_repair_snapshot(&mut self) -> Option<Arc<SemanticSnapshot>> {
        let recovered = self.recovering_parse().ok()?;
        if recovered.diagnostics().is_empty() {
            return None;
        }

        let mut edits = Vec::new();
        for diagnostic in recovered.diagnostics() {
            let fix = diagnostic.fixes.iter().find(|fix| {
                fix.applicability == crate::FixApplicability::MachineApplicable
                    && !fix.edits.is_empty()
                    && fix.edits.iter().all(|edit| {
                        edit.span.end.saturating_sub(edit.span.start) == edit.replacement.len()
                    })
            })?;
            edits.extend(fix.edits.iter().cloned());
        }
        edits.sort_by_key(|edit| (edit.span.start, edit.span.end));
        if edits
            .windows(2)
            .any(|pair| pair[0].span.end > pair[1].span.start)
        {
            return None;
        }

        let mut repaired = self.source.clone();
        for edit in edits.into_iter().rev() {
            if edit.span.end > repaired.len()
                || !repaired.is_char_boundary(edit.span.start)
                || !repaired.is_char_boundary(edit.span.end)
            {
                return None;
            }
            repaired.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }

        let mut database = Self::with_context_and_source_name(
            self.context.clone(),
            self.source_name.clone(),
            repaired,
        );
        Some(match database.check() {
            Ok(checked) => Arc::new(SemanticSnapshot::Checked(checked)),
            Err(_) => Arc::new(SemanticSnapshot::Recovered(
                database.recovering_check().ok()?,
            )),
        })
    }

    pub fn declarations_named(&mut self, name: &str) -> SemanticQueryResult<Vec<Declaration>> {
        let lowered = self.lower()?;
        Ok(lowered.hir().declarations_named(name).cloned().collect())
    }

    pub fn completions(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<crate::completion::CompletionList> {
        crate::completion::complete(self, offset)
    }

    pub fn hover(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<crate::insight::HoverInfo>> {
        crate::insight::hover(self, offset)
    }

    pub fn inlay_hints(
        &mut self,
        range: Span,
    ) -> SemanticQueryResult<Vec<crate::inlay_hints::InlayHint>> {
        let snapshot = self.semantic_snapshot()?;
        Ok(crate::inlay_hints::inferred_type_hints(&snapshot, range))
    }

    pub fn signature_help(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<crate::insight::SignatureHelp>> {
        crate::insight::signature_help(self, offset)
    }

    pub fn refactorings(
        &mut self,
        selection: Span,
    ) -> SemanticQueryResult<Vec<crate::refactor::Refactoring>> {
        crate::refactor::extract_refactorings(self, selection)
    }

    pub fn expression_type(&mut self, expression: ExprId) -> SemanticQueryResult<Option<TypeId>> {
        Ok(self
            .semantic_snapshot()?
            .semantics()
            .expression_type(expression))
    }

    pub fn value_type(&mut self, value: ValueId) -> SemanticQueryResult<Option<TypeId>> {
        Ok(self.semantic_snapshot()?.semantics().value_type(value))
    }

    pub fn function_result_type(
        &mut self,
        function: FunctionId,
    ) -> SemanticQueryResult<Option<TypeId>> {
        Ok(self
            .semantic_snapshot()?
            .semantics()
            .function_result(function))
    }

    pub fn type_kind(&mut self, ty: TypeId) -> SemanticQueryResult<Option<TypeKind>> {
        Ok(self
            .semantic_snapshot()?
            .semantics()
            .types()
            .get(ty)
            .cloned())
    }

    pub fn resolved_call(
        &mut self,
        expression: ExprId,
    ) -> SemanticQueryResult<Option<ResolvedCall>> {
        Ok(self
            .semantic_snapshot()?
            .semantics()
            .call(expression)
            .cloned())
    }

    pub fn resolved_path(
        &mut self,
        expression: ExprId,
    ) -> SemanticQueryResult<Option<ResolvedPath>> {
        let snapshot = self.semantic_snapshot()?;
        if let Some(typed_hir) = snapshot.typed_hir() {
            Ok(typed_hir
                .value_path(expression)
                .map(|(root, members)| ResolvedPath {
                    root,
                    members: members.to_vec(),
                }))
        } else {
            let root = snapshot.semantics().value(expression);
            let members = snapshot.semantics().path_members(expression);
            Ok((root.is_some() || members.is_some()).then(|| ResolvedPath {
                root,
                members: members.unwrap_or_default().to_vec(),
            }))
        }
    }

    pub fn assignment_target(
        &mut self,
        assignment: AssignmentId,
    ) -> SemanticQueryResult<Option<ValueId>> {
        Ok(self
            .semantic_snapshot()?
            .semantics()
            .assignment_target(assignment))
    }

    pub fn reference_index(&mut self) -> QueryResult<ReferenceIndex> {
        if self.cache.references.is_none() {
            self.cache.references = Some(match self.check() {
                Ok(checked) => Ok(Arc::new(ReferenceIndex::build(&checked))),
                Err(errors) => Err(errors),
            });
        }
        self.cache.references.as_ref().unwrap().clone()
    }

    pub fn definition_index(&mut self) -> QueryResult<DefinitionIndex> {
        if self.cache.definitions.is_none() {
            self.cache.definitions = Some(self.semantic_snapshot().map(|snapshot| {
                Arc::new(match &*snapshot {
                    SemanticSnapshot::Checked(checked) => DefinitionIndex::build(checked),
                    SemanticSnapshot::Recovered(checked) => {
                        DefinitionIndex::build_recovered(checked)
                    }
                })
            }));
        }
        self.cache.definitions.as_ref().unwrap().clone()
    }

    pub fn token_at(&mut self, offset: usize) -> SemanticQueryResult<Option<Token>> {
        let recovered = self.recovering_parse()?;
        Ok(recovered.source_document().token_at(offset).cloned())
    }

    pub(crate) fn hover_query_offset(&mut self, offset: usize) -> SemanticQueryResult<usize> {
        let recovered = self.recovering_parse()?;
        Ok(recovered
            .source_document()
            .symbol_token_at(offset)
            .filter(|token| token.span.end == offset)
            .map_or(offset, |token| token.span.end - 1))
    }

    pub(crate) fn caret_query_offset(&mut self, offset: usize) -> SemanticQueryResult<usize> {
        let recovered = self.recovering_parse()?;
        Ok(recovered
            .source_document()
            .caret_symbol_token_at(offset)
            .filter(|token| token.span.end == offset)
            .map_or(offset, |token| token.span.end - 1))
    }

    pub fn analysis_at(&mut self, offset: usize) -> SemanticQueryResult<Option<PositionAnalysis>> {
        let snapshot = self.semantic_snapshot()?;
        if let Some(checked) = snapshot.checked() {
            let Some(expression) = checked
                .typed_hir()
                .expressions()
                .filter(|expression| {
                    expression.span.start <= offset
                        && offset < expression.span.end
                        && expression.span.start != expression.span.end
                })
                .min_by_key(|expression| {
                    (
                        expression.span.end - expression.span.start,
                        Reverse(expression.id.index()),
                    )
                })
            else {
                return Ok(None);
            };
            let type_kind = checked.semantics().types().kind(expression.ty).clone();
            Ok(Some(PositionAnalysis {
                expression: expression.id,
                span: expression.span,
                segments: expression_segments(checked, expression),
                ty: expression.ty,
                type_kind,
                resolution: expression.resolution.clone(),
            }))
        } else {
            let semantics = snapshot.semantics();
            let Some(expression) = syntax_expression_at(snapshot.syntax(), semantics, offset)
            else {
                return Ok(None);
            };
            let ty = semantics
                .expression_type(expression.id)
                .expect("the recovered expression finder only retains typed expressions");
            Ok(Some(PositionAnalysis {
                expression: expression.id,
                span: expression.span,
                segments: syntax_expression_segments(snapshot.source_document(), expression),
                ty,
                type_kind: semantics.types().kind(ty).clone(),
                resolution: syntax_expression_resolution(semantics, expression),
            }))
        }
    }

    pub fn definition_at(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<DefinitionTarget>> {
        let offset = self.caret_query_offset(offset)?;
        self.definition_at_query_offset(offset)
    }

    pub(crate) fn definition_at_query_offset(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<DefinitionTarget>> {
        let definitions = self.definition_index()?;
        if let Some(reference) = definitions.reference_at(offset) {
            return Ok(definitions
                .get(reference.target)
                .cloned()
                .map(DefinitionTarget::Source));
        }
        let analysis = self.analysis_at(offset)?;
        if let Some(analysis) = &analysis
            && let Some(segment_index) = analysis
                .segments
                .iter()
                .position(|segment| segment.span.start <= offset && offset < segment.span.end)
            && let Some(target) = definition_for_resolution(
                &definitions,
                analysis,
                segment_index,
                self.context.standard_library(),
            )
        {
            return Ok(Some(target));
        }
        self.language_definition_at(offset, analysis.as_ref())
    }

    pub fn references_at(
        &mut self,
        offset: usize,
        include_declaration: bool,
    ) -> SemanticQueryResult<Vec<Span>> {
        let Some(DefinitionTarget::Source(definition)) = self.definition_at(offset)? else {
            return Ok(Vec::new());
        };
        let definitions = self.definition_index()?;
        Ok(definitions
            .references_to(definition.id)
            .filter(|reference| include_declaration || reference.span != definition.span)
            .map(|reference| reference.span)
            .collect())
    }

    fn language_definition_at(
        &mut self,
        offset: usize,
        analysis: Option<&PositionAnalysis>,
    ) -> SemanticQueryResult<Option<DefinitionTarget>> {
        let Some(token) = self.token_at(offset)? else {
            return Ok(None);
        };
        if let TokenKind::Ident(name) = &token.kind {
            let recovered = self.recovering_parse()?;
            if let Some(policy) = recovered.syntax().tick_rate
                && (policy.keyword_span == token.span
                    || policy
                        .attached
                        .is_some_and(|rate| rate.keyword_span == token.span)
                    || policy
                        .detached
                        .is_some_and(|rate| rate.keyword_span == token.span))
            {
                return Ok(Some(DefinitionTarget::Language(LanguageItemId::TickRate)));
            }
            if let Some(item) = contextual_language_item_at(recovered.syntax(), token.span) {
                return Ok(Some(DefinitionTarget::Language(item)));
            }
            if let Some(provider) = recovered
                .syntax()
                .state
                .as_ref()
                .and_then(|state| state.provider.as_ref())
                .filter(|provider| {
                    provider.span.start <= offset
                        && offset < provider.span.end
                        && provider.name == *name
                })
                .and_then(|provider| {
                    self.context
                        .standard_library()
                        .state_provider_by_name(&provider.name)
                })
            {
                return Ok(Some(DefinitionTarget::StandardLibrarySymbol(
                    StdlibSymbolId::StateProvider(provider.id),
                )));
            }
            if (name == "utf8" || name == "utf16le")
                && recovered
                    .syntax()
                    .state
                    .as_ref()
                    .into_iter()
                    .flat_map(|state| state.all_fields())
                    .filter_map(|field| match &field.source {
                        crate::ast::StateSource::Pointer(path) => path.decoder,
                        crate::ast::StateSource::Expression(_) => None,
                    })
                    .any(|decoder| match decoder {
                        crate::ast::StateMemoryDecoder::Utf8 { span, .. } => {
                            name == "utf8" && span.start <= offset && offset < span.end
                        }
                        crate::ast::StateMemoryDecoder::Utf16Le { span, .. } => {
                            name == "utf16le" && span.start <= offset && offset < span.end
                        }
                    })
            {
                return Ok(Some(DefinitionTarget::Language(if name == "utf8" {
                    LanguageItemId::NativeStringDecoder
                } else {
                    LanguageItemId::NativeUtf16LeDecoder
                })));
            }
        }
        if let TokenKind::Ident(name) = &token.kind
            && let Some(namespace) = analysis
                .and_then(|analysis| {
                    let segment = analysis.segments.iter().position(|segment| {
                        segment.span.start <= offset && offset < segment.span.end
                    })?;
                    let path = analysis.segments[..=segment]
                        .iter()
                        .map(|segment| segment.name.as_str())
                        .collect::<Vec<_>>();
                    self.context.standard_library().namespace_by_path(&path)
                })
                .or_else(|| self.context.standard_library().namespace_by_name(name))
        {
            return Ok(Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Namespace(namespace.id),
            )));
        }
        if let TokenKind::Ident(name) = &token.kind
            && let Some(ty) = self.context.standard_library().type_by_name(name)
        {
            return Ok(Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Type(ty.id),
            )));
        }
        if let TokenKind::Ident(name) = &token.kind
            && let Some(constructor) = self
                .context
                .standard_library()
                .named_type_constructor_by_name(name)
        {
            return Ok(Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::TypeConstructor(constructor.id),
            )));
        }
        let language = LanguageCatalog::new();
        let item = match &token.kind {
            TokenKind::Ident(name) => language.item_for_source_token(name),
            TokenKind::Question => {
                let propagation = if let Some(analysis) = analysis {
                    let snapshot = self.semantic_snapshot()?;
                    if let Some(typed_hir) = snapshot.typed_hir() {
                        typed_hir
                            .expression(analysis.expression)
                            .is_some_and(|expression| {
                                matches!(expression.kind, TypedExpressionKind::Propagate { .. })
                            })
                    } else {
                        syntax_expression_by_id(snapshot.syntax(), analysis.expression).is_some_and(
                            |expression| matches!(expression.kind, ExprKind::Propagate(_)),
                        )
                    }
                } else {
                    false
                };
                Some(language.item(if propagation {
                    LanguageItemId::Propagate
                } else {
                    LanguageItemId::OptionType
                }))
            }
            TokenKind::Bang => Some(language.item(LanguageItemId::ResultType)),
            TokenKind::LBracket => {
                let is_index = analysis.is_some_and(|analysis| {
                    let Ok(snapshot) = self.semantic_snapshot() else {
                        return false;
                    };
                    syntax_expression_by_id(snapshot.syntax(), analysis.expression).is_some_and(
                        |expression| {
                            matches!(
                                expression.kind,
                                ExprKind::Index { bracket_span, .. }
                                    if bracket_span.start <= offset && offset < bracket_span.end
                            )
                        },
                    )
                });
                Some(language.item(if is_index {
                    LanguageItemId::ArrayIndex
                } else {
                    LanguageItemId::ArrayType
                }))
            }
            TokenKind::DocComment(_) => Some(language.item(LanguageItemId::DocumentationComment)),
            TokenKind::TemplateStart | TokenKind::TemplateChunk(_) | TokenKind::TemplateEnd => {
                Some(language.item(LanguageItemId::TemplateString))
            }
            _ => None,
        };
        Ok(item.map(|item| DefinitionTarget::Language(item.id)))
    }

    pub fn diagnostics(&mut self) -> Arc<[Diagnostic]> {
        if self.cache.diagnostics.is_none() {
            let diagnostics = match self.check() {
                Ok(checked) => self
                    .warning_policy
                    .apply(checked.diagnostics().iter().cloned()),
                Err(errors)
                    if !errors
                        .iter()
                        .any(|diagnostic| self.warning_policy.changes(diagnostic)) =>
                {
                    self.cache.diagnostics = Some(errors);
                    return Arc::clone(self.cache.diagnostics.as_ref().unwrap());
                }
                Err(errors) => self.warning_policy.apply(errors.iter().cloned()),
            };
            self.cache.diagnostics = Some(Arc::from(diagnostics));
        }
        Arc::clone(self.cache.diagnostics.as_ref().unwrap())
    }
}

struct ContextualLanguageItemAt {
    target: Span,
    item: Option<LanguageItemId>,
}

impl<'ast> Visitor<'ast> for ContextualLanguageItemAt {
    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        if let crate::ast::StateSource::Pointer(path) = &field.source
            && path.at_span == Some(self.target)
        {
            self.item = Some(LanguageItemId::StatePointerField);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        if setting
            .external_key
            .as_ref()
            .is_some_and(|key| key.keyword_span == self.target)
        {
            self.item = Some(LanguageItemId::StableSettingKey);
            return;
        }
        match &setting.kind {
            crate::ast::SettingKind::Choice {
                keyword_span,
                options,
                ..
            } => {
                if *keyword_span == self.target
                    || options
                        .iter()
                        .any(|option| option.default_span == Some(self.target))
                {
                    self.item = Some(LanguageItemId::ChoiceSetting);
                }
            }
            crate::ast::SettingKind::File {
                keyword_span,
                filters,
            } => {
                if *keyword_span == self.target
                    || filters.iter().any(|filter| {
                        matches!(
                            filter,
                            crate::ast::SettingFileFilter::Mime { keyword_span, .. }
                                if *keyword_span == self.target
                        )
                    })
                {
                    self.item = Some(LanguageItemId::FileSetting);
                }
            }
            crate::ast::SettingKind::Bool { .. } | crate::ast::SettingKind::Title { .. } => {}
        }
    }

    fn visit_setting_family(&mut self, family: &'ast crate::ast::SettingFamilyDecl) {
        if family.keyword_span == self.target || family.in_span == self.target {
            self.item = Some(LanguageItemId::SettingFamily);
        } else if family.key_keyword_span == Some(self.target) {
            self.item = Some(LanguageItemId::StableSettingKey);
        }
    }

    fn visit_stmt(&mut self, statement: &'ast crate::ast::Stmt) {
        if let crate::ast::Stmt::For { in_span, .. } = statement
            && *in_span == self.target
        {
            self.item = Some(LanguageItemId::For);
        }
        visit::walk_stmt(self, statement);
    }
}

fn contextual_language_item_at(
    program: &crate::ast::Program,
    target: Span,
) -> Option<LanguageItemId> {
    let mut finder = ContextualLanguageItemAt { target, item: None };
    finder.visit_program(program);
    finder.item
}

impl Default for CompilerDatabase {
    fn default() -> Self {
        Self::new(String::new())
    }
}
