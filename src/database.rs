//! Revisioned, single-source compiler queries for editor and tooling clients.

use std::{cmp::Reverse, collections::HashMap, fmt, sync::Arc};

use crate::{
    CheckedProgram, Diagnostic, LoweredProgram, ParsedProgram, RecoveredCheck, RecoveredParse,
    ast::{
        AssignmentId, EnumId, EnumTypeId, EnumVariantId, ExprId, ExprKind, FunctionId,
        MatchPattern, RecordFieldId, RecordId, Span, Stmt, TypeRef as SyntaxTypeRef, ValueId,
    },
    highlight::SemanticHighlightIndex,
    hir::{
        Declaration, ExpressionResolution, ResolvedAssignment, TypedExpression, TypedExpressionKind,
    },
    language::{LanguageCatalog, LanguageItemId},
    lexer::{Token, TokenKind},
    semantic::{ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::{StandardLibrary, StdlibItemId, StdlibOwner, StdlibSymbolId},
    syntax::SourceDocument,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

pub type QueryResult<T> = Result<Arc<T>, Arc<[Diagnostic]>>;
pub type SemanticQueryResult<T> = Result<T, Arc<[Diagnostic]>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub root: Option<ResolvedValue>,
    pub members: Vec<ResolvedMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionAnalysis {
    pub expression: ExprId,
    pub span: Span,
    /// Exact identifier tokens forming a path or call target. This excludes
    /// call arguments and other identifiers in child expressions.
    pub segments: Vec<IdentifierSegment>,
    pub ty: TypeId,
    pub type_kind: TypeKind,
    pub resolution: Option<ExpressionResolution>,
}

impl PositionAnalysis {
    pub fn segment_at(&self, offset: usize) -> Option<&IdentifierSegment> {
        self.segments
            .iter()
            .find(|segment| segment.span.start <= offset && offset < segment.span.end)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierSegment {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceDefinitionId {
    Value(ValueId),
    Function(FunctionId),
    Record(RecordId),
    RecordField(RecordFieldId),
    Enum(EnumId),
    EnumVariant(EnumVariantId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDefinition {
    pub id: SourceDefinitionId,
    pub name: String,
    /// The exact identifier span, rather than the surrounding declaration.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    pub id: SourceDefinitionId,
    pub name: String,
    /// The exact occurrence selected by the cursor, not necessarily the
    /// declaration occurrence.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionTarget {
    Source(SourceDefinition),
    StandardLibrary(StdlibItemId),
    StandardLibrarySymbol(StdlibSymbolId),
    Language(LanguageItemId),
}

#[derive(Debug, Clone)]
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
            Self::Diagnostics(_) => {
                formatter.write_str("rename requires a semantically valid document")
            }
            Self::NotRenameable => formatter.write_str("the selected symbol cannot be renamed"),
            Self::InvalidIdentifier => formatter.write_str(
                "the new name must be an ASCII identifier beginning with a letter, `_`, or `$`",
            ),
            Self::ReservedIdentifier => {
                formatter.write_str("the new name is reserved by the language or standard library")
            }
            Self::ConflictingBinding => formatter.write_str(
                "the new name conflicts with another declaration or would change name resolution",
            ),
        }
    }
}

impl std::error::Error for RenameError {}

#[derive(Debug, Clone, Default)]
pub struct DefinitionIndex {
    values: HashMap<ValueId, SourceDefinition>,
    functions: HashMap<FunctionId, SourceDefinition>,
    records: HashMap<RecordId, SourceDefinition>,
    record_fields: HashMap<RecordFieldId, SourceDefinition>,
    enums: HashMap<EnumId, SourceDefinition>,
    enum_variants: HashMap<EnumVariantId, SourceDefinition>,
    syntax_references: Vec<SyntaxReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxReference {
    pub target: SourceDefinitionId,
    pub span: Span,
}

impl DefinitionIndex {
    fn build(checked: &CheckedProgram) -> Self {
        Self::build_from_parts(
            checked.source_document(),
            checked.syntax(),
            checked.semantics(),
        )
    }

    fn build_recovered(checked: &RecoveredCheck) -> Self {
        Self::build_from_parts(
            checked.source_document(),
            checked.syntax(),
            checked.semantics(),
        )
    }

    fn build_from_parts(
        document: &SourceDocument,
        syntax: &crate::ast::Program,
        semantics: &SemanticModel,
    ) -> Self {
        let mut collector = DefinitionCollector {
            document,
            syntax,
            semantics,
            index: Self::default(),
        };
        collector.visit_program(syntax);
        collector
            .index
            .syntax_references
            .sort_by_key(|reference| (reference.span.start, reference.span.end));
        collector.index.syntax_references.dedup();
        collector.index
    }

    pub fn get(&self, id: SourceDefinitionId) -> Option<&SourceDefinition> {
        match id {
            SourceDefinitionId::Value(id) => self.values.get(&id),
            SourceDefinitionId::Function(id) => self.functions.get(&id),
            SourceDefinitionId::Record(id) => self.records.get(&id),
            SourceDefinitionId::RecordField(id) => self.record_fields.get(&id),
            SourceDefinitionId::Enum(id) => self.enums.get(&id),
            SourceDefinitionId::EnumVariant(id) => self.enum_variants.get(&id),
        }
    }

    pub fn syntax_references(&self) -> &[SyntaxReference] {
        &self.syntax_references
    }

    pub fn reference_at(&self, offset: usize) -> Option<&SyntaxReference> {
        self.syntax_references
            .iter()
            .find(|reference| reference.span.start <= offset && offset < reference.span.end)
    }

    pub fn references_to(
        &self,
        target: SourceDefinitionId,
    ) -> impl Iterator<Item = &SyntaxReference> {
        self.syntax_references
            .iter()
            .filter(move |reference| reference.target == target)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueReferenceKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueReference {
    pub target: ValueId,
    pub kind: ValueReferenceKind,
    pub span: Span,
    pub expression: Option<ExprId>,
    pub assignment: Option<AssignmentId>,
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceIndex {
    by_value: HashMap<ValueId, Arc<[ValueReference]>>,
}

impl ReferenceIndex {
    fn build(checked: &CheckedProgram) -> Self {
        let mut references = HashMap::<ValueId, Vec<ValueReference>>::new();
        for expression in checked.typed_hir().expressions() {
            let read = checked
                .typed_hir()
                .value_path(expression.id)
                .and_then(|(root, _)| root)
                .or_else(|| {
                    checked
                        .typed_hir()
                        .call(expression.id)
                        .and_then(call_receiver)
                });
            if let Some(read) = read {
                let target = resolved_value_id(read);
                references.entry(target).or_default().push(ValueReference {
                    target,
                    kind: ValueReferenceKind::Read,
                    span: expression.span,
                    expression: Some(expression.id),
                    assignment: None,
                });
            }
        }
        for (assignment, target) in checked.semantics().assignment_targets() {
            let Some(ResolvedAssignment { span, .. }) = checked.typed_hir().assignment(assignment)
            else {
                continue;
            };
            references.entry(target).or_default().push(ValueReference {
                target,
                kind: ValueReferenceKind::Write,
                span,
                expression: None,
                assignment: Some(assignment),
            });
        }
        let by_value = references
            .into_iter()
            .map(|(value, mut references)| {
                references.sort_by_key(|reference| {
                    (reference.span.start, reference.span.end, reference.kind)
                });
                (value, Arc::from(references))
            })
            .collect();
        Self { by_value }
    }

    pub fn references_to(&self, value: ValueId) -> &[ValueReference] {
        self.by_value.get(&value).map_or(&[], AsRef::as_ref)
    }

    pub fn referenced_values(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.by_value.keys().copied()
    }
}

fn resolved_value_id(value: ResolvedValue) -> ValueId {
    match value {
        ResolvedValue::Variable(value)
        | ResolvedValue::CurrentState(value)
        | ResolvedValue::OldState(value)
        | ResolvedValue::Setting(value)
        | ResolvedValue::OldSetting(value) => value,
    }
}

fn call_receiver(call: &ResolvedCall) -> Option<ResolvedValue> {
    match call {
        ResolvedCall::UserMethod { receiver, .. } => Some(*receiver),
        ResolvedCall::StandardLibrary { receiver, .. } => *receiver,
        ResolvedCall::UserFunction { .. }
        | ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::ResultSuccess { .. } => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevision(u64);

impl SourceRevision {
    pub const fn index(self) -> u64 {
        self.0
    }
}

/// Caches the compiler products for one SplitScript source buffer.
///
/// A source change invalidates all dependent queries together. Setting the
/// same text is a no-op, which is useful for editors that deliver redundant
/// document-change notifications.
#[derive(Debug)]
pub struct CompilerDatabase {
    source: String,
    revision: SourceRevision,
    recovered: Option<QueryResult<RecoveredParse>>,
    recovering_lowered: Option<QueryResult<LoweredProgram>>,
    parsed: Option<QueryResult<ParsedProgram>>,
    lowered: Option<QueryResult<LoweredProgram>>,
    checked: Option<QueryResult<CheckedProgram>>,
    recovering_checked: Option<QueryResult<RecoveredCheck>>,
    references: Option<QueryResult<ReferenceIndex>>,
    definitions: Option<QueryResult<DefinitionIndex>>,
    highlights: Option<QueryResult<SemanticHighlightIndex>>,
    document_symbols: Option<QueryResult<Vec<crate::symbols::DocumentSymbol>>>,
    formatted: Option<QueryResult<String>>,
    diagnostics: Option<Arc<[Diagnostic]>>,
}

impl CompilerDatabase {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            revision: SourceRevision::default(),
            recovered: None,
            recovering_lowered: None,
            parsed: None,
            lowered: None,
            checked: None,
            recovering_checked: None,
            references: None,
            definitions: None,
            highlights: None,
            document_symbols: None,
            formatted: None,
            diagnostics: None,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
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
        self.recovered = None;
        self.recovering_lowered = None;
        self.parsed = None;
        self.lowered = None;
        self.checked = None;
        self.recovering_checked = None;
        self.references = None;
        self.definitions = None;
        self.highlights = None;
        self.document_symbols = None;
        self.formatted = None;
        self.diagnostics = None;
        true
    }

    pub fn recovering_parse(&mut self) -> QueryResult<RecoveredParse> {
        if self.recovered.is_none() {
            self.recovered = Some(
                crate::parse_recovering(&self.source)
                    .map(Arc::new)
                    .map_err(Arc::from),
            );
        }
        self.recovered.as_ref().unwrap().clone()
    }

    pub fn parse(&mut self) -> QueryResult<ParsedProgram> {
        if self.parsed.is_none() {
            let parsed = match self.recovering_parse() {
                Ok(recovered) if recovered.diagnostics().is_empty() => {
                    Ok(Arc::new(ParsedProgram {
                        document: recovered.source_document().clone(),
                        syntax: recovered.syntax().clone(),
                    }))
                }
                Ok(recovered) => Err(Arc::from(recovered.diagnostics().to_vec())),
                Err(errors) => Err(errors),
            };
            self.parsed = Some(parsed);
        }
        self.parsed.as_ref().unwrap().clone()
    }

    /// Returns canonical source text for the current valid syntax tree.
    ///
    /// This query shares the database's strict parse and is suitable for an
    /// LSP document-formatting request. Syntax diagnostics are returned
    /// without producing a rewrite.
    pub fn format(&mut self) -> QueryResult<String> {
        if self.formatted.is_none() {
            self.formatted = Some(match self.parse() {
                Ok(parsed) => Ok(Arc::new(crate::formatter::format_parsed(&parsed))),
                Err(errors) => Err(errors),
            });
        }
        self.formatted.as_ref().unwrap().clone()
    }

    /// Returns editor highlighting for every token retained by parsing.
    ///
    /// Semantic resolutions refine names when type checking succeeds. Syntax
    /// recovery still provides lexical and declaration highlighting while a
    /// source buffer is temporarily incomplete.
    pub fn semantic_highlights(&mut self) -> QueryResult<SemanticHighlightIndex> {
        if self.highlights.is_none() {
            let semantics = match self.check() {
                Ok(checked) => Some(checked.semantics().clone()),
                Err(_) => self
                    .recovering_check()
                    .ok()
                    .map(|checked| checked.semantics().clone()),
            };
            self.highlights = Some(match self.recovering_parse() {
                Ok(parsed) => Ok(Arc::new(SemanticHighlightIndex::build(
                    parsed.source_document(),
                    parsed.syntax(),
                    semantics.as_ref(),
                ))),
                Err(errors) => Err(errors),
            });
        }
        self.highlights.as_ref().unwrap().clone()
    }

    /// Returns the recovered, source-ordered document outline. Symbol ranges
    /// and hierarchy belong to the compiler; protocol clients only convert
    /// their byte spans to editor positions.
    pub fn document_symbols(&mut self) -> QueryResult<Vec<crate::symbols::DocumentSymbol>> {
        if self.document_symbols.is_none() {
            self.document_symbols = Some(match self.recovering_parse() {
                Ok(parsed) => Ok(Arc::new(crate::symbols::document_symbols(
                    parsed.source_document(),
                    parsed.syntax(),
                ))),
                Err(errors) => Err(errors),
            });
        }
        self.document_symbols.as_ref().unwrap().clone()
    }

    /// Lowers every declaration retained by recovery, even when syntax
    /// diagnostics make the strict parse query fail.
    pub fn recovering_lower(&mut self) -> QueryResult<LoweredProgram> {
        if self.recovering_lowered.is_none() {
            self.recovering_lowered = Some(match self.recovering_parse() {
                Ok(recovered) => {
                    let syntax = recovered.syntax().clone();
                    Ok(Arc::new(LoweredProgram {
                        document: recovered.source_document().clone(),
                        hir: crate::hir::Program::lower(&syntax),
                        syntax,
                    }))
                }
                Err(errors) => Err(errors),
            });
        }
        self.recovering_lowered.as_ref().unwrap().clone()
    }

    pub fn lower(&mut self) -> QueryResult<LoweredProgram> {
        if self.lowered.is_none() {
            self.lowered = Some(match self.parse() {
                Ok(parsed) => Ok(Arc::new(crate::lower((*parsed).clone()))),
                Err(errors) => Err(errors),
            });
        }
        self.lowered.as_ref().unwrap().clone()
    }

    pub fn check(&mut self) -> QueryResult<CheckedProgram> {
        if self.checked.is_none() {
            self.checked = Some(match self.lower() {
                Ok(lowered) => crate::check((*lowered).clone())
                    .map(Arc::new)
                    .map_err(Arc::from),
                Err(errors) => Err(errors),
            });
        }
        self.checked.as_ref().unwrap().clone()
    }

    pub fn recovering_check(&mut self) -> QueryResult<RecoveredCheck> {
        if self.recovering_checked.is_none() {
            self.recovering_checked = Some(match self.lower() {
                Ok(lowered) => Ok(Arc::new(crate::check_recovering((*lowered).clone()))),
                Err(errors) => Err(errors),
            });
        }
        self.recovering_checked.as_ref().unwrap().clone()
    }

    pub fn declarations_named(&mut self, name: &str) -> SemanticQueryResult<Vec<Declaration>> {
        let lowered = self.lower()?;
        Ok(lowered.hir().declarations_named(name).cloned().collect())
    }

    /// Returns context-sensitive completion candidates at a byte offset.
    pub fn completions(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<crate::completion::CompletionList> {
        crate::completion::complete(self, offset)
    }

    /// Returns catalog-backed hover information at a byte offset.
    pub fn hover(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<crate::insight::HoverInfo>> {
        crate::insight::hover(self, offset)
    }

    /// Returns signature help for the active standard-library call.
    pub fn signature_help(
        &mut self,
        offset: usize,
    ) -> SemanticQueryResult<Option<crate::insight::SignatureHelp>> {
        crate::insight::signature_help(self, offset)
    }

    pub fn expression_type(&mut self, expression: ExprId) -> SemanticQueryResult<Option<TypeId>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().expression_type(expression)),
            Err(_) => Ok(self
                .recovering_check()?
                .semantics()
                .expression_type(expression)),
        }
    }

    pub fn value_type(&mut self, value: ValueId) -> SemanticQueryResult<Option<TypeId>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().value_type(value)),
            Err(_) => Ok(self.recovering_check()?.semantics().value_type(value)),
        }
    }

    pub fn function_result_type(
        &mut self,
        function: FunctionId,
    ) -> SemanticQueryResult<Option<TypeId>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().function_result(function)),
            Err(_) => Ok(self
                .recovering_check()?
                .semantics()
                .function_result(function)),
        }
    }

    pub fn type_kind(&mut self, ty: TypeId) -> SemanticQueryResult<Option<TypeKind>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().types().get(ty).cloned()),
            Err(_) => Ok(self
                .recovering_check()?
                .semantics()
                .types()
                .get(ty)
                .cloned()),
        }
    }

    pub fn resolved_call(
        &mut self,
        expression: ExprId,
    ) -> SemanticQueryResult<Option<ResolvedCall>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().call(expression).cloned()),
            Err(_) => Ok(self
                .recovering_check()?
                .semantics()
                .call(expression)
                .cloned()),
        }
    }

    pub fn resolved_path(
        &mut self,
        expression: ExprId,
    ) -> SemanticQueryResult<Option<ResolvedPath>> {
        match self.check() {
            Ok(checked) => Ok(checked
                .typed_hir()
                .value_path(expression)
                .map(|(root, members)| ResolvedPath {
                    root,
                    members: members.to_vec(),
                })),
            Err(_) => {
                let checked = self.recovering_check()?;
                let root = checked.semantics().value(expression);
                let members = checked.semantics().path_members(expression);
                Ok((root.is_some() || members.is_some()).then(|| ResolvedPath {
                    root,
                    members: members.unwrap_or_default().to_vec(),
                }))
            }
        }
    }

    pub fn assignment_target(
        &mut self,
        assignment: AssignmentId,
    ) -> SemanticQueryResult<Option<ValueId>> {
        match self.check() {
            Ok(checked) => Ok(checked.semantics().assignment_target(assignment)),
            Err(_) => Ok(self
                .recovering_check()?
                .semantics()
                .assignment_target(assignment)),
        }
    }

    pub fn reference_index(&mut self) -> QueryResult<ReferenceIndex> {
        if self.references.is_none() {
            self.references = Some(match self.check() {
                Ok(checked) => Ok(Arc::new(ReferenceIndex::build(&checked))),
                Err(errors) => Err(errors),
            });
        }
        self.references.as_ref().unwrap().clone()
    }

    pub fn definition_index(&mut self) -> QueryResult<DefinitionIndex> {
        if self.definitions.is_none() {
            self.definitions = Some(match self.check() {
                Ok(checked) => Ok(Arc::new(DefinitionIndex::build(&checked))),
                Err(_) => match self.recovering_check() {
                    Ok(checked) => Ok(Arc::new(DefinitionIndex::build_recovered(&checked))),
                    Err(errors) => Err(errors),
                },
            });
        }
        self.definitions.as_ref().unwrap().clone()
    }

    /// Returns the exact non-trivia token containing `offset`.
    pub fn token_at(&mut self, offset: usize) -> SemanticQueryResult<Option<Token>> {
        let recovered = self.recovering_parse()?;
        Ok(recovered.source_document().token_at(offset).cloned())
    }

    /// Returns the smallest typed expression containing `offset`.
    pub fn analysis_at(&mut self, offset: usize) -> SemanticQueryResult<Option<PositionAnalysis>> {
        match self.check() {
            Ok(checked) => {
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
                let segments = expression_segments(&checked, expression);
                Ok(Some(PositionAnalysis {
                    expression: expression.id,
                    span: expression.span,
                    segments,
                    ty: expression.ty,
                    type_kind,
                    resolution: expression.resolution.clone(),
                }))
            }
            Err(_) => {
                let checked = self.recovering_check()?;
                let semantics = checked.semantics();
                let Some(expression) = syntax_expression_at(checked.syntax(), semantics, offset)
                else {
                    return Ok(None);
                };
                let ty = semantics
                    .expression_type(expression.id)
                    .expect("the recovered expression finder only retains typed expressions");
                Ok(Some(PositionAnalysis {
                    expression: expression.id,
                    span: expression.span,
                    segments: syntax_expression_segments(
                        checked.source_document(),
                        checked.enum_types(),
                        expression,
                    ),
                    ty,
                    type_kind: semantics.types().kind(ty).clone(),
                    resolution: syntax_expression_resolution(semantics, expression),
                }))
            }
        }
    }

    /// Resolves the identifier segment under `offset` to either a source
    /// declaration or a compiler catalog symbol.
    pub fn definition_at(
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
            && let Some(target) = definition_for_resolution(&definitions, analysis, segment_index)
        {
            return Ok(Some(target));
        }
        self.language_definition_at(offset, analysis.as_ref())
    }

    /// Returns every exact source occurrence of the declaration under
    /// `offset`. Compiler catalog symbols have no source references and return
    /// an empty list.
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

    /// Returns the source declaration that can be renamed at `offset`.
    /// Language and standard-library catalog symbols are intentionally not
    /// renameable source declarations.
    pub fn rename_target_at(&mut self, offset: usize) -> SemanticQueryResult<Option<RenameTarget>> {
        let definitions = self.definition_index()?;
        if let Some(reference) = definitions.reference_at(offset) {
            return Ok(definitions
                .get(reference.target)
                .map(|definition| RenameTarget {
                    id: definition.id,
                    name: definition.name.clone(),
                    span: reference.span,
                }));
        }
        Ok(match self.definition_at(offset)? {
            Some(DefinitionTarget::Source(definition)) => {
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
        if is_reserved_source_identifier(new_name) {
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
        let mut candidate = Self::new(candidate_source);
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

    fn language_definition_at(
        &mut self,
        offset: usize,
        analysis: Option<&PositionAnalysis>,
    ) -> SemanticQueryResult<Option<DefinitionTarget>> {
        let Some(token) = self.token_at(offset)? else {
            return Ok(None);
        };
        if let TokenKind::Ident(name) = &token.kind
            && let Some(ty) = StandardLibrary::new().type_by_name(name)
        {
            return Ok(Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Type(ty.id),
            )));
        }
        let language = LanguageCatalog::new();
        let item = match &token.kind {
            TokenKind::Ident(name) => language.item_for_source_token(name),
            TokenKind::Question => {
                let propagation = if let Some(analysis) = analysis {
                    match self.check() {
                        Ok(checked) => checked
                            .typed_hir()
                            .expression(analysis.expression)
                            .is_some_and(|expression| {
                                matches!(expression.kind, TypedExpressionKind::Propagate { .. })
                            }),
                        Err(_) => syntax_expression_by_id(
                            self.recovering_check()?.syntax(),
                            analysis.expression,
                        )
                        .is_some_and(|expression| {
                            matches!(expression.kind, ExprKind::Propagate(_))
                        }),
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
            TokenKind::DocComment(_) => Some(language.item(LanguageItemId::SettingDocumentation)),
            TokenKind::TemplateStart | TokenKind::TemplateChunk(_) | TokenKind::TemplateEnd => {
                Some(language.item(LanguageItemId::TemplateString))
            }
            _ => None,
        };
        Ok(item.map(|item| DefinitionTarget::Language(item.id)))
    }

    /// Returns diagnostics from the first failing compiler stage.
    pub fn diagnostics(&mut self) -> Arc<[Diagnostic]> {
        if self.diagnostics.is_none() {
            self.diagnostics = Some(match self.check() {
                Ok(_) => Arc::from(Vec::<Diagnostic>::new()),
                Err(errors) => errors,
            });
        }
        Arc::clone(self.diagnostics.as_ref().unwrap())
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

fn is_reserved_source_identifier(name: &str) -> bool {
    let language_reserved = LanguageCatalog::new().item_for_source_token(name).is_some();
    let standard_library = StandardLibrary::new();
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
    let mut output = String::with_capacity(
        source.len() - removed + spans.len().saturating_mul(replacement.len()),
    );
    let mut cursor = 0;
    for span in spans {
        output.push_str(&source[cursor..span.start]);
        output.push_str(replacement);
        cursor = span.end;
    }
    output.push_str(&source[cursor..]);
    output
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

fn expression_segments(
    checked: &CheckedProgram,
    expression: &TypedExpression,
) -> Vec<IdentifierSegment> {
    let names = match &expression.kind {
        TypedExpressionKind::Path(names) => names.clone(),
        TypedExpressionKind::Member { name, .. } => vec![name.clone()],
        TypedExpressionKind::Call { source_path, .. } => source_path.clone(),
        TypedExpressionKind::Enum {
            enumeration,
            variant,
            ..
        } => enum_type_name(*enumeration, checked.enum_types())
            .map(|enumeration| vec![enumeration, variant.clone()])
            .unwrap_or_default(),
        _ => return Vec::new(),
    };
    let mut tokens = checked.source_document().tokens().filter(|token| {
        expression.span.start <= token.span.start && token.span.end <= expression.span.end
    });
    names
        .iter()
        .filter_map(|name| {
            tokens.find_map(|token| match &token.kind {
                TokenKind::Ident(spelling) if spelling == name => Some(IdentifierSegment {
                    name: name.clone(),
                    span: token.span,
                }),
                _ => None,
            })
        })
        .collect()
}

fn syntax_expression_segments(
    document: &SourceDocument,
    enum_types: &[crate::ast::EnumDecl],
    expression: &crate::ast::Expr,
) -> Vec<IdentifierSegment> {
    let names = match &expression.kind {
        ExprKind::Path(names) => names.clone(),
        ExprKind::Member { name, .. } => vec![name.clone()],
        ExprKind::Call { callee, .. } => callee.clone(),
        ExprKind::Enum {
            enumeration,
            variant,
            ..
        } => enum_type_name(*enumeration, enum_types)
            .map(|enumeration| vec![enumeration, variant.clone()])
            .unwrap_or_default(),
        _ => return Vec::new(),
    };
    let mut tokens = document.tokens().filter(|token| {
        expression.span.start <= token.span.start && token.span.end <= expression.span.end
    });
    names
        .iter()
        .filter_map(|name| {
            tokens.find_map(|token| match &token.kind {
                TokenKind::Ident(spelling) if spelling == name => Some(IdentifierSegment {
                    name: name.clone(),
                    span: token.span,
                }),
                _ => None,
            })
        })
        .collect()
}

fn enum_type_name(enumeration: EnumTypeId, enum_types: &[crate::ast::EnumDecl]) -> Option<String> {
    match enumeration {
        EnumTypeId::Source(id) => enum_types
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|declaration| declaration.name.clone()),
        EnumTypeId::Standard(id) => Some(StandardLibrary::new().type_decl(id).name.to_owned()),
    }
}

fn syntax_expression_resolution(
    semantics: &SemanticModel,
    expression: &crate::ast::Expr,
) -> Option<ExpressionResolution> {
    match &expression.kind {
        ExprKind::Path(_) => Some(ExpressionResolution::ValuePath {
            root: semantics.value(expression.id),
            members: semantics
                .path_members(expression.id)
                .unwrap_or_default()
                .to_vec(),
        }),
        ExprKind::Member { .. } => Some(ExpressionResolution::Member {
            members: semantics
                .path_members(expression.id)
                .unwrap_or_default()
                .to_vec(),
        }),
        ExprKind::Call { .. } => semantics
            .call(expression.id)
            .cloned()
            .map(ExpressionResolution::Call),
        ExprKind::Record { .. } => semantics
            .record_literal_fields(expression.id)
            .map(|fields| ExpressionResolution::RecordLiteral {
                fields: fields.to_vec(),
            }),
        ExprKind::Enum { .. } => semantics
            .enum_variant(expression.id)
            .map(|variant| ExpressionResolution::EnumConstructor { variant }),
        _ => None,
    }
}

struct ExpressionCollector<'ast> {
    expressions: Vec<&'ast crate::ast::Expr>,
}

impl<'ast> Visitor<'ast> for ExpressionCollector<'ast> {
    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        self.expressions.push(expression);
        visit::walk_expr(self, expression);
    }
}

fn syntax_expressions(program: &crate::ast::Program) -> Vec<&crate::ast::Expr> {
    let mut collector = ExpressionCollector {
        expressions: Vec::new(),
    };
    collector.visit_program(program);
    collector.expressions
}

fn syntax_expression_at<'a>(
    program: &'a crate::ast::Program,
    semantics: &SemanticModel,
    offset: usize,
) -> Option<&'a crate::ast::Expr> {
    syntax_expressions(program)
        .into_iter()
        .filter(|expression| {
            expression.span.start <= offset
                && offset < expression.span.end
                && expression.span.start != expression.span.end
                && semantics.expression_type(expression.id).is_some()
        })
        .min_by_key(|expression| {
            (
                expression.span.end - expression.span.start,
                Reverse(expression.id.index()),
            )
        })
}

fn syntax_expression_by_id(program: &crate::ast::Program, id: ExprId) -> Option<&crate::ast::Expr> {
    syntax_expressions(program)
        .into_iter()
        .find(|expression| expression.id == id)
}

fn definition_for_resolution(
    definitions: &DefinitionIndex,
    analysis: &PositionAnalysis,
    segment: usize,
) -> Option<DefinitionTarget> {
    match analysis.resolution.as_ref()? {
        ExpressionResolution::ValuePath { root, members } => {
            definition_for_value_path(definitions, *root, members, segment)
        }
        ExpressionResolution::Member { members } => (segment == 0)
            .then(|| members.first())
            .flatten()
            .and_then(|member| definition_for_member(definitions, member)),
        ExpressionResolution::Call(call) => {
            let callable_segment = analysis.segments.len().checked_sub(1)?;
            if segment == callable_segment {
                return match call {
                    ResolvedCall::UserFunction { function }
                    | ResolvedCall::UserMethod { function, .. } => definitions
                        .get(SourceDefinitionId::Function(*function))
                        .cloned()
                        .map(DefinitionTarget::Source),
                    ResolvedCall::StandardLibrary { item, .. } => {
                        Some(DefinitionTarget::StandardLibrary(*item))
                    }
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
            }
            match call {
                ResolvedCall::UserMethod {
                    receiver,
                    receiver_members,
                    ..
                }
                | ResolvedCall::StandardLibrary {
                    receiver: Some(receiver),
                    receiver_members,
                    ..
                } => definition_for_value_path(
                    definitions,
                    Some(*receiver),
                    receiver_members,
                    segment,
                ),
                ResolvedCall::UserFunction { .. }
                | ResolvedCall::StandardLibrary { receiver: None, .. }
                | ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::ResultSuccess { .. } => None,
            }
        }
        ExpressionResolution::EnumConstructor { variant } => {
            if segment + 1 == analysis.segments.len() {
                match variant {
                    ResolvedEnumVariantId::Source(variant) => definitions
                        .get(SourceDefinitionId::EnumVariant(*variant))
                        .cloned()
                        .map(DefinitionTarget::Source),
                    ResolvedEnumVariantId::Standard(variant) => Some(
                        DefinitionTarget::StandardLibrarySymbol(StdlibSymbolId::Variant(*variant)),
                    ),
                }
            } else {
                definitions
                    .enums
                    .values()
                    .find(|enumeration| {
                        analysis
                            .segments
                            .get(segment)
                            .is_some_and(|source| source.name == enumeration.name)
                    })
                    .cloned()
                    .map(DefinitionTarget::Source)
                    .or_else(|| {
                        StandardLibrary::new()
                            .type_by_name(&analysis.segments[segment].name)
                            .filter(|ty| StandardLibrary::new().variants_of(ty.id).next().is_some())
                            .map(|ty| {
                                DefinitionTarget::StandardLibrarySymbol(StdlibSymbolId::Type(ty.id))
                            })
                    })
            }
        }
        ExpressionResolution::RecordLiteral { .. } => None,
    }
}

fn source_definition_for_resolution(
    segment_count: usize,
    resolution: &ExpressionResolution,
    segment: usize,
) -> Option<SourceDefinitionId> {
    match resolution {
        ExpressionResolution::ValuePath { root, members } => {
            source_definition_for_value_path(*root, members, segment)
        }
        ExpressionResolution::Member { members } => (segment == 0)
            .then(|| members.first())
            .flatten()
            .and_then(source_definition_for_member),
        ExpressionResolution::Call(call) => {
            let callable_segment = segment_count.checked_sub(1)?;
            if segment == callable_segment {
                return match call {
                    ResolvedCall::UserFunction { function }
                    | ResolvedCall::UserMethod { function, .. } => {
                        Some(SourceDefinitionId::Function(*function))
                    }
                    ResolvedCall::StandardLibrary { .. }
                    | ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
            }
            match call {
                ResolvedCall::UserMethod {
                    receiver,
                    receiver_members,
                    ..
                }
                | ResolvedCall::StandardLibrary {
                    receiver: Some(receiver),
                    receiver_members,
                    ..
                } => source_definition_for_value_path(Some(*receiver), receiver_members, segment),
                ResolvedCall::UserFunction { .. }
                | ResolvedCall::StandardLibrary { receiver: None, .. }
                | ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::ResultSuccess { .. } => None,
            }
        }
        ExpressionResolution::EnumConstructor { variant } => {
            (segment + 1 == segment_count).then(|| match variant {
                ResolvedEnumVariantId::Source(variant) => {
                    Some(SourceDefinitionId::EnumVariant(*variant))
                }
                ResolvedEnumVariantId::Standard(_) => None,
            })?
        }
        ExpressionResolution::RecordLiteral { .. } => None,
    }
}

fn source_definition_for_member(member: &ResolvedMember) -> Option<SourceDefinitionId> {
    match member {
        ResolvedMember::RecordField(field) => Some(SourceDefinitionId::RecordField(*field)),
        ResolvedMember::StandardField(_) => None,
    }
}

fn definition_for_member(
    definitions: &DefinitionIndex,
    member: &ResolvedMember,
) -> Option<DefinitionTarget> {
    match member {
        ResolvedMember::RecordField(field) => definitions
            .get(SourceDefinitionId::RecordField(*field))
            .cloned()
            .map(DefinitionTarget::Source),
        ResolvedMember::StandardField(field) => Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(*field),
        )),
    }
}

fn source_definition_for_value_path(
    root: Option<ResolvedValue>,
    members: &[ResolvedMember],
    segment: usize,
) -> Option<SourceDefinitionId> {
    let root = root?;
    let root_segment = match root {
        ResolvedValue::Variable(_) => 0,
        ResolvedValue::CurrentState(_)
        | ResolvedValue::OldState(_)
        | ResolvedValue::Setting(_)
        | ResolvedValue::OldSetting(_) => 1,
    };
    if segment == root_segment {
        return Some(SourceDefinitionId::Value(resolved_value_id(root)));
    }
    let member = segment.checked_sub(root_segment + 1)?;
    match members.get(member)? {
        ResolvedMember::RecordField(field) => Some(SourceDefinitionId::RecordField(*field)),
        ResolvedMember::StandardField(_) => None,
    }
}

fn definition_for_value_path(
    definitions: &DefinitionIndex,
    root: Option<ResolvedValue>,
    members: &[ResolvedMember],
    segment: usize,
) -> Option<DefinitionTarget> {
    let root = root?;
    let root_segment = match root {
        ResolvedValue::Variable(_) => 0,
        ResolvedValue::CurrentState(_)
        | ResolvedValue::OldState(_)
        | ResolvedValue::Setting(_)
        | ResolvedValue::OldSetting(_) => 1,
    };
    if segment < root_segment {
        let item = match root {
            ResolvedValue::CurrentState(_) => LanguageItemId::CurrentSnapshot,
            ResolvedValue::OldState(_) => LanguageItemId::OldSnapshot,
            ResolvedValue::Setting(_) => LanguageItemId::Settings,
            ResolvedValue::OldSetting(_) => LanguageItemId::OldSettingsSnapshot,
            ResolvedValue::Variable(_) => return None,
        };
        return Some(DefinitionTarget::Language(item));
    }
    if segment == root_segment {
        return definitions
            .get(SourceDefinitionId::Value(resolved_value_id(root)))
            .cloned()
            .map(DefinitionTarget::Source);
    }
    let member = segment.checked_sub(root_segment + 1)?;
    match members.get(member)? {
        ResolvedMember::RecordField(field) => definitions
            .get(SourceDefinitionId::RecordField(*field))
            .cloned()
            .map(DefinitionTarget::Source),
        ResolvedMember::StandardField(field) => Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(*field),
        )),
    }
}

