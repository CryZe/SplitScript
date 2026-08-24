//! Revisioned, single-source compiler queries for editor and tooling clients.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod cache;
mod position;
mod queries;
mod references;
mod rename;
mod snapshot;

pub use position::{IdentifierSegment, PositionAnalysis};
pub use queries::{CompilerDatabase, SourceRevision};
pub use references::{ReferenceIndex, ValueReference, ValueReferenceKind};
pub use rename::{RenameError, RenamePlan, RenameTarget};
pub use snapshot::SemanticSnapshot;

use position::{syntax_expression_resolution, syntax_expression_segments};

use crate::{
    CheckedProgram, Diagnostic, RecoveredCheck,
    ast::{
        EnumId, EnumVariantId, ExprKind, FunctionId, MatchPattern, RecordFieldId, RecordId, Span,
        Stmt, TypeRef as SyntaxTypeRef, ValueId,
    },
    hir::ExpressionResolution,
    language::LanguageItemId,
    lexer::{Token, TokenKind},
    semantic::{ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::{StandardLibrary, StdlibItemId, StdlibSymbolId},
    syntax::SourceDocument,
    visit::{self, Visitor},
};

pub type QueryResult<T> = Result<Arc<T>, Arc<[Diagnostic]>>;
pub type SemanticQueryResult<T> = Result<T, Arc<[Diagnostic]>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub root: Option<ResolvedValue>,
    pub members: Vec<ResolvedMember>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceDefinitionId {
    State,
    Settings,
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
pub enum DefinitionTarget {
    Source(SourceDefinition),
    StandardLibrary(StdlibItemId),
    StandardLibrarySymbol(StdlibSymbolId),
    Language(LanguageItemId),
}

#[derive(Debug, Clone, Default)]
pub struct DefinitionIndex {
    state: Option<SourceDefinition>,
    settings: Option<SourceDefinition>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentHighlightKind {
    Text,
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentHighlight {
    pub span: Span,
    pub kind: DocumentHighlightKind,
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
        // The semantic model can contain compiler-provided values such as the
        // implicit method receiver. They intentionally have no source
        // declaration, so they must not masquerade as dangling source
        // references and prevent language-symbol fallback in editor queries.
        let mut defined = HashSet::new();
        if collector.index.state.is_some() {
            defined.insert(SourceDefinitionId::State);
        }
        if collector.index.settings.is_some() {
            defined.insert(SourceDefinitionId::Settings);
        }
        defined.extend(
            collector
                .index
                .values
                .keys()
                .copied()
                .map(SourceDefinitionId::Value),
        );
        defined.extend(
            collector
                .index
                .functions
                .keys()
                .copied()
                .map(SourceDefinitionId::Function),
        );
        defined.extend(
            collector
                .index
                .records
                .keys()
                .copied()
                .map(SourceDefinitionId::Record),
        );
        defined.extend(
            collector
                .index
                .record_fields
                .keys()
                .copied()
                .map(SourceDefinitionId::RecordField),
        );
        defined.extend(
            collector
                .index
                .enums
                .keys()
                .copied()
                .map(SourceDefinitionId::Enum),
        );
        defined.extend(
            collector
                .index
                .enum_variants
                .keys()
                .copied()
                .map(SourceDefinitionId::EnumVariant),
        );
        collector
            .index
            .syntax_references
            .retain(|reference| defined.contains(&reference.target));
        collector
            .index
            .syntax_references
            .sort_by_key(|reference| (reference.span.start, reference.span.end));
        collector.index.syntax_references.dedup();
        collector.index
    }

    pub fn get(&self, id: SourceDefinitionId) -> Option<&SourceDefinition> {
        match id {
            SourceDefinitionId::State => self.state.as_ref(),
            SourceDefinitionId::Settings => self.settings.as_ref(),
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

fn definition_for_resolution(
    definitions: &DefinitionIndex,
    analysis: &PositionAnalysis,
    segment: usize,
    standard_library: StandardLibrary,
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
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => definitions
                        .get(SourceDefinitionId::Function(*function))
                        .cloned()
                        .map(DefinitionTarget::Source),
                    ResolvedCall::StandardLibrary { item, .. } => {
                        Some(DefinitionTarget::StandardLibrary(*item))
                    }
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
            }
            match call {
                ResolvedCall::UserMethod { receiver, .. }
                | ResolvedCall::StandardLibrary {
                    receiver: Some(receiver),
                    ..
                } => receiver
                    .path()
                    .and_then(|(root, members)| {
                        definition_for_value_path(definitions, Some(root), members, segment)
                    })
                    .or_else(|| {
                        receiver
                            .members()
                            .get(segment)
                            .and_then(|member| definition_for_member(definitions, member))
                    }),
                ResolvedCall::UserFunction { .. }
                | ResolvedCall::StandardLibrary { receiver: None, .. }
                | ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::IteratorItem { .. }
                | ResolvedCall::ResultSuccess { .. } => None,
            }
        }
        ExpressionResolution::DynamicCall(callee) => match callee {
            crate::semantic::DynamicCallCallee::Value(value) => definitions
                .get(SourceDefinitionId::Value(*value))
                .cloned()
                .map(DefinitionTarget::Source),
            crate::semantic::DynamicCallCallee::Expression(_) => None,
        },
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
                        standard_library
                            .type_by_name(&analysis.segments[segment].name)
                            .filter(|ty| standard_library.variants_of(ty.id).next().is_some())
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
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => {
                        Some(SourceDefinitionId::Function(*function))
                    }
                    ResolvedCall::StandardLibrary { .. }
                    | ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
            }
            match call {
                ResolvedCall::UserMethod { receiver, .. }
                | ResolvedCall::StandardLibrary {
                    receiver: Some(receiver),
                    ..
                } => receiver
                    .path()
                    .and_then(|(root, members)| {
                        source_definition_for_value_path(Some(root), members, segment)
                    })
                    .or_else(|| {
                        receiver
                            .members()
                            .get(segment)
                            .and_then(source_definition_for_member)
                    }),
                ResolvedCall::UserFunction { .. }
                | ResolvedCall::StandardLibrary { receiver: None, .. }
                | ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::IteratorItem { .. }
                | ResolvedCall::ResultSuccess { .. } => None,
            }
        }
        ExpressionResolution::DynamicCall(callee) => match callee {
            crate::semantic::DynamicCallCallee::Value(value) => {
                Some(SourceDefinitionId::Value(*value))
            }
            crate::semantic::DynamicCallCallee::Expression(_) => None,
        },
        ExpressionResolution::EnumConstructor { variant } => (segment + 1 == segment_count)
            .then_some(match variant {
                ResolvedEnumVariantId::Source(variant) => {
                    Some(SourceDefinitionId::EnumVariant(*variant))
                }
                ResolvedEnumVariantId::Standard(_) => None,
            })?,
        ExpressionResolution::RecordLiteral { .. } => None,
    }
}

fn source_definition_for_member(member: &ResolvedMember) -> Option<SourceDefinitionId> {
    match member {
        ResolvedMember::StateField(field) | ResolvedMember::SettingField(field) => {
            Some(SourceDefinitionId::Value(*field))
        }
        ResolvedMember::RecordField(field) => Some(SourceDefinitionId::RecordField(*field)),
        ResolvedMember::StandardField(_) => None,
    }
}

fn definition_for_member(
    definitions: &DefinitionIndex,
    member: &ResolvedMember,
) -> Option<DefinitionTarget> {
    match member {
        ResolvedMember::StateField(field) | ResolvedMember::SettingField(field) => definitions
            .get(SourceDefinitionId::Value(*field))
            .cloned()
            .map(DefinitionTarget::Source),
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
    if matches!(
        root,
        ResolvedValue::CurrentSnapshot | ResolvedValue::OldSnapshot
    ) {
        return (segment == 0).then_some(SourceDefinitionId::State);
    }
    if matches!(
        root,
        ResolvedValue::SettingsView | ResolvedValue::OldSettingsView
    ) {
        if segment == 0 {
            return Some(SourceDefinitionId::Settings);
        }
        return match members.get(segment - 1)? {
            ResolvedMember::SettingField(setting) => Some(SourceDefinitionId::Value(*setting)),
            member => source_definition_for_member(member),
        };
    }
    let root_segment = match root {
        ResolvedValue::ProviderValue(_) => 0,
        ResolvedValue::Variable(_) => 0,
        ResolvedValue::CurrentSnapshot
        | ResolvedValue::OldSnapshot
        | ResolvedValue::SettingsView
        | ResolvedValue::OldSettingsView => unreachable!(),
        ResolvedValue::CurrentState(_)
        | ResolvedValue::OldState(_)
        | ResolvedValue::Setting(_)
        | ResolvedValue::OldSetting(_) => 1,
    };
    if segment < root_segment {
        return match root {
            ResolvedValue::CurrentState(_) | ResolvedValue::OldState(_) => {
                Some(SourceDefinitionId::State)
            }
            ResolvedValue::Setting(_) | ResolvedValue::OldSetting(_) => {
                Some(SourceDefinitionId::Settings)
            }
            ResolvedValue::ProviderValue(_)
            | ResolvedValue::Variable(_)
            | ResolvedValue::CurrentSnapshot
            | ResolvedValue::OldSnapshot
            | ResolvedValue::SettingsView
            | ResolvedValue::OldSettingsView => None,
        };
    }
    if segment == root_segment {
        if matches!(root, ResolvedValue::ProviderValue(_)) {
            return Some(SourceDefinitionId::State);
        }
        return root.source_value().map(SourceDefinitionId::Value);
    }
    let member = segment.checked_sub(root_segment + 1)?;
    match members.get(member)? {
        ResolvedMember::StateField(field) | ResolvedMember::SettingField(field) => {
            Some(SourceDefinitionId::Value(*field))
        }
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
    if matches!(
        root,
        ResolvedValue::CurrentSnapshot | ResolvedValue::OldSnapshot
    ) {
        return (segment == 0)
            .then(|| definitions.get(SourceDefinitionId::State))
            .flatten()
            .cloned()
            .map(DefinitionTarget::Source);
    }
    if matches!(
        root,
        ResolvedValue::SettingsView | ResolvedValue::OldSettingsView
    ) {
        let definition = if segment == 0 {
            SourceDefinitionId::Settings
        } else {
            let ResolvedMember::SettingField(setting) = members.get(segment - 1)? else {
                return members
                    .get(segment - 1)
                    .and_then(|member| definition_for_member(definitions, member));
            };
            SourceDefinitionId::Value(*setting)
        };
        return definitions
            .get(definition)
            .cloned()
            .map(DefinitionTarget::Source);
    }
    let root_segment = match root {
        ResolvedValue::ProviderValue(_) => 0,
        ResolvedValue::Variable(_) => 0,
        ResolvedValue::CurrentSnapshot
        | ResolvedValue::OldSnapshot
        | ResolvedValue::SettingsView
        | ResolvedValue::OldSettingsView => unreachable!(),
        ResolvedValue::CurrentState(_)
        | ResolvedValue::OldState(_)
        | ResolvedValue::Setting(_)
        | ResolvedValue::OldSetting(_) => 1,
    };
    if segment < root_segment {
        let definition = match root {
            ResolvedValue::CurrentState(_) | ResolvedValue::OldState(_) => {
                SourceDefinitionId::State
            }
            ResolvedValue::Setting(_) | ResolvedValue::OldSetting(_) => {
                SourceDefinitionId::Settings
            }
            ResolvedValue::ProviderValue(_)
            | ResolvedValue::Variable(_)
            | ResolvedValue::CurrentSnapshot
            | ResolvedValue::OldSnapshot
            | ResolvedValue::SettingsView
            | ResolvedValue::OldSettingsView => return None,
        };
        return definitions
            .get(definition)
            .cloned()
            .map(DefinitionTarget::Source);
    }
    if segment == root_segment {
        if matches!(root, ResolvedValue::ProviderValue(_)) {
            return definitions
                .get(SourceDefinitionId::State)
                .cloned()
                .map(DefinitionTarget::Source);
        }
        return definitions
            .get(SourceDefinitionId::Value(root.source_value()?))
            .cloned()
            .map(DefinitionTarget::Source);
    }
    let member = segment.checked_sub(root_segment + 1)?;
    match members.get(member)? {
        ResolvedMember::StateField(field) | ResolvedMember::SettingField(field) => definitions
            .get(SourceDefinitionId::Value(*field))
            .cloned()
            .map(DefinitionTarget::Source),
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
            SourceDefinitionId::State => self.index.state = Some(definition),
            SourceDefinitionId::Settings => self.index.settings = Some(definition),
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

    fn insert_domain_definition(&mut self, definition: SourceDefinition) {
        match definition.id {
            SourceDefinitionId::State => self.index.state = Some(definition),
            SourceDefinitionId::Settings => self.index.settings = Some(definition),
            _ => unreachable!("domain definitions are state or settings"),
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
    fn visit_program(&mut self, program: &'ast crate::ast::Program) {
        if let Some(state) = &program.state
            && let Some(definition) =
                self.definition(SourceDefinitionId::State, "state", state.span)
        {
            if let Some(value) = state.layout_value {
                self.index.values.insert(
                    value,
                    SourceDefinition {
                        id: SourceDefinitionId::Value(value),
                        name: "layout".to_owned(),
                        span: definition.span,
                    },
                );
            }
            if let Some(enumeration) = &state.layout_enum {
                self.index.enums.insert(
                    enumeration.id,
                    SourceDefinition {
                        id: SourceDefinitionId::Enum(enumeration.id),
                        name: enumeration.name.clone(),
                        span: definition.span,
                    },
                );
            }
            self.insert_domain_definition(definition);
        }
        if let Some(span) = program.settings_span
            && let Some(definition) =
                self.definition(SourceDefinitionId::Settings, "settings", span)
        {
            self.insert_domain_definition(definition);
        }
        visit::walk_program(self, program);
    }

    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.insert_value(field.id, &field.name, field.span);
        if let Some(annotation) = field.annotation {
            self.add_type_after_colon(annotation, field.span);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        self.insert_value(setting.id, &setting.name, setting.span);
        if let crate::ast::SettingKind::Choice { options, .. } = &setting.kind {
            for option in options {
                let Some(variant) = self.semantics.setting_choice_option(option.id) else {
                    continue;
                };
                let Some(enumeration) = self.syntax.enum_declarations().find(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|candidate| candidate.id == variant)
                }) else {
                    continue;
                };
                let enumeration_name = enumeration.name.as_str();
                let variant_name = enumeration
                    .variants
                    .iter()
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
                    self.add_reference(SourceDefinitionId::Enum(enumeration.id), span);
                }
                if let Some(span) = variant_span {
                    self.add_reference(SourceDefinitionId::EnumVariant(variant), span);
                }
            }
        }
    }

    fn visit_setting_family(&mut self, family: &'ast crate::ast::SettingFamilyDecl) {
        self.insert_value(family.binding_id, &family.binding, family.binding_span);
        for pattern in family.key.iter().chain(std::iter::once(&family.label)) {
            for part in &pattern.parts {
                if let crate::ast::SettingTextPart::Binding { span } = part {
                    self.add_reference(SourceDefinitionId::Value(family.binding_id), *span);
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
            let initializer_start = variable
                .value
                .as_ref()
                .map_or(variable.span.end, |value| value.span.start);
            self.add_type_after_colon(
                annotation,
                Span {
                    start: variable.span.start,
                    end: initializer_start,
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

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.insert_value(binding.id, &binding.name, binding.span);
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        let binding = match &arm.pattern {
            MatchPattern::Enum {
                binding: Some(binding),
                ..
            }
            | MatchPattern::OptionSome(Some(binding))
            | MatchPattern::IteratorItem(Some(binding))
            | MatchPattern::ResultSuccess(Some(binding))
            | MatchPattern::ResultError(Some(binding)) => Some(binding),
            MatchPattern::Enum { binding: None, .. }
            | MatchPattern::Bool(_)
            | MatchPattern::Char(_)
            | MatchPattern::String(_)
            | MatchPattern::Int { .. }
            | MatchPattern::FileVersion(_)
            | MatchPattern::None
            | MatchPattern::IteratorEnd
            | MatchPattern::OptionSome(None)
            | MatchPattern::IteratorItem(None)
            | MatchPattern::ResultSuccess(None)
            | MatchPattern::ResultError(None)
            | MatchPattern::Wildcard => None,
        };
        if let Some(binding) = binding {
            self.insert_value(binding.id, &binding.name, arm.span);
        }
        if matches!(arm.pattern, MatchPattern::Enum { .. }) {
            let pattern_end = arm
                .guard
                .as_ref()
                .map_or(arm.value.span.start, |guard| guard.span.start);
            let pattern_span = Span {
                start: arm.span.start,
                end: pattern_end,
            };
            if let Some(ResolvedEnumVariantId::Source(variant)) =
                self.semantics.pattern_variant(arm.pattern_id)
                && let Some(enumeration) = self.syntax.enum_declarations().find(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|candidate| candidate.id == variant)
                })
                && let Some(enumeration_definition) = self
                    .index
                    .get(SourceDefinitionId::Enum(enumeration.id))
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
            ExprKind::Record {
                name_span, fields, ..
            } => {
                if let Some(crate::semantic::ResolvedRecordId::Source(record)) =
                    self.semantics.record_literal(expression.id)
                {
                    self.add_reference(SourceDefinitionId::Record(record), *name_span);
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
                    }) && let crate::semantic::ResolvedRecordFieldId::Source(field) = field
                    {
                        self.add_reference(SourceDefinitionId::RecordField(*field), span);
                    }
                    start = value.span.end;
                }
            }
            ExprKind::Cast { target, .. } => {
                self.add_type_after_ident(*target, expression.span, "as");
            }
            ExprKind::Path(_) | ExprKind::Call { .. } => {
                let segments = syntax_expression_segments(self.document, expression);
                if let Some(resolution) = syntax_expression_resolution(self.semantics, expression) {
                    if let ExpressionResolution::EnumConstructor {
                        variant: ResolvedEnumVariantId::Source(variant),
                    } = resolution
                        && let Some(identifier) = segments.first()
                        && let Some(enumeration) =
                            self.syntax.enum_declarations().find(|enumeration| {
                                enumeration
                                    .variants
                                    .iter()
                                    .any(|candidate| candidate.id == variant)
                            })
                    {
                        self.add_reference(
                            SourceDefinitionId::Enum(enumeration.id),
                            identifier.span,
                        );
                    }
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
            ExprKind::Error
            | ExprKind::None
            | ExprKind::IteratorEnd
            | ExprKind::Bool(_)
            | ExprKind::Int { .. }
            | ExprKind::Float(_)
            | ExprKind::Char(_)
            | ExprKind::String(_)
            | ExprKind::InterpolatedString(_)
            | ExprKind::Signature(_)
            | ExprKind::Array(_)
            | ExprKind::Range { .. }
            | ExprKind::Block(_)
            | ExprKind::Loop(_)
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Fallback { .. }
            | ExprKind::Break(_)
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Throw(_)
            | ExprKind::Suspend { .. }
            | ExprKind::Propagate(_)
            | ExprKind::Index { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Invoke { .. }
            | ExprKind::Closure { .. } => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn named_type(
    syntax: &crate::ast::Program,
    ty: SyntaxTypeRef,
) -> Option<(SourceDefinitionId, &str)> {
    match ty {
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
        SyntaxTypeRef::Async(id) => syntax
            .async_types
            .iter()
            .find(|future| future.id == id)
            .and_then(|future| named_type(syntax, future.value)),
        SyntaxTypeRef::Application(id) => syntax
            .type_applications
            .iter()
            .find(|application| application.id == id)
            .and_then(|application| {
                application
                    .arguments
                    .iter()
                    .find_map(|argument| named_type(syntax, *argument))
            }),
        SyntaxTypeRef::Range(id) => syntax
            .range_types
            .iter()
            .find(|range| range.id == id)
            .and_then(|range| named_type(syntax, range.lower)),
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
        SyntaxTypeRef::Core(_) | SyntaxTypeRef::Callable(_) => None,
    }
}