struct DefinitionCollector<'a> {
    document: &'a SourceDocument,
    syntax: &'a crate::ast::Program,
    semantics: &'a SemanticModel,
    index: DefinitionIndex,
}

impl DefinitionCollector<'_> {
    fn definition(
        &self,
        id: SourceDefinitionId,
        name: &str,
        declaration_span: Span,
    ) -> Option<SourceDefinition> {
        let span = self
            .document
            .tokens()
            .find(|token| {
                declaration_span.start <= token.span.start
                    && token.span.end <= declaration_span.end
                    && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
            })?
            .span;
        Some(SourceDefinition {
            id,
            name: name.to_owned(),
            span,
        })
    }

    fn insert_value(&mut self, id: ValueId, name: &str, span: Span) {
        if let Some(definition) = self.definition(SourceDefinitionId::Value(id), name, span) {
            self.insert_definition(definition);
        }
    }

    fn insert_definition(&mut self, definition: SourceDefinition) {
        self.index.syntax_references.push(SyntaxReference {
            target: definition.id,
            span: definition.span,
        });
        match definition.id {
            SourceDefinitionId::Value(id) => {
                self.index.values.insert(id, definition);
            }
            SourceDefinitionId::Function(id) => {
                self.index.functions.insert(id, definition);
            }
            SourceDefinitionId::Record(id) => {
                self.index.records.insert(id, definition);
            }
            SourceDefinitionId::RecordField(id) => {
                self.index.record_fields.insert(id, definition);
            }
            SourceDefinitionId::Enum(id) => {
                self.index.enums.insert(id, definition);
            }
            SourceDefinitionId::EnumVariant(id) => {
                self.index.enum_variants.insert(id, definition);
            }
        }
    }

    fn add_reference(&mut self, target: SourceDefinitionId, span: Span) {
        self.index
            .syntax_references
            .push(SyntaxReference { target, span });
    }

    fn add_type_after_colon(&mut self, ty: SyntaxTypeRef, span: Span) {
        let Some((target, name)) = named_type(self.syntax, ty) else {
            return;
        };
        if let Some(span) =
            self.identifier_after(span, |kind| matches!(kind, TokenKind::Colon), name)
        {
            self.add_reference(target, span);
        }
    }

    fn add_type_after_arrow(&mut self, ty: SyntaxTypeRef, span: Span) {
        let Some((target, name)) = named_type(self.syntax, ty) else {
            return;
        };
        let tokens = self.tokens_in(span);
        let Some(arrow) = tokens
            .windows(2)
            .position(|pair| pair[0].kind == TokenKind::Minus && pair[1].kind == TokenKind::Gt)
        else {
            return;
        };
        if let Some(token) = tokens[arrow + 2..]
            .iter()
            .find(|token| matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name))
        {
            self.add_reference(target, token.span);
        }
    }

    fn add_type_after_ident(&mut self, ty: SyntaxTypeRef, span: Span, marker: &str) {
        let Some((target, name)) = named_type(self.syntax, ty) else {
            return;
        };
        if let Some(span) = self.identifier_after(
            span,
            |kind| matches!(kind, TokenKind::Ident(spelling) if spelling == marker),
            name,
        ) {
            self.add_reference(target, span);
        }
    }

    fn identifier_after(
        &self,
        span: Span,
        marker: impl Fn(&TokenKind) -> bool,
        name: &str,
    ) -> Option<Span> {
        let tokens = self.tokens_in(span);
        let marker = tokens.iter().position(|token| marker(&token.kind))?;
        tokens[marker + 1..].iter().find_map(|token| {
            matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
                .then_some(token.span)
        })
    }

    fn tokens_in(&self, span: Span) -> Vec<&Token> {
        self.document
            .tokens()
            .filter(|token| span.start <= token.span.start && token.span.end <= span.end)
            .collect()
    }
}

impl<'ast> Visitor<'ast> for DefinitionCollector<'_> {
    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.insert_value(field.id, &field.name, field.span);
        if let Some(annotation) = field.annotation {
            self.add_type_after_colon(annotation, field.span);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        self.insert_value(setting.id, &setting.name, setting.span);
        if let crate::ast::SettingKind::Choice {
            enumeration,
            options,
            ..
        } = &setting.kind
            && let Some(enumeration_name) = self
                .syntax
                .enums
                .iter()
                .find(|candidate| candidate.id == *enumeration)
                .map(|enumeration| enumeration.name.as_str())
        {
            for option in options {
                let Some(variant) = self.semantics.setting_choice_option(option.id) else {
                    continue;
                };
                let variant_name = self
                    .syntax
                    .enums
                    .iter()
                    .flat_map(|enumeration| &enumeration.variants)
                    .find(|candidate| candidate.id == variant)
                    .map(|variant| variant.name.as_str());
                let (enumeration_span, variant_span) = {
                    let tokens = self.tokens_in(option.span);
                    let enumeration_span = tokens.iter().find_map(|token| {
                        matches!(&token.kind, TokenKind::Ident(spelling) if spelling == enumeration_name)
                            .then_some(token.span)
                    });
                    let variant_span = variant_name.and_then(|variant_name| {
                        tokens.iter().find_map(|token| {
                            matches!(&token.kind, TokenKind::Ident(spelling) if spelling == variant_name)
                                .then_some(token.span)
                        })
                    });
                    (enumeration_span, variant_span)
                };
                if let Some(span) = enumeration_span {
                    self.add_reference(SourceDefinitionId::Enum(*enumeration), span);
                }
                if let Some(span) = variant_span {
                    self.add_reference(SourceDefinitionId::EnumVariant(variant), span);
                }
            }
        }
    }

    fn visit_record(&mut self, record: &'ast crate::ast::RecordDecl) {
        if let Some(definition) = self.definition(
            SourceDefinitionId::Record(record.id),
            &record.name,
            record.span,
        ) {
            self.insert_definition(definition);
        }
        for field in &record.fields {
            if let Some(definition) = self.definition(
                SourceDefinitionId::RecordField(field.id),
                &field.name,
                field.span,
            ) {
                self.insert_definition(definition);
            }
            self.add_type_after_colon(field.ty, field.span);
        }
        visit::walk_record(self, record);
    }

    fn visit_enum(&mut self, enumeration: &'ast crate::ast::EnumDecl) {
        if let Some(definition) = self.definition(
            SourceDefinitionId::Enum(enumeration.id),
            &enumeration.name,
            enumeration.span,
        ) {
            self.insert_definition(definition);
        }
        for variant in &enumeration.variants {
            if let Some(definition) = self.definition(
                SourceDefinitionId::EnumVariant(variant.id),
                &variant.name,
                variant.span,
            ) {
                self.insert_definition(definition);
            }
            if let Some(payload) = variant.payload {
                self.add_type_after_ident(payload, variant.span, &variant.name);
            }
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_function(&mut self, function: &'ast crate::ast::FunctionDecl) {
        if let Some(definition) = self.definition(
            SourceDefinitionId::Function(function.id),
            &function.name,
            function.span,
        ) {
            self.insert_definition(definition);
        }
        if let Some(receiver) = function.method_of
            && let Some((target, name)) = named_type(self.syntax, receiver)
            && let Some(span) = self.tokens_in(function.span).iter().find_map(|token| {
                matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
                    .then_some(token.span)
            })
        {
            self.add_reference(target, span);
        }
        for parameter in &function.params {
            self.insert_value(parameter.id, &parameter.name, parameter.span);
            if (function.method_of.is_none() || parameter.name != "self")
                && let Some(annotation) = parameter.annotation
            {
                self.add_type_after_colon(annotation, parameter.span);
            }
        }
        if let Some(result) = function.return_annotation {
            self.add_type_after_arrow(
                result,
                Span {
                    start: function.span.start,
                    end: function.body.span.start,
                },
            );
        }
        visit::walk_function(self, function);
    }

    fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
        self.insert_value(variable.id, &variable.name, variable.span);
        if let Some(annotation) = variable.annotation {
            self.add_type_after_colon(
                annotation,
                Span {
                    start: variable.span.start,
                    end: variable.value.span.start,
                },
            );
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.insert_value(binding.id, &binding.name, binding.span);
        if let Some(annotation) = &binding.annotation {
            self.add_type_after_colon(*annotation, binding.span);
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
            MatchPattern::Enum { binding: None, .. }
            | MatchPattern::Bool(_)
            | MatchPattern::Int { .. }
            | MatchPattern::None
            | MatchPattern::OptionSome(None)
            | MatchPattern::ResultSuccess(None)
            | MatchPattern::ResultError(None)
            | MatchPattern::Wildcard => None,
        };
        if let Some(binding) = binding {
            self.insert_value(binding.id, &binding.name, arm.span);
        }
        if let MatchPattern::Enum { enumeration, .. } = &arm.pattern {
            let pattern_end = arm
                .guard
                .as_ref()
                .map_or(arm.value.span.start, |guard| guard.span.start);
            let pattern_span = Span {
                start: arm.span.start,
                end: pattern_end,
            };
            if let EnumTypeId::Source(enumeration) = enumeration
                && let Some(enumeration_definition) = self
                    .index
                    .get(SourceDefinitionId::Enum(*enumeration))
                    .cloned()
                && let Some(span) = self.tokens_in(pattern_span).iter().find_map(|token| {
                    matches!(&token.kind, TokenKind::Ident(spelling) if spelling == &enumeration_definition.name)
                        .then_some(token.span)
                })
            {
                self.add_reference(enumeration_definition.id, span);
            }
            let Some(variant_id) = self.semantics.pattern_variant(arm.pattern_id) else {
                visit::walk_match_arm(self, arm);
                return;
            };
            if let ResolvedEnumVariantId::Source(variant_id) = variant_id
                && let Some(variant_definition) = self
                    .index
                    .get(SourceDefinitionId::EnumVariant(variant_id))
                    .cloned()
                && let Some(span) = self.tokens_in(pattern_span).iter().find_map(|token| {
                    matches!(&token.kind, TokenKind::Ident(spelling) if spelling == &variant_definition.name)
                        .then_some(token.span)
                })
            {
                self.add_reference(variant_definition.id, span);
            }
        }
        visit::walk_match_arm(self, arm);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Assign {
            id,
            name,
            value,
            span,
            ..
        } = statement
            && let Some(target) = self.semantics.assignment_target(*id)
            && let Some(identifier) = self
                .tokens_in(Span {
                    start: span.start,
                    end: value.span.start,
                })
                .into_iter()
                .find(|token| matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name))
        {
            self.add_reference(SourceDefinitionId::Value(target), identifier.span);
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        match &expression.kind {
            ExprKind::Record { record, fields } => {
                if let Some(record_name) = self
                    .syntax
                    .records
                    .iter()
                    .find(|candidate| candidate.id == *record)
                    .map(|record| record.name.as_str())
                    && let Some(span) = self.tokens_in(expression.span).iter().find_map(|token| {
                        matches!(&token.kind, TokenKind::Ident(spelling) if spelling == record_name)
                            .then_some(token.span)
                    })
                {
                    self.add_reference(SourceDefinitionId::Record(*record), span);
                }
                let resolved = self
                    .semantics
                    .record_literal_fields(expression.id)
                    .unwrap_or_default();
                let mut start = expression.span.start;
                for ((name, value), field) in fields.iter().zip(resolved) {
                    let label_span = Span {
                        start,
                        end: value.span.start,
                    };
                    if let Some(span) = self.tokens_in(label_span).iter().rev().find_map(|token| {
                        matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
                            .then_some(token.span)
                    }) {
                        self.add_reference(SourceDefinitionId::RecordField(*field), span);
                    }
                    start = value.span.end;
                }
            }
            ExprKind::Cast { target, .. } => {
                self.add_type_after_ident(*target, expression.span, "as");
            }
            ExprKind::Path(_) | ExprKind::Call { .. } => {
                let segments =
                    syntax_expression_segments(self.document, &self.syntax.enums, expression);
                if let Some(resolution) = syntax_expression_resolution(self.semantics, expression) {
                    for (segment, identifier) in segments.iter().enumerate() {
                        if let Some(target) =
                            source_definition_for_resolution(segments.len(), &resolution, segment)
                        {
                            self.add_reference(target, identifier.span);
                        }
                    }
                }
            }
            ExprKind::Member { name_span, .. } => {
                if let Some(member) = self
                    .semantics
                    .path_members(expression.id)
                    .and_then(|members| members.first())
                    && let Some(target) = source_definition_for_member(member)
                {
                    self.add_reference(target, *name_span);
                }
            }
            ExprKind::Enum { enumeration, .. } => {
                let segments =
                    syntax_expression_segments(self.document, &self.syntax.enums, expression);
                if let EnumTypeId::Source(enumeration) = enumeration
                    && let Some(identifier) = segments.first()
                {
                    self.add_reference(SourceDefinitionId::Enum(*enumeration), identifier.span);
                }
                if let Some(ResolvedEnumVariantId::Source(variant)) =
                    self.semantics.enum_variant(expression.id)
                    && let Some(identifier) = segments.get(1)
                {
                    self.add_reference(SourceDefinitionId::EnumVariant(variant), identifier.span);
                }
            }
            ExprKind::Error
            | ExprKind::None
            | ExprKind::Bool(_)
            | ExprKind::Int { .. }
            | ExprKind::Float(_)
            | ExprKind::String(_)
            | ExprKind::InterpolatedString(_)
            | ExprKind::Signature(_)
            | ExprKind::Array(_)
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Fallback { .. }
            | ExprKind::Propagate(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. } => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn named_type(
    syntax: &crate::ast::Program,
    ty: SyntaxTypeRef,
) -> Option<(SourceDefinitionId, &str)> {
    match ty {
        SyntaxTypeRef::Record(id) => syntax
            .records
            .iter()
            .find(|record| record.id == id)
            .map(|record| (SourceDefinitionId::Record(id), record.name.as_str())),
        SyntaxTypeRef::Enum(id) => syntax
            .enums
            .iter()
            .find(|enumeration| enumeration.id == id)
            .map(|enumeration| (SourceDefinitionId::Enum(id), enumeration.name.as_str())),
        SyntaxTypeRef::Array(id) => syntax
            .array_types
            .iter()
            .find(|array| array.id == id)
            .and_then(|array| named_type(syntax, array.element)),
        SyntaxTypeRef::Option(id) => syntax
            .option_types
            .iter()
            .find(|option| option.id == id)
            .and_then(|option| named_type(syntax, option.value)),
        SyntaxTypeRef::Result(id) => syntax
            .result_types
            .iter()
            .find(|result| result.id == id)
            .and_then(|result| named_type(syntax, result.value)),
        SyntaxTypeRef::Named(id) => {
            let name = syntax.type_name(id);
            syntax
                .records
                .iter()
                .find(|record| record.name == name)
                .map(|record| (SourceDefinitionId::Record(record.id), record.name.as_str()))
                .or_else(|| {
                    syntax
                        .enums
                        .iter()
                        .find(|enumeration| enumeration.name == name)
                        .map(|enumeration| {
                            (
                                SourceDefinitionId::Enum(enumeration.id),
                                enumeration.name.as_str(),
                            )
                        })
                })
        }
        SyntaxTypeRef::Void
        | SyntaxTypeRef::Bool
        | SyntaxTypeRef::I8
        | SyntaxTypeRef::U8
        | SyntaxTypeRef::I16
        | SyntaxTypeRef::U16
        | SyntaxTypeRef::I32
        | SyntaxTypeRef::U32
        | SyntaxTypeRef::I64
        | SyntaxTypeRef::U64
        | SyntaxTypeRef::Address
        | SyntaxTypeRef::F32
        | SyntaxTypeRef::F64
        | SyntaxTypeRef::Standard(_) => None,
    }
}

impl Default for CompilerDatabase {
    fn default() -> Self {
        Self::new(String::new())
    }
}
