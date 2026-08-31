//! Compiler-owned completion candidates shared by editor frontends.

use std::collections::BTreeMap;

mod settings;
mod state;
mod top_level;
mod types;

use settings::complete_settings_dsl;
use state::complete_state_dsl;
use top_level::{complete_state_header, complete_top_level};
use types::{complete_explicit_type_argument, complete_type_position};

use crate::{
    ast::{
        Block, Expr, ExprKind, MatchPattern, Program, SettingKind, Span, Stmt,
        TypeRef as SyntaxTypeRef,
    },
    catalog::Documentation,
    database::{CompilerDatabase, SemanticQueryResult},
    documentation::{StandardLibraryDocumentation, language_item_uri, symbol_uri},
    effects::{OperationAnalysis, action_has_attached_process, action_has_state_snapshots},
    hir::ExpressionResolution,
    language::{LanguageCatalog, LanguageItem, LanguageItemId, LanguageItemKind},
    lexer::TokenKind,
    semantic::ResolvedCall,
    stdlib::{
        ItemKind, StandardLibrary, StdlibCapabilityId, StdlibItem, StdlibItemId, StdlibNamespace,
        StdlibSymbolId, StdlibTypeConstructorId, StdlibTypeId, TypeRef,
    },
    stdlib_semantic::StandardLibrarySemanticExt,
    syntax::SourceDocument,
    types::TypeKind,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionKind {
    Keyword,
    Snippet,
    Namespace,
    Function,
    Method,
    Variable,
    Setting,
    StateField,
    Property,
    Type,
    Struct,
    Enum,
    EnumMember,
    Constant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    /// Stable compiler-owned reference page for a catalog-backed completion.
    pub documentation_uri: Option<String>,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionList {
    pub replacement: Span,
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, Copy)]
struct ContextAvailability {
    attached_process: bool,
    state_snapshots: bool,
    process_selection: bool,
}

/// Immutable lexical and syntactic facts shared by every completion strategy
/// for one cursor request. The lossless parser already owns this token stream;
/// keeping one indexed view prevents each grammar-specific strategy from
/// lexing and linearly rebuilding it independently.
pub(super) struct CompletionRequest<'a> {
    pub(super) source: &'a str,
    pub(super) syntax: &'a Program,
    pub(super) tokens: Vec<&'a crate::lexer::Token>,
    pub(super) offset: usize,
    pub(super) replacement: Span,
}

impl<'a> CompletionRequest<'a> {
    fn new(document: &'a SourceDocument, syntax: &'a Program, offset: usize) -> Self {
        let source = document.source();
        let offset = floor_char_boundary(source, offset.min(source.len()));
        Self {
            source,
            syntax,
            tokens: document
                .tokens()
                .filter(|token| !matches!(token.kind, TokenKind::Eof))
                .collect(),
            offset,
            replacement: identifier_span(source, offset),
        }
    }
}

pub(crate) fn complete(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<CompletionList> {
    let compiler_context = database.context();
    let standard_library = compiler_context.standard_library();
    // Keep the revision's shared recovery product alive for the entire request.
    // Completion only borrows its source and syntax; cloning the whole program
    // here used to make every query own an unnecessary deep syntax copy.
    let recovered = database.recovering_parse()?;
    let request = CompletionRequest::new(recovered.source_document(), recovered.syntax(), offset);
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    if let Some(completions) = complete_setting_key(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_tick_rate_field(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_settings_dsl(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_state_header(&request, &standard_library) {
        Ok(completions)
    } else if let Some(completions) = complete_state_decoder(source, offset) {
        Ok(completions)
    } else if let Some(completions) = complete_managed_field_modifier(&request) {
        Ok(completions)
    } else if let Some(completions) =
        complete_explicit_type_argument(source, syntax, offset, &standard_library)
    {
        Ok(completions)
    } else if let Some(completions) = complete_type_position(&request, &standard_library) {
        Ok(completions)
    } else if let Some(completions) = complete_state_dsl(&request, &standard_library) {
        Ok(completions)
    } else if let Some(context) = member_context(&request) {
        Ok(complete_member(
            database,
            source,
            syntax,
            &request.tokens,
            context,
            compiler_context,
        ))
    } else {
        let action = syntax
            .actions
            .iter()
            .find(|action| contains_offset(action.body.span, offset))
            .map(|action| action.kind);
        let inside_function = syntax
            .functions
            .iter()
            .any(|function| contains_offset(function.body.span, offset));
        let top_level = action.is_none() && is_top_level_offset(syntax, offset);
        let has_attached_process = action.is_none_or(action_has_attached_process);
        let has_state_snapshots = action
            .map(action_has_state_snapshots)
            .unwrap_or(inside_function);
        let effects = (!has_attached_process || !has_state_snapshots)
            .then(|| {
                completion_operation_analysis(
                    database,
                    source,
                    identifier_span(source, offset),
                    compiler_context,
                )
            })
            .flatten();
        Ok(complete_root(
            source,
            syntax,
            offset,
            standard_library,
            ContextAvailability {
                attached_process: has_attached_process,
                state_snapshots: has_state_snapshots,
                process_selection: action == Some(crate::ast::ActionKind::SelectProcess),
            },
            effects.as_ref(),
            top_level,
        ))
    }
}

fn complete_managed_field_modifier(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    let replacement = request.replacement;
    if !syntax
        .managed_class_declarations()
        .into_iter()
        .any(|class| class.span.start < offset && offset < class.span.end)
    {
        return None;
    }

    let segment_start = source[..replacement.start]
        .rfind(['{', '}', ';'])
        .map_or(0, |index| index + 1);
    let tokens = request
        .tokens
        .iter()
        .copied()
        .filter(|token| segment_start <= token.span.start && token.span.end <= replacement.start)
        .collect::<Vec<_>>();
    let mut identifiers = tokens.iter().filter_map(|token| match &token.kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    });
    if identifiers.next()? != "String" || identifiers.next().is_none() {
        return None;
    }
    if tokens
        .iter()
        .any(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "maxLength"))
    {
        return None;
    }

    let item = LanguageCatalog::new().item(LanguageItemId::ManagedStringMaxLength);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Keyword,
        item,
        "maxLength ${1:64};".to_owned(),
        true,
    ));
    Some(builder.finish())
}

fn complete_setting_key(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    let tokens = &request.tokens;
    let (index, token) = tokens.iter().enumerate().find(|(_, token)| {
        matches!(token.kind, TokenKind::String(_))
            && token.span.start < offset
            && offset < token.span.end
    })?;
    let [root, dot, method, open] = tokens.get(index.checked_sub(4)?..index)? else {
        return None;
    };
    let TokenKind::Ident(root_name) = &root.kind else {
        return None;
    };
    if !matches!(root_name.as_str(), "settings" | "oldSettings")
        || !matches!(dot.kind, TokenKind::Dot)
        || !matches!(open.kind, TokenKind::LParen)
    {
        return None;
    }
    let method = match &method.kind {
        TokenKind::Ident(method) if method == "enabled" || method == "contains" => method.as_str(),
        _ => return None,
    };
    let replacement = Span {
        start: token.span.start + 1,
        end: token.span.end.saturating_sub(1),
    };
    if !(replacement.start <= offset && offset <= replacement.end) {
        return None;
    }
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    for setting in &syntax.settings {
        let compatible = match setting.kind {
            SettingKind::Bool { .. } => true,
            SettingKind::Choice { .. } | SettingKind::File { .. } => method == "contains",
            SettingKind::Title { .. } => false,
        };
        if !compatible {
            continue;
        }
        let kind = match setting.kind {
            SettingKind::Bool { .. } => "boolean setting key",
            SettingKind::Choice { .. } => "choice setting key",
            SettingKind::File { .. } => "file setting key",
            SettingKind::Title { .. } => unreachable!(),
        };
        let direct = if setting.source_visible {
            format!("; directly available as {root_name}.{}", setting.name)
        } else {
            String::new()
        };
        builder.add(CompletionItem {
            label: setting.runtime_key().to_owned(),
            kind: CompletionKind::Setting,
            detail: Some(format!("{kind}{direct}")),
            documentation: setting.tooltip.clone(),
            documentation_uri: None,
            insert_text: escape_string_contents(setting.runtime_key()),
            is_snippet: false,
        });
    }
    Some(builder.finish())
}

fn escape_string_contents(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\'' => escaped.push_str("\\'"),
            character => escaped.push(character),
        }
    }
    escaped
}

fn complete_tick_rate_field(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let offset = request.offset;
    let replacement = request.replacement;
    let tokens = &request.tokens;

    let (open, close) = tokens.iter().enumerate().find_map(|(index, token)| {
        if !matches!(&token.kind, TokenKind::Ident(name) if name == "tickRate") {
            return None;
        }
        let open = tokens[index + 1..]
            .iter()
            .position(|token| matches!(token.kind, TokenKind::LBrace))?
            + index
            + 1;
        let close = matching_closing_brace(tokens, open).unwrap_or(tokens.len());
        let closing_start = tokens
            .get(close)
            .map_or(source.len(), |token| token.span.start);
        (tokens[open].span.end <= offset && offset <= closing_start).then_some((open, close))
    })?;

    let mut depth = 1_u32;
    let mut segment_has_colon = false;
    for token in &tokens[open + 1..close] {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Comma if depth == 1 => segment_has_colon = false,
            TokenKind::Colon if depth == 1 => segment_has_colon = true,
            _ => {}
        }
    }
    if depth != 1 || segment_has_colon {
        return None;
    }

    let mut declared = Vec::new();
    depth = 1;
    for (index, token) in tokens[open + 1..close].iter().enumerate() {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Ident(ref name)
                if depth == 1
                    && tokens
                        .get(open + index + 2)
                        .is_some_and(|next| matches!(next.kind, TokenKind::Colon))
                    && token.span != replacement =>
            {
                declared.push(name.as_str());
            }
            _ => {}
        }
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    if !declared.contains(&"attached") {
        builder.add(CompletionItem {
            label: "attached".to_owned(),
            kind: CompletionKind::Property,
            detail: Some("attached polling rate (Hz)".to_owned()),
            documentation: Some(
                "Polling rate used while a process is attached. Defaults to 120 Hz.".to_owned(),
            ),
            documentation_uri: Some(language_item_uri(LanguageItemId::TickRate)),
            insert_text: "attached: ${1:120},".to_owned(),
            is_snippet: true,
        });
    }
    if !declared.contains(&"detached") {
        builder.add(CompletionItem {
            label: "detached".to_owned(),
            kind: CompletionKind::Property,
            detail: Some("detached polling rate (Hz)".to_owned()),
            documentation: Some(
                "Polling rate used while waiting for a process. Defaults to 1 Hz.".to_owned(),
            ),
            documentation_uri: Some(language_item_uri(LanguageItemId::TickRate)),
            insert_text: "detached: ${1:1},".to_owned(),
            is_snippet: true,
        });
    }
    Some(builder.finish())
}

fn matching_closing_brace(tokens: &[&crate::lexer::Token], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn completion_operation_analysis(
    database: &mut CompilerDatabase,
    source: &str,
    replacement: Span,
    compiler_context: crate::CompilerContext,
) -> Option<OperationAnalysis> {
    if let Some(effects) = database
        .semantic_snapshot()
        .ok()
        .and_then(|snapshot| snapshot.effects().cloned())
    {
        return Some(effects);
    }

    // A partially typed root identifier is normally an unknown expression and
    // prevents typed-HIR effect analysis. Replace only that identifier with a
    // valid inert value; declaration and function IDs remain stable, allowing
    // completion to retain the same transitive operation facts while typing.
    let mut probe_source = source.to_owned();
    probe_source.replace_range(replacement.start..replacement.end, "None");
    let mut probe = CompilerDatabase::with_context(compiler_context, probe_source);
    probe
        .semantic_snapshot()
        .ok()
        .and_then(|snapshot| snapshot.effects().cloned())
}

fn complete_state_decoder(source: &str, offset: usize) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let before = source[..replacement.start].trim_end();
    let before_as = before.strip_suffix("as")?;
    if before_as
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
    {
        return None;
    }
    let field_fragment = before_as
        .rsplit_once(['\n', '{', ';'])
        .map_or(before_as, |(_, fragment)| fragment);
    if !field_fragment.split_whitespace().any(|token| token == "at") {
        return None;
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    let language = LanguageCatalog::new();
    let item = language.item(LanguageItemId::NativeStringDecoder);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Function,
        item,
        "utf8(${1:maxBytes})".to_owned(),
        true,
    ));
    let item = language.item(LanguageItemId::NativeUtf16LeDecoder);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Function,
        item,
        "utf16le(${1:maxUtf16Units})".to_owned(),
        true,
    ));
    Some(builder.finish())
}

#[derive(Debug, Clone)]
struct MemberContext {
    receiver_path: Vec<String>,
    receiver_offset: usize,
    dot: usize,
    prefix: String,
    replacement: Span,
}

struct CompletionBuilder {
    prefix: String,
    replacement: Span,
    items: BTreeMap<String, CompletionItem>,
}

impl CompletionBuilder {
    fn new(prefix: String, replacement: Span) -> Self {
        Self {
            prefix,
            replacement,
            items: BTreeMap::new(),
        }
    }

    fn add(&mut self, item: CompletionItem) {
        if self.accepts(&item) {
            self.items.entry(item.label.clone()).or_insert(item);
        }
    }

    /// Lexical bindings shadow catalog and top-level candidates with the same
    /// spelling, just as they do during name resolution.
    fn add_scoped(&mut self, item: CompletionItem) {
        if self.accepts(&item) {
            self.items.insert(item.label.clone(), item);
        }
    }

    fn accepts(&self, item: &CompletionItem) -> bool {
        item.label
            .to_ascii_lowercase()
            .starts_with(&self.prefix.to_ascii_lowercase())
    }

    fn finish(self) -> CompletionList {
        let mut items = self.items.into_values().collect::<Vec<_>>();
        items.sort_by_key(|item| (item.kind, item.label.to_ascii_lowercase()));
        CompletionList {
            replacement: self.replacement,
            items,
        }
    }
}

fn complete_root(
    source: &str,
    syntax: &Program,
    offset: usize,
    standard_library: StandardLibrary,
    availability: ContextAvailability,
    effects: Option<&OperationAnalysis>,
    top_level: bool,
) -> CompletionList {
    let replacement = identifier_span(source, offset);
    if top_level {
        return complete_top_level(source, syntax, offset);
    }
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);

    for item in LanguageCatalog::new().items() {
        if !availability.state_snapshots
            && matches!(
                item.id,
                LanguageItemId::CurrentSnapshot | LanguageItemId::OldSnapshot
            )
        {
            continue;
        }
        if let Some(completion) = language_completion(item) {
            builder.add(completion);
        }
    }
    let provider = if availability.process_selection {
        standard_library.default_state_provider()
    } else {
        selected_provider(syntax, &standard_library)
    };
    add_root_standard_library(
        &mut builder,
        &standard_library,
        availability.attached_process,
    );
    if availability.attached_process
        && let Some(provider) = provider
    {
        let ty = standard_library.type_decl(provider.process_type);
        builder.add(CompletionItem {
            label: provider.value_name.to_owned(),
            kind: CompletionKind::Variable,
            detail: Some(ty.name.to_owned()),
            documentation: Some(render_documentation(&provider.documentation)),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::StateProvider(provider.id),
                &standard_library,
            )),
            insert_text: provider.value_name.to_owned(),
            is_snippet: false,
        });
        for context in provider.contexts {
            let ty = standard_library.type_decl(context.ty);
            builder.add(CompletionItem {
                label: context.name.to_owned(),
                kind: CompletionKind::Variable,
                detail: Some(ty.name.to_owned()),
                documentation: Some(render_documentation(&context.documentation)),
                documentation_uri: Some(symbol_uri(
                    StdlibSymbolId::Type(context.ty),
                    &standard_library,
                )),
                insert_text: context.name.to_owned(),
                is_snippet: false,
            });
        }
    }
    add_source_declarations(
        &mut builder,
        syntax,
        availability.attached_process,
        availability.state_snapshots,
        effects,
    );
    add_state_source_bindings(&mut builder, syntax, offset);
    add_visible_bindings(&mut builder, syntax, offset);
    builder.finish()
}

fn add_state_source_bindings(builder: &mut CompletionBuilder, syntax: &Program, offset: usize) {
    let Some(state) = &syntax.state else {
        return;
    };
    if !state.layouts.is_empty() {
        let Some(layout) = state.layouts.iter().find(|layout| {
            layout
                .fields
                .iter()
                .any(|field| contains_offset(field.span, offset))
        }) else {
            return;
        };
        for field in &layout.fields {
            builder.add_scoped(simple_completion(
                &field.name,
                CompletionKind::StateField,
                "sibling state field",
            ));
        }
        return;
    }
    let in_common = state
        .fields
        .iter()
        .any(|field| contains_offset(field.span, offset));
    let active_group = state.conditional_fields.iter().find(|group| {
        group
            .fields
            .iter()
            .any(|field| contains_offset(field.span, offset))
    });
    if !in_common && active_group.is_none() {
        return;
    }
    for field in &state.fields {
        builder.add_scoped(simple_completion(
            &field.name,
            CompletionKind::StateField,
            "sibling state field",
        ));
    }
    if let Some(group) = active_group {
        for field in &group.fields {
            builder.add_scoped(simple_completion(
                &field.name,
                CompletionKind::StateField,
                "conditional sibling state field",
            ));
        }
    }
}

fn layout_selector_completion(state: &crate::ast::StateDecl) -> CompletionItem {
    let checks = state
        .layout_enum
        .as_ref()
        .expect("named layouts have a generated enum")
        .variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let placeholder = index + 1;
            format!(
                "    if ${{{placeholder}:{} build check}} {{\n        return StateLayout.{}\n    }}\n",
                variant.name, variant.name
            )
        })
        .collect::<String>();
    let item = LanguageCatalog::new().action(crate::ast::ActionKind::OnAttach);
    catalog_language_completion(
        item.name,
        CompletionKind::Snippet,
        item,
        format!("onAttach {{\n{checks}    $0\n    await process.closed()\n}}"),
        true,
    )
}

fn is_top_level_offset(syntax: &Program, offset: usize) -> bool {
    !syntax
        .actions
        .iter()
        .any(|action| contains_offset(action.body.span, offset))
        && !syntax
            .functions
            .iter()
            .any(|function| contains_offset(function.body.span, offset))
        && syntax
            .state
            .as_ref()
            .is_none_or(|state| !contains_offset(state.span, offset))
        && syntax
            .settings
            .iter()
            .all(|setting| !contains_offset(setting.span, offset))
        && syntax
            .structs
            .iter()
            .all(|structure| !contains_offset(structure.span, offset))
        && syntax
            .enums
            .iter()
            .all(|enumeration| !contains_offset(enumeration.span, offset))
        && syntax
            .globals
            .iter()
            .all(|global| !contains_offset(global.span, offset))
}

fn complete_member(
    database: &mut CompilerDatabase,
    source: &str,
    syntax: &Program,
    tokens: &[&crate::lexer::Token],
    context: MemberContext,
    compiler_context: crate::CompilerContext,
) -> CompletionList {
    let standard_library = compiler_context.standard_library();
    let mut builder = CompletionBuilder::new(context.prefix.clone(), context.replacement);
    let active_layout_facts = active_attachment_layout_facts(syntax, context.dot);
    let path = context
        .receiver_path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    match path.as_slice() {
        ["current"] | ["old"] if snapshot_context_available(syntax, context.dot) => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::StateField,
                        "state field",
                    ));
                }
                if let Some(layout) = active_state_layout(syntax, source, tokens, context.dot) {
                    for field in &layout.fields {
                        if !state.is_common_field(&field.name) {
                            builder.add(simple_completion(
                                &field.name,
                                CompletionKind::StateField,
                                "layout-specific state field",
                            ));
                        }
                    }
                }
                for (index, group) in state.conditional_fields.iter().enumerate() {
                    if layout_group_is_active(
                        syntax,
                        &state.conditional_fields,
                        index,
                        &active_layout_facts,
                    ) {
                        for field in &group.fields {
                            builder.add(simple_completion(
                                &field.name,
                                CompletionKind::StateField,
                                "conditional state field",
                            ));
                        }
                    }
                }
            }
        }
        ["settings"] | ["oldSettings"] => {
            for setting in &syntax.settings {
                if setting.source_visible && !matches!(setting.kind, SettingKind::Title { .. }) {
                    let mut completion = simple_completion(
                        &setting.name,
                        CompletionKind::Setting,
                        &setting.description,
                    );
                    completion.documentation = setting.tooltip.clone();
                    builder.add(completion);
                }
            }
        }
        [name] => {
            if let Some(enumeration) = syntax.enum_declarations().find(|item| item.name == *name) {
                for variant in &enumeration.variants {
                    let (insert_text, is_snippet) = if variant.payload.is_some() {
                        (format!("{}(${{1:value}})", variant.name), true)
                    } else {
                        (variant.name.clone(), false)
                    };
                    builder.add(CompletionItem {
                        label: variant.name.clone(),
                        kind: CompletionKind::EnumMember,
                        detail: Some(format!("{}.{}", enumeration.name, variant.name)),
                        documentation: None,
                        documentation_uri: None,
                        insert_text,
                        is_snippet,
                    });
                }
            }
            if let Some(provider) = selected_provider_at(syntax, &standard_library, context.dot)
                && provider.value_name == *name
            {
                add_inferred_fields(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &standard_library,
                    &active_layout_facts,
                );
                add_inferred_methods(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &[],
                    &standard_library,
                );
            }
            if let Some(context) = selected_provider_at(syntax, &standard_library, context.dot)
                .and_then(|provider| {
                    provider
                        .contexts
                        .iter()
                        .find(|context| context.name == *name)
                })
            {
                add_inferred_fields(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(context.ty),
                    &standard_library,
                    &active_layout_facts,
                );
                add_inferred_methods(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(context.ty),
                    &[],
                    &standard_library,
                );
            }
            if let Some(class) = syntax
                .managed_class_declarations()
                .into_iter()
                .find(|class| class.name == *name)
            {
                builder.add(CompletionItem {
                    label: "instances".to_owned(),
                    kind: CompletionKind::Method,
                    detail: Some(format!(
                        "{}.instances() -> async [{}\u{2e}Ref]",
                        class.name, class.name
                    )),
                    documentation: Some(
                        "Cooperatively scans writable process memory and returns a completed snapshot of live references to this managed class."
                            .to_owned(),
                    ),
                    documentation_uri: None,
                    insert_text: "instances()".to_owned(),
                    is_snippet: false,
                });
                for field in class.fields.iter().filter(|field| field.is_static) {
                    let mut completion = simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "managed static field",
                    );
                    completion.documentation = field.documentation.clone();
                    builder.add(completion);
                }
                for (index, group) in class.conditional_fields.iter().enumerate() {
                    if layout_group_is_active(
                        syntax,
                        &class.conditional_fields,
                        index,
                        &active_layout_facts,
                    ) {
                        for field in group.fields.iter().filter(|field| field.is_static) {
                            let mut completion = simple_completion(
                                &field.name,
                                CompletionKind::Property,
                                "conditional managed static field",
                            );
                            completion.documentation = field.documentation.clone();
                            builder.add(completion);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let enum_name = path.join(".");
    if let Some(enumeration) = syntax
        .enum_declarations()
        .find(|item| item.name == enum_name)
    {
        for variant in &enumeration.variants {
            builder.add(CompletionItem {
                label: variant.name.clone(),
                kind: CompletionKind::EnumMember,
                detail: Some(format!("{}.{}", enumeration.name, variant.name)),
                documentation: variant.documentation.clone(),
                documentation_uri: None,
                insert_text: variant.name.clone(),
                is_snippet: false,
            });
        }
    }

    if !path.is_empty() {
        add_standard_library_path_members(&mut builder, &path, &standard_library);
    }

    if let Some(receiver) = infer_receiver(database, source, tokens, &context, compiler_context) {
        add_inferred_fields(
            &mut builder,
            syntax,
            &receiver.ty,
            &standard_library,
            &active_layout_facts,
        );
        add_inferred_methods(
            &mut builder,
            syntax,
            &receiver.ty,
            &receiver.constraints,
            &standard_library,
        );
    }
    builder.finish()
}

fn selected_provider(
    syntax: &Program,
    standard_library: &StandardLibrary,
) -> Option<&'static crate::stdlib::StdlibStateProvider> {
    let state = syntax.state.as_ref()?;
    state
        .provider
        .as_ref()
        .and_then(|provider| standard_library.state_provider_by_name(&provider.name))
        .or_else(|| {
            state
                .provider
                .is_none()
                .then(|| standard_library.default_state_provider())
                .flatten()
        })
}

fn selected_provider_at(
    syntax: &Program,
    standard_library: &StandardLibrary,
    offset: usize,
) -> Option<&'static crate::stdlib::StdlibStateProvider> {
    if syntax.actions.iter().any(|action| {
        action.kind == crate::ast::ActionKind::SelectProcess
            && contains_offset(action.body.span, offset)
    }) {
        standard_library.default_state_provider()
    } else {
        selected_provider(syntax, standard_library)
    }
}

fn language_completion(item: &LanguageItem) -> Option<CompletionItem> {
    if matches!(
        item.id,
        LanguageItemId::NativeStringDecoder | LanguageItemId::NativeUtf16LeDecoder
    ) {
        return None;
    }
    let (kind, insert_text, is_snippet) = match item.kind {
        LanguageItemKind::Action(_) => (
            CompletionKind::Snippet,
            format!("{} {{\n    $0\n}}", item.name),
            true,
        ),
        LanguageItemKind::BuiltinType(_) => (CompletionKind::Type, item.name.to_owned(), false),
        LanguageItemKind::SnapshotRoot => (CompletionKind::Variable, item.name.to_owned(), false),
        LanguageItemKind::Keyword | LanguageItemKind::Declaration => match item.name {
            "for" => (
                CompletionKind::Snippet,
                "for ${1:value} in ${2:values} {\n    $0\n}".to_owned(),
                true,
            ),
            "loop" => (
                CompletionKind::Snippet,
                "loop {\n    $0\n}".to_owned(),
                true,
            ),
            _ => (CompletionKind::Keyword, item.name.to_owned(), false),
        },
        LanguageItemKind::Syntax if is_identifier(item.name) => {
            let (insert, snippet) = match item.name {
                "Some" | "Ok" | "Err" => (format!("{}(${{1:value}})", item.name), true),
                "sig" => ("sig\"${1:pattern}\"".to_owned(), true),
                _ => (item.name.to_owned(), false),
            };
            (CompletionKind::Keyword, insert, snippet)
        }
        LanguageItemKind::Syntax => return None,
    };
    Some(catalog_language_completion(
        item.name,
        kind,
        item,
        insert_text,
        is_snippet,
    ))
}

fn catalog_language_completion(
    label: &str,
    kind: CompletionKind,
    item: &LanguageItem,
    insert_text: String,
    is_snippet: bool,
) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(item.form.to_owned()),
        documentation: Some(render_documentation(&item.documentation)),
        documentation_uri: Some(language_item_uri(item.id)),
        insert_text,
        is_snippet,
    }
}

fn add_root_standard_library(
    builder: &mut CompletionBuilder,
    library: &StandardLibrary,
    has_attached_process: bool,
) {
    for namespace in library
        .namespaces()
        .iter()
        .filter(|namespace| namespace.path.len() == 1)
    {
        builder.add(stdlib_namespace_completion(namespace, library));
    }
    for ty in library.types() {
        builder.add(CompletionItem {
            label: ty.name.to_owned(),
            kind: match ty.kind {
                crate::stdlib::StdlibTypeKind::Intrinsic => CompletionKind::Type,
                crate::stdlib::StdlibTypeKind::Struct => CompletionKind::Struct,
                crate::stdlib::StdlibTypeKind::Enum => CompletionKind::Enum,
            },
            detail: Some("standard-library type".to_owned()),
            documentation: Some(render_documentation(&ty.documentation)),
            documentation_uri: Some(symbol_uri(StdlibSymbolId::Type(ty.id), library)),
            insert_text: ty.name.to_owned(),
            is_snippet: false,
        });
    }
    for item in library.items() {
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() == 1 {
            if !has_attached_process
                && library
                    .operation_semantics(item.id)
                    .requires_attached_process
            {
                continue;
            }
            builder.add(stdlib_completion(
                item.name,
                item,
                CompletionKind::Function,
                library,
            ));
        }
    }
}

fn stdlib_namespace_completion(
    namespace: &StdlibNamespace,
    library: &StandardLibrary,
) -> CompletionItem {
    CompletionItem {
        label: namespace.name.to_owned(),
        kind: CompletionKind::Namespace,
        detail: Some("standard-library namespace".to_owned()),
        documentation: Some(render_documentation(&namespace.documentation)),
        documentation_uri: Some(symbol_uri(StdlibSymbolId::Namespace(namespace.id), library)),
        insert_text: namespace.name.to_owned(),
        is_snippet: false,
    }
}

fn add_standard_library_path_members(
    builder: &mut CompletionBuilder,
    prefix: &[&str],
    library: &StandardLibrary,
) {
    if let [type_name] = prefix
        && let Some(ty) = library.type_by_name(type_name)
    {
        for variant in library.variants_of(ty.id) {
            builder.add(CompletionItem {
                label: variant.name.to_owned(),
                kind: CompletionKind::EnumMember,
                detail: Some(format!("{}.{}", ty.name, variant.name)),
                documentation: Some(render_documentation(&variant.documentation)),
                documentation_uri: Some(symbol_uri(StdlibSymbolId::Variant(variant.id), library)),
                insert_text: variant.name.to_owned(),
                is_snippet: false,
            });
        }
    }

    for item in library.items() {
        if let [type_name] = prefix
            && let Some(constructor) = library.named_type_constructor_by_name(type_name)
            && item.owner == crate::stdlib::StdlibOwner::TypeConstructor(constructor.id)
            && matches!(item.kind, crate::stdlib::ItemKind::Method { .. })
        {
            continue;
        }
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() <= prefix.len() || path[..prefix.len()] != *prefix {
            continue;
        }
        let label = path[prefix.len()];
        if path.len() == prefix.len() + 1 {
            let kind = if item.kind == ItemKind::Constant {
                CompletionKind::Constant
            } else {
                CompletionKind::Function
            };
            builder.add(stdlib_completion(label, item, kind, library));
        }
    }

    for namespace in library.namespaces().iter().filter(|namespace| {
        namespace.path.len() == prefix.len() + 1 && namespace.path[..prefix.len()] == *prefix
    }) {
        builder.add(stdlib_namespace_completion(namespace, library));
    }
}

fn stdlib_completion(
    label: &str,
    item: &StdlibItem,
    kind: CompletionKind,
    library: &StandardLibrary,
) -> CompletionItem {
    let documentation = StandardLibraryDocumentation::generate_with_library(library, item.id, &[]);
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(documentation.signature.clone()),
        documentation: Some(documentation.summary_markdown()),
        documentation_uri: Some(symbol_uri(StdlibSymbolId::Item(item.id), library)),
        insert_text: if item.kind == ItemKind::Constant {
            label.to_owned()
        } else {
            function_snippet(label, item)
        },
        is_snippet: item.kind != ItemKind::Constant,
    }
}

fn function_snippet(label: &str, item: &StdlibItem) -> String {
    let parameters = item
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{label}({parameters})")
}

fn add_source_declarations(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    has_attached_process: bool,
    has_state_snapshots: bool,
    effects: Option<&OperationAnalysis>,
) {
    if syntax
        .state
        .as_ref()
        .is_some_and(|state| state.layout_value.is_some())
    {
        builder.add(simple_completion(
            "layout",
            CompletionKind::Variable,
            "selected state layout",
        ));
    }
    for global in &syntax.globals {
        builder.add(simple_completion(
            &global.name,
            CompletionKind::Variable,
            "global variable",
        ));
    }
    for function in &syntax.functions {
        if function.method_of.is_some() {
            continue;
        }
        if !has_attached_process
            && effects.is_none_or(|effects| effects.function(function.id).requires_attached_process)
        {
            continue;
        }
        if !has_state_snapshots
            && effects.is_none_or(|effects| effects.function(function.id).requires_state_snapshots)
        {
            continue;
        }
        let parameters = function
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
            .collect::<Vec<_>>()
            .join(", ");
        builder.add(CompletionItem {
            label: function.name.clone(),
            kind: CompletionKind::Function,
            detail: Some("user function".to_owned()),
            documentation: None,
            documentation_uri: None,
            insert_text: format!("{}({parameters})", function.name),
            is_snippet: true,
        });
    }
    for structure in &syntax.structs {
        builder.add(simple_completion(
            &structure.name,
            CompletionKind::Struct,
            "struct type",
        ));
    }
    for enumeration in syntax.enum_declarations() {
        builder.add(simple_completion(
            &enumeration.name,
            CompletionKind::Enum,
            "enum type",
        ));
    }
}

fn snapshot_context_available(syntax: &Program, offset: usize) -> bool {
    if syntax
        .functions
        .iter()
        .any(|function| contains_offset(function.body.span, offset))
    {
        return true;
    }
    syntax
        .actions
        .iter()
        .find(|action| contains_offset(action.body.span, offset))
        .is_some_and(|action| action_has_state_snapshots(action.kind))
}

fn add_visible_bindings(builder: &mut CompletionBuilder, syntax: &Program, offset: usize) {
    if let Some(function) = syntax
        .functions
        .iter()
        .find(|function| contains_offset(function.body.span, offset))
    {
        for parameter in &function.params {
            add_scoped_variable(builder, &parameter.name, "parameter");
        }
        add_block_bindings(builder, &function.body, offset);
        return;
    }
    if let Some(action) = syntax
        .actions
        .iter()
        .find(|action| contains_offset(action.body.span, offset))
    {
        add_block_bindings(builder, &action.body, offset);
    }
}

fn add_block_bindings(builder: &mut CompletionBuilder, block: &Block, offset: usize) {
    for statement in &block.statements {
        let span = statement_span(statement);
        if offset < span.start {
            break;
        }
        if offset <= span.end {
            add_statement_inner_bindings(builder, statement, offset);
            break;
        }
        add_completed_statement_binding(builder, statement);
    }
}

fn add_completed_statement_binding(builder: &mut CompletionBuilder, statement: &Stmt) {
    match statement {
        Stmt::Debug { statement, .. } => add_completed_statement_binding(builder, statement),
        Stmt::Variable(variable) => {
            add_scoped_variable(builder, &variable.name, "local variable");
        }
        Stmt::Suspend {
            binding: Some(binding),
            ..
        } => add_scoped_variable(builder, &binding.name, "local variable"),
        Stmt::Assign { .. }
        | Stmt::StateAssign { .. }
        | Stmt::IndexAssign { .. }
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::For { .. }
        | Stmt::Suspend { binding: None, .. }
        | Stmt::Expression(_) => {}
    }
}

fn add_statement_inner_bindings(builder: &mut CompletionBuilder, statement: &Stmt, offset: usize) {
    match statement {
        Stmt::Debug { statement, .. } => {
            if contains_offset(statement_span(statement), offset) {
                add_statement_inner_bindings(builder, statement, offset);
            }
        }
        Stmt::Variable(variable) => {
            if let Some(value) = &variable.value {
                add_expression_bindings(builder, value, offset);
            }
        }
        Stmt::Assign { value, .. } | Stmt::Suspend { value, .. } | Stmt::Expression(value) => {
            add_expression_bindings(builder, value, offset);
        }
        Stmt::StateAssign { target, value, .. } | Stmt::IndexAssign { target, value, .. } => {
            for expression in [target, value] {
                if contains_offset(expression.span, offset) {
                    add_expression_bindings(builder, expression, offset);
                    break;
                }
            }
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            if contains_offset(condition.span, offset) {
                add_expression_bindings(builder, condition, offset);
            } else if contains_offset(then_block.span, offset) {
                add_block_bindings(builder, then_block, offset);
            } else if let Some(else_block) = else_block
                && contains_offset(else_block.span, offset)
            {
                add_block_bindings(builder, else_block, offset);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            if contains_offset(condition.span, offset) {
                add_expression_bindings(builder, condition, offset);
            } else if contains_offset(body.span, offset) {
                add_block_bindings(builder, body, offset);
            }
        }
        Stmt::For {
            binding,
            iterable,
            body,
            ..
        } => {
            if contains_offset(iterable.span, offset) {
                add_expression_bindings(builder, iterable, offset);
            } else if contains_offset(body.span, offset) {
                add_scoped_variable(builder, &binding.name, "loop binding");
                add_block_bindings(builder, body, offset);
            }
        }
    }
}

fn add_expression_bindings(builder: &mut CompletionBuilder, expression: &Expr, offset: usize) {
    if !contains_offset(expression.span, offset) {
        return;
    }
    match &expression.kind {
        ExprKind::Match { value, arms } => {
            if contains_offset(value.span, offset) {
                add_expression_bindings(builder, value, offset);
                return;
            }
            for arm in arms {
                let target = arm
                    .guard
                    .as_ref()
                    .filter(|guard| contains_offset(guard.span, offset))
                    .or_else(|| contains_offset(arm.value.span, offset).then_some(&arm.value));
                if let Some(target) = target {
                    add_pattern_binding(builder, &arm.pattern);
                    add_expression_bindings(builder, target, offset);
                    return;
                }
            }
        }
        ExprKind::InterpolatedString(parts) => {
            for part in parts {
                if let crate::ast::InterpolatedPart::Expr(part) = part {
                    add_expression_bindings(builder, part, offset);
                }
            }
        }
        ExprKind::Array(values) => add_child_expression_bindings(builder, values, offset),
        ExprKind::Range { start, end, .. } => {
            add_expression_bindings(builder, start, offset);
            add_expression_bindings(builder, end, offset);
        }
        ExprKind::Block(block) | ExprKind::Loop(block) => {
            add_block_bindings(builder, block, offset)
        }
        ExprKind::Struct { fields, .. } => {
            for field in fields {
                add_expression_bindings(builder, &field.value, offset);
            }
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            for child in [condition.as_ref(), then_expr.as_ref(), else_expr.as_ref()] {
                add_expression_bindings(builder, child, offset);
            }
        }
        ExprKind::Fallback { value, fallback } => {
            add_expression_bindings(builder, value, offset);
            add_expression_bindings(builder, fallback, offset);
        }
        ExprKind::Break(Some(value))
        | ExprKind::Return(Some(value))
        | ExprKind::Throw(value)
        | ExprKind::Suspend { value, .. }
        | ExprKind::Propagate(value)
        | ExprKind::Member {
            receiver: value, ..
        }
        | ExprKind::Unary { expr: value, .. } => {
            add_expression_bindings(builder, value, offset);
        }
        ExprKind::Index {
            receiver, index, ..
        } => {
            add_expression_bindings(builder, receiver, offset);
            add_expression_bindings(builder, index, offset);
        }
        ExprKind::Cast { expr, .. } => add_expression_bindings(builder, expr, offset),
        ExprKind::Binary { left, right, .. } => {
            add_expression_bindings(builder, left, offset);
            add_expression_bindings(builder, right, offset);
        }
        ExprKind::Call { args, .. } => add_child_expression_bindings(builder, args, offset),
        ExprKind::Invoke { callee, args } => {
            add_expression_bindings(builder, callee, offset);
            add_child_expression_bindings(builder, args, offset);
        }
        ExprKind::Closure { params, body, .. } => {
            for parameter in params {
                add_scoped_variable(builder, &parameter.name, "closure parameter");
            }
            add_expression_bindings(builder, body, offset);
        }
        ExprKind::Error
        | ExprKind::None
        | ExprKind::Break(None)
        | ExprKind::Continue
        | ExprKind::IteratorEnd
        | ExprKind::Return(None)
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::Signature(_)
        | ExprKind::Path(_) => {}
    }
}

fn add_child_expression_bindings(
    builder: &mut CompletionBuilder,
    expressions: &[Expr],
    offset: usize,
) {
    for expression in expressions {
        add_expression_bindings(builder, expression, offset);
    }
}

fn add_pattern_binding(builder: &mut CompletionBuilder, pattern: &MatchPattern) {
    let binding = match pattern {
        MatchPattern::Enum { binding, .. }
        | MatchPattern::OptionSome(binding)
        | MatchPattern::IteratorItem(binding)
        | MatchPattern::ResultSuccess(binding)
        | MatchPattern::ResultError(binding) => binding.as_ref(),
        MatchPattern::Bool(_)
        | MatchPattern::Char(_)
        | MatchPattern::String(_)
        | MatchPattern::Int { .. }
        | MatchPattern::FileVersion(_)
        | MatchPattern::None
        | MatchPattern::IteratorEnd
        | MatchPattern::Wildcard => None,
    };
    if let Some(binding) = binding {
        add_scoped_variable(builder, &binding.name, "pattern binding");
    }
}

fn add_scoped_variable(builder: &mut CompletionBuilder, name: &str, detail: &str) {
    builder.add_scoped(simple_completion(name, CompletionKind::Variable, detail));
}

fn contains_offset(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Debug { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::StateAssign { span, .. }
        | Stmt::IndexAssign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Suspend { span, .. } => *span,
        Stmt::Variable(variable) => variable.span,
        Stmt::Expression(expression) => expression.span,
    }
}

fn add_inferred_fields(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    receiver: &TypeKind,
    standard_library: &StandardLibrary,
    active_layout_facts: &[(crate::ast::StructFieldId, crate::ast::EnumVariantId)],
) {
    match receiver {
        TypeKind::Error => {}
        TypeKind::StateSnapshot => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "state field",
                    ));
                }
                for (index, group) in state.conditional_fields.iter().enumerate() {
                    if layout_group_is_active(
                        syntax,
                        &state.conditional_fields,
                        index,
                        active_layout_facts,
                    ) {
                        for field in &group.fields {
                            builder.add(simple_completion(
                                &field.name,
                                CompletionKind::Property,
                                "conditional state field",
                            ));
                        }
                    }
                }
            }
        }
        TypeKind::SettingsView => {
            for setting in &syntax.settings {
                if setting.source_visible && !matches!(setting.kind, SettingKind::Title { .. }) {
                    let mut completion = simple_completion(
                        &setting.name,
                        CompletionKind::Setting,
                        &setting.description,
                    );
                    completion.documentation = setting.tooltip.clone();
                    builder.add(completion);
                }
            }
        }
        TypeKind::Struct(id) => {
            if let Some(structure) = syntax.structs.iter().find(|structure| structure.id == *id) {
                for field in &structure.fields {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "struct field",
                    ));
                }
            }
        }
        TypeKind::ManagedClass(id) | TypeKind::ManagedReference(id) => {
            if let Some(class) = syntax.managed_class(*id) {
                let conditional_fields = class
                    .conditional_fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        layout_group_is_active(
                            syntax,
                            &class.conditional_fields,
                            *index,
                            active_layout_facts,
                        )
                    })
                    .flat_map(|(_, group)| &group.fields);
                for field in class
                    .fields
                    .iter()
                    .chain(conditional_fields)
                    .filter(|field| !field.is_static)
                {
                    let mut completion = simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        if matches!(receiver, TypeKind::ManagedReference(_)) {
                            "fallible live managed field"
                        } else {
                            "managed snapshot field"
                        },
                    );
                    completion.documentation = field.documentation.clone();
                    builder.add(completion);
                }
                if matches!(receiver, TypeKind::ManagedReference(_)) {
                    builder.add(CompletionItem {
                        label: "snapshot".to_owned(),
                        kind: CompletionKind::Method,
                        detail: Some(format!("{}.Ref.snapshot() -> {}!", class.name, class.name)),
                        documentation: Some(
                            "Reads every active instance field transactionally and returns one immutable local snapshot. If any field read fails, no partial snapshot is exposed."
                                .to_owned(),
                        ),
                        documentation_uri: None,
                        insert_text: "snapshot()".to_owned(),
                        is_snippet: false,
                    });
                }
            }
        }
        TypeKind::Standard(owner) => {
            for field in standard_library.public_fields(*owner) {
                builder.add(CompletionItem {
                    label: field.name.to_owned(),
                    kind: CompletionKind::Property,
                    detail: Some(format!(
                        "{}.{}",
                        standard_library.type_decl(*owner).name,
                        field.name
                    )),
                    documentation: Some(render_documentation(&field.documentation)),
                    documentation_uri: Some(symbol_uri(
                        StdlibSymbolId::Field(field.id),
                        standard_library,
                    )),
                    insert_text: field.name.to_owned(),
                    is_snippet: false,
                });
            }
        }
        TypeKind::Array { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Array)
        }
        TypeKind::Option { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Option)
        }
        TypeKind::Result { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Result)
        }
        TypeKind::Range { kind, .. } => add_constructor_fields(
            builder,
            standard_library,
            match kind {
                crate::ast::RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                crate::ast::RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
            },
        ),
        TypeKind::Set { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Set)
        }
        TypeKind::Application { constructor, .. } => {
            add_constructor_fields(builder, standard_library, *constructor)
        }
        TypeKind::Builtin(_)
        | TypeKind::Enum(_)
        | TypeKind::GenericParameter { .. }
        | TypeKind::Async { .. }
        | TypeKind::Callable { .. } => {}
    }
}

fn add_constructor_fields(
    builder: &mut CompletionBuilder,
    standard_library: &StandardLibrary,
    owner: StdlibTypeConstructorId,
) {
    for field in standard_library.public_constructor_fields(owner) {
        builder.add(CompletionItem {
            label: field.name.to_owned(),
            kind: CompletionKind::Property,
            detail: Some(format!(
                "{}.{}",
                standard_library.render_field_owner(field.owner),
                field.name
            )),
            documentation: Some(render_documentation(&field.documentation)),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::Field(field.id),
                standard_library,
            )),
            insert_text: field.name.to_owned(),
            is_snippet: false,
        });
    }
}

fn active_state_layout<'a>(
    syntax: &'a Program,
    source: &str,
    tokens: &[&crate::lexer::Token],
    offset: usize,
) -> Option<&'a crate::ast::StateLayoutDecl> {
    struct Finder<'a> {
        syntax: &'a Program,
        offset: usize,
        variant: Option<crate::ast::EnumVariantId>,
        match_start: Option<usize>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if self.variant.is_none()
                && let ExprKind::Match { value, arms } = &expression.kind
                && matches!(&value.kind, ExprKind::Path(path) if path.as_slice() == ["layout"])
                && contains_offset(expression.span, self.offset)
                // Recovering member completion may end the arm's parsed value
                // immediately before the dot. Arm starts and the enclosing
                // match span remain reliable, so choose the latest arm that
                // has begun at the cursor instead of requiring its shortened
                // value span to contain the cursor.
                && let Some(arm) = arms
                    .iter()
                    .rev()
                    .find(|arm| arm.span.start <= self.offset)
                && let MatchPattern::Enum { variant, .. } = &arm.pattern
                && let Some(layout) = self.syntax.state.as_ref().and_then(|state| {
                    state.layouts.iter().find(|layout| {
                        state
                            .layout_enum
                            .as_ref()
                            .and_then(|enumeration| {
                                enumeration
                                    .variants
                                    .iter()
                                    .find(|candidate| candidate.id == layout.variant)
                            })
                            .is_some_and(|candidate| candidate.name == *variant)
                    })
                })
            {
                self.variant = Some(layout.variant);
                self.match_start = Some(expression.span.start);
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        syntax,
        offset,
        variant: None,
        match_start: None,
    };
    finder.visit_program(syntax);
    let variant = finder
        .variant
        .filter(|_| {
            finder
                .match_start
                .is_some_and(|start| cursor_is_inside_braces(tokens, start, offset))
        })
        .or_else(|| {
            let before = source.get(..offset)?;
            let match_start = before.rfind("match layout")?;
            if !cursor_is_inside_braces(tokens, match_start, offset) {
                return None;
            }
            let state = syntax.state.as_ref()?;
            state
                .layouts
                .iter()
                .filter_map(|layout| {
                    let name = state
                        .layout_enum
                        .as_ref()?
                        .variants
                        .iter()
                        .find(|variant| variant.id == layout.variant)?
                        .name
                        .as_str();
                    let marker = format!("StateLayout.{name}");
                    let position = before.rfind(&marker)?;
                    (match_start < position && before[position + marker.len()..].contains("=>"))
                        .then_some((position, layout.variant))
                })
                .max_by_key(|(position, _)| *position)
                .map(|(_, variant)| variant)
        })?;
    syntax
        .state
        .as_ref()?
        .layouts
        .iter()
        .find(|layout| layout.variant == variant)
}

fn active_attachment_layout_facts(
    syntax: &Program,
    offset: usize,
) -> Vec<(crate::ast::StructFieldId, crate::ast::EnumVariantId)> {
    struct Finder<'a> {
        syntax: &'a Program,
        offset: usize,
        facts: Vec<(crate::ast::StructFieldId, crate::ast::EnumVariantId)>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } = statement
            {
                if contains_offset(then_block.span, self.offset) {
                    collect_attachment_layout_facts(self.syntax, condition, &mut self.facts);
                } else if else_block
                    .as_ref()
                    .is_some_and(|block| contains_offset(block.span, self.offset))
                {
                    collect_attachment_layout_falsy_facts(self.syntax, condition, &mut self.facts);
                }
            }
            visit::walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } = &expression.kind
            {
                if contains_offset(then_expr.span, self.offset) {
                    collect_attachment_layout_facts(self.syntax, condition, &mut self.facts);
                } else if contains_offset(else_expr.span, self.offset) {
                    collect_attachment_layout_falsy_facts(self.syntax, condition, &mut self.facts);
                }
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        syntax,
        offset,
        facts: Vec::new(),
    };
    finder.visit_program(syntax);
    finder.facts.sort_by_key(|(field, _)| field.index());
    finder.facts.dedup();
    finder.facts
}

fn layout_group_is_active<Field>(
    syntax: &Program,
    groups: &[crate::ast::ConditionalFieldsDecl<Field>],
    target: usize,
    active: &[(crate::ast::StructFieldId, crate::ast::EnumVariantId)],
) -> bool {
    let mut chain_start = target;
    while chain_start > 0 && groups[chain_start].else_span.is_some() {
        chain_start -= 1;
    }
    for group in &groups[chain_start..target] {
        let Some(condition) = &group.condition else {
            return false;
        };
        if layout_condition_value(syntax, active, condition) != Some(false) {
            return false;
        }
    }
    groups[target]
        .condition
        .as_ref()
        .is_none_or(|condition| layout_condition_value(syntax, active, condition) == Some(true))
}

fn layout_condition_value(
    syntax: &Program,
    active: &[(crate::ast::StructFieldId, crate::ast::EnumVariantId)],
    condition: &Expr,
) -> Option<bool> {
    match &condition.kind {
        ExprKind::Bool(value) => Some(*value),
        ExprKind::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
        } => layout_condition_value(syntax, active, expr).map(|value| !value),
        ExprKind::Binary {
            op: crate::ast::BinaryOp::And,
            left,
            right,
        } => match (
            layout_condition_value(syntax, active, left),
            layout_condition_value(syntax, active, right),
        ) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Or,
            left,
            right,
        } => match (
            layout_condition_value(syntax, active, left),
            layout_condition_value(syntax, active, right),
        ) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Eq | crate::ast::BinaryOp::Ne,
            left,
            right,
        } => {
            let fact = attachment_layout_fact(syntax, left, right)
                .or_else(|| attachment_layout_fact(syntax, right, left))?;
            let selected = active
                .iter()
                .find(|(field, _)| *field == fact.0)
                .map(|(_, variant)| *variant == fact.1)?;
            Some(
                if matches!(
                    condition.kind,
                    ExprKind::Binary {
                        op: crate::ast::BinaryOp::Ne,
                        ..
                    }
                ) {
                    !selected
                } else {
                    selected
                },
            )
        }
        _ => None,
    }
}

fn collect_attachment_layout_facts(
    syntax: &Program,
    expression: &Expr,
    output: &mut Vec<(crate::ast::StructFieldId, crate::ast::EnumVariantId)>,
) {
    match &expression.kind {
        ExprKind::Binary {
            op: crate::ast::BinaryOp::And,
            left,
            right,
        } => {
            collect_attachment_layout_facts(syntax, left, output);
            collect_attachment_layout_facts(syntax, right, output);
        }
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Eq,
            left,
            right,
        } => {
            if let Some(fact) = attachment_layout_fact(syntax, left, right)
                .or_else(|| attachment_layout_fact(syntax, right, left))
            {
                output.push(fact);
            }
        }
        _ => {}
    }
}

fn collect_attachment_layout_falsy_facts(
    syntax: &Program,
    expression: &Expr,
    output: &mut Vec<(crate::ast::StructFieldId, crate::ast::EnumVariantId)>,
) {
    if let ExprKind::Binary {
        op: crate::ast::BinaryOp::Or,
        left,
        right,
    } = &expression.kind
    {
        collect_attachment_layout_falsy_facts(syntax, left, output);
        collect_attachment_layout_falsy_facts(syntax, right, output);
    } else if let Some(fact) = inverse_attachment_layout_fact(syntax, expression) {
        output.push(fact);
    }
}

fn inverse_attachment_layout_fact(
    syntax: &Program,
    expression: &Expr,
) -> Option<(crate::ast::StructFieldId, crate::ast::EnumVariantId)> {
    let ExprKind::Binary {
        op: crate::ast::BinaryOp::Eq | crate::ast::BinaryOp::Ne,
        left,
        right,
    } = &expression.kind
    else {
        return None;
    };
    let selected = attachment_layout_fact(syntax, left, right)
        .or_else(|| attachment_layout_fact(syntax, right, left))?;
    if matches!(
        expression.kind,
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Ne,
            ..
        }
    ) {
        return Some(selected);
    }
    let enumeration = syntax.enum_declarations().find(|enumeration| {
        enumeration
            .variants
            .iter()
            .any(|variant| variant.id == selected.1)
    })?;
    if enumeration.variants.len() != 2 {
        return None;
    }
    let inverse = enumeration
        .variants
        .iter()
        .find(|variant| variant.id != selected.1)?;
    Some((selected.0, inverse.id))
}

fn attachment_layout_fact(
    syntax: &Program,
    dimension: &Expr,
    variant: &Expr,
) -> Option<(crate::ast::StructFieldId, crate::ast::EnumVariantId)> {
    let dimension = completion_expression_path(dimension)?;
    let ["layout", dimension_name] = dimension.as_slice() else {
        return None;
    };
    let variant = completion_expression_path(variant)?;
    let [enum_name, variant_name] = variant.as_slice() else {
        return None;
    };
    let layout = syntax
        .structs
        .iter()
        .find(|structure| structure.name == "Layout")?;
    let field = layout
        .fields
        .iter()
        .find(|field| field.name == *dimension_name)?;
    let enumeration = syntax
        .enum_declarations()
        .find(|enumeration| enumeration.name == *enum_name)?;
    let variant = enumeration
        .variants
        .iter()
        .find(|variant| variant.name == *variant_name)?;
    Some((field.id, variant.id))
}

fn completion_expression_path(expression: &Expr) -> Option<Vec<&str>> {
    match &expression.kind {
        ExprKind::Path(path) => Some(path.iter().map(String::as_str).collect()),
        ExprKind::Member { receiver, name, .. } => {
            let mut path = completion_expression_path(receiver)?;
            path.push(name);
            Some(path)
        }
        _ => None,
    }
}

fn cursor_is_inside_braces(tokens: &[&crate::lexer::Token], start: usize, offset: usize) -> bool {
    let search_start = tokens.partition_point(|token| token.span.start < start);
    let Some(open) = tokens[search_start..]
        .iter()
        .position(|token| matches!(token.kind, TokenKind::LBrace))
        .map(|relative| search_start + relative)
    else {
        return false;
    };
    let mut depth = 0_u32;
    for token in tokens[open..]
        .iter()
        .take_while(|token| token.span.start < offset)
    {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth != 0
}

fn add_inferred_methods(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    receiver: &TypeKind,
    generic_constraints: &[StdlibCapabilityId],
    standard_library: &StandardLibrary,
) {
    if matches!(receiver, TypeKind::Standard(StdlibTypeId::UnityGameObject)) {
        builder.add(CompletionItem {
            label: "component".to_owned(),
            kind: CompletionKind::Method,
            detail: Some("UnityGameObject.component<T>() -> T.Ref!".to_owned()),
            documentation: Some(
                "Finds the managed component whose runtime class matches the declared Unity schema class `T`."
                    .to_owned(),
            ),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::Type(StdlibTypeId::UnityGameObject),
                standard_library,
            )),
            insert_text: "component<${1:Class}>()".to_owned(),
            is_snippet: true,
        });
    }
    let methods = standard_library
        .methods_for_type(receiver)
        .into_iter()
        .filter(|item| {
            !(matches!(
                item.id,
                StdlibItemId::ArrayPush
                    | StdlibItemId::ArrayExtend
                    | StdlibItemId::ArrayRemoveAt
                    | StdlibItemId::ArrayRemove
                    | StdlibItemId::ArrayPop
                    | StdlibItemId::ArrayClear
            ) && matches!(
                receiver,
                TypeKind::Array {
                    length: Some(_),
                    ..
                }
            ))
        })
        .chain(
            matches!(receiver, TypeKind::GenericParameter { .. })
                .then(|| {
                    standard_library
                        .methods()
                        .filter(|item| {
                            let receiver = match item.kind {
                                ItemKind::Method { receiver } => receiver,
                                ItemKind::Function => {
                                    return false;
                                }
                                ItemKind::Constant => return false,
                            };
                            let TypeRef::Parameter(parameter) = receiver else {
                                return false;
                            };
                            item.signature
                                .type_parameters
                                .iter()
                                .find(|candidate| candidate.name == parameter)
                                .is_some_and(|parameter| {
                                    parameter.constraints.iter().all(|constraint| {
                                        standard_library
                                            .capabilities_satisfy(generic_constraints, *constraint)
                                    })
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
    for item in methods {
        let ItemKind::Method { .. } = item.kind else {
            unreachable!()
        };
        builder.add(stdlib_completion(
            item.name,
            item,
            CompletionKind::Method,
            standard_library,
        ));
    }
    for function in &syntax.functions {
        if function.method_of.is_some_and(|declared| {
            syntax_receiver_matches(syntax, declared, receiver, standard_library)
        }) {
            let parameters = function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
                .collect::<Vec<_>>()
                .join(", ");
            builder.add(CompletionItem {
                label: function.name.clone(),
                kind: CompletionKind::Method,
                detail: Some("user method".to_owned()),
                documentation: None,
                documentation_uri: None,
                insert_text: format!("{}({parameters})", function.name),
                is_snippet: true,
            });
        }
    }
}

fn syntax_receiver_matches(
    syntax: &Program,
    declared: SyntaxTypeRef,
    receiver: &TypeKind,
    standard_library: &StandardLibrary,
) -> bool {
    match (declared, receiver) {
        (SyntaxTypeRef::Array(expected), TypeKind::Array { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Option(expected), TypeKind::Option { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Result(expected), TypeKind::Result { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Named(name), TypeKind::Standard(actual)) => standard_library
            .type_by_name(syntax.type_name(name))
            .is_some_and(|expected| expected.id == *actual),
        (SyntaxTypeRef::Named(name), TypeKind::Struct(actual)) => syntax
            .structs
            .iter()
            .any(|structure| structure.name == syntax.type_name(name) && structure.id == *actual),
        (SyntaxTypeRef::Named(name), TypeKind::Enum(actual)) => syntax
            .enums
            .iter()
            .any(|item| item.name == syntax.type_name(name) && item.id == *actual),
        (syntax, TypeKind::Builtin(actual)) => syntax.core_type() == Some(*actual),
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct ReceiverFacts {
    ty: TypeKind,
    constraints: Vec<StdlibCapabilityId>,
    recovered: bool,
}

fn infer_receiver(
    database: &mut CompilerDatabase,
    source: &str,
    tokens: &[&crate::lexer::Token],
    context: &MemberContext,
    compiler_context: crate::CompilerContext,
) -> Option<ReceiverFacts> {
    let receiver_offset = context.receiver_offset;
    // When completing `receiver.method`, analysis at the end of `receiver`
    // can resolve the surrounding call. Its recorded receiver type is the
    // type whose methods we need. For `receiver.method().`, however, the
    // completed call is itself the new receiver, so its result type is the
    // correct one. Non-path receivers have no textual receiver segments.
    let use_resolved_call_receiver = !context.receiver_path.is_empty();
    // Reuse the current revision before considering a repaired source. In LSP
    // use this database normally already contains the diagnostic pass, so a
    // second database used to repeat the complete semantic pipeline merely to
    // discover one receiver type.
    let direct = analyze_receiver_database(database, receiver_offset, use_resolved_call_receiver);
    if direct.as_ref().is_some_and(|facts| !facts.recovered) {
        return direct;
    }

    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..context.dot]);
    let suffix_start = completion_probe_suffix_end(tokens, context.replacement.end);
    probe_source.push_str(&source[suffix_start..]);
    analyze_receiver_source(probe_source, receiver_offset, compiler_context, false).or(direct)
}

fn analyze_receiver_source(
    source: String,
    receiver_offset: usize,
    compiler_context: crate::CompilerContext,
    use_resolved_call_receiver: bool,
) -> Option<ReceiverFacts> {
    let mut database = CompilerDatabase::with_context(compiler_context, source);
    analyze_receiver_database(&mut database, receiver_offset, use_resolved_call_receiver)
}

fn analyze_receiver_database(
    database: &mut CompilerDatabase,
    receiver_offset: usize,
    use_resolved_call_receiver: bool,
) -> Option<ReceiverFacts> {
    let analysis = database.analysis_at(receiver_offset).ok()??;
    let snapshot = database.semantic_snapshot().ok()?;
    let recovered = snapshot.checked().is_none();
    let resolved_call_receiver = match &analysis.resolution {
        Some(ExpressionResolution::Call(ResolvedCall::UserMethod { receiver_type, .. })) => {
            Some(*receiver_type)
        }
        Some(ExpressionResolution::Call(ResolvedCall::StandardLibrary {
            receiver_type: Some(receiver_type),
            ..
        })) => Some(*receiver_type),
        _ => None,
    };
    let receiver_type = if use_resolved_call_receiver {
        resolved_call_receiver?
    } else {
        analysis.ty
    };
    let constraints = snapshot
        .semantics()
        .generic_parameter_constraints(receiver_type)
        .to_vec();
    let ty = snapshot.semantics().types().kind(receiver_type).clone();
    Some(ReceiverFacts {
        ty,
        constraints,
        recovered,
    })
}

/// Removes an already-written call and propagation suffix from a completion
/// probe. When completion is manually requested on `receiver.method()`, the
/// identifier replacement alone would leave `receiver()` and infer the wrong
/// expression (or fail to type it entirely). The probe needs the type of the
/// expression before the member dot, regardless of whether the member text is
/// partial or already followed by its postfix syntax.
fn completion_probe_suffix_end(tokens: &[&crate::lexer::Token], identifier_end: usize) -> usize {
    let mut index = tokens.partition_point(|token| token.span.start < identifier_end);
    if index == tokens.len() {
        return identifier_end;
    }
    if !matches!(tokens[index].kind, TokenKind::LParen) {
        return identifier_end;
    }

    let mut depth = 0_u32;
    let mut end = identifier_end;
    while let Some(token) = tokens.get(index) {
        match token.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    end = token.span.end;
                    index += 1;
                    break;
                }
            }
            TokenKind::Eof => return identifier_end,
            _ => {}
        }
        index += 1;
    }
    if depth != 0 {
        return identifier_end;
    }
    if tokens
        .get(index)
        .is_some_and(|token| matches!(token.kind, TokenKind::Question))
    {
        end = tokens[index].span.end;
    }
    end
}

fn member_context(request: &CompletionRequest<'_>) -> Option<MemberContext> {
    let source = request.source;
    let offset = request.offset;
    let replacement = request.replacement;
    let dot_index = request
        .tokens
        .partition_point(|token| token.span.end <= replacement.start)
        .checked_sub(1)?;
    let dot_token = request.tokens[dot_index];
    if !matches!(dot_token.kind, TokenKind::Dot) {
        return None;
    }
    let receiver = request.tokens.get(dot_index.checked_sub(1)?)?;
    if matches!(receiver.kind, TokenKind::Eof) || receiver.span.start == receiver.span.end {
        return None;
    }

    let mut receiver_path = Vec::new();
    let mut index = dot_index;
    while let Some(identifier_index) = index.checked_sub(1) {
        let TokenKind::Ident(name) = &request.tokens[identifier_index].kind else {
            break;
        };
        receiver_path.push(name.clone());
        let Some(previous_dot) = identifier_index.checked_sub(1) else {
            break;
        };
        if !matches!(request.tokens[previous_dot].kind, TokenKind::Dot) {
            break;
        }
        index = previous_dot;
    }
    receiver_path.reverse();
    Some(MemberContext {
        receiver_path,
        receiver_offset: receiver.span.end.checked_sub(1)?,
        dot: dot_token.span.start,
        prefix: source[replacement.start..offset].to_owned(),
        replacement,
    })
}

fn identifier_span(source: &str, offset: usize) -> Span {
    let mut start = offset;
    while start > 0 && is_identifier_byte(source.as_bytes()[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < source.len() && is_identifier_byte(source.as_bytes()[end]) {
        end += 1;
    }
    Span { start, end }
}

fn is_identifier(name: &str) -> bool {
    splitscript_syntax::is_identifier(name)
}

fn is_identifier_byte(byte: u8) -> bool {
    splitscript_syntax::is_identifier_continue_byte(byte)
}

fn floor_char_boundary(source: &str, mut offset: usize) -> usize {
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn simple_completion(label: &str, kind: CompletionKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(detail.to_owned()),
        documentation: None,
        documentation_uri: None,
        insert_text: label.to_owned(),
        is_snippet: false,
    }
}

fn render_documentation<Id>(documentation: &Documentation<Id>) -> String {
    crate::documentation::strip_intra_doc_links(&crate::documentation::prose_markdown(
        documentation.summary,
        documentation.details,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(database: &mut CompilerDatabase, needle: &str) -> Vec<String> {
        let offset = database
            .source()
            .find(needle)
            .expect("completion marker exists")
            + needle.len();
        database
            .completions(offset)
            .expect("completion should succeed")
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn completes_only_missing_tick_rate_fields() {
        let mut empty = CompilerDatabase::new("state \"game.exe\" {}\ntickRate {\n    \n}");
        let completions = labels(&mut empty, "tickRate {");
        assert!(completions.contains(&"attached".to_owned()));
        assert!(completions.contains(&"detached".to_owned()));

        let mut partial = CompilerDatabase::new(
            "state \"game.exe\" {}\ntickRate {\n    attached: 60,\n    det\n}",
        );
        let completion = partial
            .completions(partial.source().find("det\n").unwrap() + 3)
            .expect("tick-rate completion should succeed");
        assert_eq!(completion.items.len(), 1, "{completion:#?}");
        assert_eq!(completion.items[0].label, "detached");
        assert_eq!(completion.items[0].insert_text, "detached: ${1:1},");
        assert!(completion.items[0].is_snippet);
    }

    #[test]
    fn completes_bounded_managed_string_policy_after_a_field_name() {
        for declaration in ["String scene ma", "String? subtitle from \"Caption\" ma"] {
            let source = format!(
                "image \"Assembly-CSharp\" {{\n    class Game {{\n        {declaration}\n    }}\n}}\nstate Unity [\"game.exe\"] {{}}"
            );
            let mut database = CompilerDatabase::new(source);
            let completion = database
                .completions(database.source().find("ma\n").unwrap() + 2)
                .expect("managed-field completion should recover incomplete syntax");
            assert_eq!(completion.items.len(), 1, "{completion:#?}");
            assert_eq!(completion.items[0].label, "maxLength");
            assert_eq!(completion.items[0].insert_text, "maxLength ${1:64};");
        }
    }

    #[test]
    fn completes_only_compatible_declared_setting_keys_inside_lookup_strings() {
        let source = r#"state "game.exe" {}
enum Mode { Fast, Slow }
settings {
    /// Splits at the boss.
    "Boss" => boss key "split-boss": true,
    "Mode" => mode key "run-mode": choice {
        "Fast" => Mode.Fast default,
        "Slow" => Mode.Slow,
    },
    for level in 2..=3 { `Level {level}` key `level-{level}`: true },
}
whileAttached {
    let boss = settings.enabled("split")
    let known = oldSettings.contains("")
}
"#;
        let mut enabled = CompilerDatabase::new(source);
        let enabled_offset =
            source.find("settings.enabled(\"split").unwrap() + "settings.enabled(\"split".len();
        let completion = enabled.completions(enabled_offset).unwrap();
        assert_eq!(completion.items.len(), 1, "{completion:#?}");
        assert_eq!(completion.items[0].label, "split-boss");
        assert_eq!(completion.items[0].insert_text, "split-boss");
        assert_eq!(
            &source[completion.replacement.start..completion.replacement.end],
            "split"
        );
        assert!(
            completion.items[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("settings.boss")
        );
        assert_eq!(
            completion.items[0].documentation.as_deref(),
            Some("Splits at the boss.")
        );

        let mut contains = CompilerDatabase::new(source);
        let contains_offset =
            source.find("oldSettings.contains(\"\"").unwrap() + "oldSettings.contains(\"".len();
        let labels = contains
            .completions(contains_offset)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        for expected in ["split-boss", "run-mode", "level-2", "level-3"] {
            assert!(
                labels.contains(&expected.to_owned()),
                "missing {expected}: {labels:#?}"
            );
        }
    }

    #[test]
    fn tick_rate_field_completion_is_not_offered_in_values_or_other_blocks() {
        let mut value =
            CompilerDatabase::new("state \"game.exe\" {}\ntickRate {\n    attached: det\n}");
        assert!(
            labels(&mut value, "attached: det")
                .iter()
                .all(|label| label != "detached")
        );

        let mut action =
            CompilerDatabase::new("state \"game.exe\" {}\nwhileAttached {\n    att\n}");
        assert!(
            labels(&mut action, "    att")
                .iter()
                .all(|label| label != "attached")
        );
    }

    #[test]
    fn settings_completion_offers_contextual_entry_and_kind_snippets() {
        let mut entries = CompilerDatabase::new("state \"game.exe\" {}\nsettings {\n    \n}");
        let labels = labels(&mut entries, "settings {");
        for expected in [
            "boolean setting",
            "settings group",
            "choice setting",
            "file setting",
            "for setting family",
        ] {
            assert!(
                labels.contains(&expected.to_owned()),
                "missing {expected}: {labels:#?}"
            );
        }

        let source = "state \"game.exe\" {}\nsettings {\n    \"Mode\" => mode: ch\n}";
        let mut kinds = CompilerDatabase::new(source);
        let completion = kinds
            .completions(source.find("ch\n").unwrap() + 2)
            .expect("setting-kind completion should succeed");
        assert_eq!(completion.items.len(), 1);
        assert_eq!(completion.items[0].label, "choice");
        assert!(completion.items[0].is_snippet);
        assert!(completion.items[0].insert_text.starts_with("choice {"));
    }

    #[test]
    fn settings_completion_understands_groups_choices_and_file_filters() {
        let mut group = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings {\n    \"General\" {\n        \n    },\n}",
        );
        assert!(labels(&mut group, "\"General\" {").contains(&"boolean setting".to_owned()));

        let mut choice = CompilerDatabase::new(
            "enum Mode { A }\nstate \"game.exe\" {}\nsettings {\n    \"Mode\" => mode: choice {\n        \n    },\n}",
        );
        assert_eq!(labels(&mut choice, "choice {"), vec!["choice option"]);

        let mut file = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings {\n    \"Input\" => input: file {\n        \n    },\n}",
        );
        let labels = labels(&mut file, "file {");
        assert!(labels.contains(&"named filter".to_owned()));
        assert!(labels.contains(&"fallback filter".to_owned()));
        assert!(labels.contains(&"MIME filter".to_owned()));
    }

    #[test]
    fn state_completion_offers_fields_layouts_types_and_sources_contextually() {
        let mut empty = CompilerDatabase::new("state \"game.exe\" {\n    \n}");
        let candidates = labels(&mut empty, "state \"game.exe\" {");
        for expected in [
            "expression field",
            "memory field",
            "inferred memory field",
            "module pointer field",
            "UTF-8 string field",
            "UTF-16LE string field",
            "layout dimensions",
            "named layout",
        ] {
            assert!(
                candidates.contains(&expected.to_owned()),
                "missing {expected}: {candidates:#?}"
            );
        }

        let source = "struct Position { x: f32, }\nstate \"game.exe\" {\n    position: Pos\n}";
        let mut typed = CompilerDatabase::new(source);
        let candidates = labels(&mut typed, "position: Pos");
        assert!(candidates.contains(&"Position".to_owned()));
        assert!(!candidates.contains(&"print".to_owned()));

        let source = "state \"game.exe\" {\n    position: i32 a\n}";
        let mut source_kind = CompilerDatabase::new(source);
        let candidates = labels(&mut source_kind, "i32 a");
        assert_eq!(candidates, vec!["at"]);
    }

    #[test]
    fn layout_refinement_completes_conditional_state_and_managed_fields() {
        let source = r#"
enum Edition { Base, Demo }
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        if layout.edition == Edition.Base { u32 level; }
    }
}
state Unity ["game.exe"] {
    layout { edition: Edition }
    if layout.edition == Edition.Base { scene: u8 at 0x100; }
}
onAttach { return Layout { edition: Edition.Base } }
split {
    let manager = GameManager.instance else return false
    if layout.edition == Edition.Base {
        current.
        manager.
    }
    return false
}
"#;
        let mut state = CompilerDatabase::new(source);
        assert!(labels(&mut state, "current.").contains(&"scene".to_owned()));
        let managed_source = source.replace("        current.\n", "        current.scene\n");
        let mut managed = CompilerDatabase::new(&managed_source);
        let managed = labels(&mut managed, "manager.");
        assert!(managed.contains(&"level".to_owned()), "{managed:#?}");
    }

    #[test]
    fn every_type_grammar_position_uses_the_shared_type_catalog() {
        let declarations = r#"
struct Position {
    x: i32,
}
enum Mode {
    Fast,
}
"#;
        let cases = [
            (
                format!("{declarations}\nfn inspect(value: ) {{}}\nstate \"game.exe\" {{}}"),
                "value: ",
            ),
            (
                format!("{declarations}\nfn inspect() ->  {{}}\nstate \"game.exe\" {{}}"),
                "-> ",
            ),
            (
                format!("{declarations}\nlet globalValue:  = None\nstate \"game.exe\" {{}}"),
                "globalValue: ",
            ),
            (
                format!(
                    "{declarations}\nfn inspect() {{ let localValue:  = None }}\nstate \"game.exe\" {{}}"
                ),
                "localValue: ",
            ),
            (
                format!(
                    "{declarations}\nfn inspect(value) {{ let cast = value as  }}\nstate \"game.exe\" {{}}"
                ),
                "value as ",
            ),
            (
                format!("{declarations}\nstate GBA {{ watched:  at 0x100; }}"),
                "watched: ",
            ),
            (
                format!("{declarations}\nstruct Holder {{ field: , }}\nstate \"game.exe\" {{}}"),
                "field: ",
            ),
            (
                format!("{declarations}\nenum Wrapped {{ Value(), }}\nstate \"game.exe\" {{}}"),
                "Value(",
            ),
            (
                format!("{declarations}\nlet genericValue: Set< = None\nstate \"game.exe\" {{}}"),
                "Set<",
            ),
            (
                format!("{declarations}\nlet arrayValue: [ = None\nstate \"game.exe\" {{}}"),
                "arrayValue: [",
            ),
        ];

        for (source, marker) in cases {
            let mut database = CompilerDatabase::new(source);
            let offset = database.source().find(marker).unwrap() + marker.len();
            let completions = database.completions(offset).unwrap();
            let labels = completions
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>();
            for expected in [
                "i32", "String", "Position", "Mode", "Set", "[T]", "[T; N]", "T?", "T!", "async T",
            ] {
                assert!(
                    labels.contains(&expected),
                    "missing `{expected}` at `{marker}`: {labels:#?}"
                );
            }
            for value_candidate in ["print", "process", "current", "return"] {
                assert!(
                    !labels.contains(&value_candidate),
                    "value candidate `{value_candidate}` leaked into `{marker}`: {labels:#?}"
                );
            }
            assert!(completions.items.iter().all(|item| matches!(
                item.kind,
                CompletionKind::Type | CompletionKind::Struct | CompletionKind::Enum
            )));
            let set = completions
                .items
                .iter()
                .find(|item| item.label == "Set")
                .unwrap();
            assert_eq!(set.insert_text, "Set<${1:T}>");
            assert!(set.is_snippet);
            assert!(set.documentation.is_some());
        }
    }

    #[test]
    fn private_standard_library_types_are_not_completed() {
        let mut database = CompilerDatabase::new("let value:  = None\nstate \"game.exe\" {}");
        let candidates = labels(&mut database, "value: ");
        for private_type in [
            "MonoLayout",
            "MonoModule",
            "MonoImage",
            "MonoClass",
            "UnityModule",
            "UnityImage",
            "UnityClass",
            "UnityField",
        ] {
            assert!(!candidates.contains(&private_type.to_owned()));
        }
    }

    #[test]
    fn value_colons_are_not_mistaken_for_type_annotations() {
        let source = r#"
struct Position {
    x: i32,
}
state "game.exe" {}
fn inspect() {
    let position = Position { x: pri }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "x: pri");
        assert!(
            completions.contains(&"print".to_owned()),
            "{completions:#?}"
        );
    }

    #[test]
    fn state_completion_respects_layout_and_specialized_provider_grammars() {
        let mut gba = CompilerDatabase::new("state GBA {\n    \n}");
        let candidates = labels(&mut gba, "state GBA {");
        assert!(candidates.contains(&"memory field".to_owned()));
        assert!(!candidates.contains(&"module pointer field".to_owned()));
        assert!(!candidates.contains(&"UTF-8 string field".to_owned()));

        let source = "state \"game.exe\" {\n    layout Steam {\n        \n    },\n    /// outer marker\n    \n}";
        let mut layouts = CompilerDatabase::new(source);
        let inside = labels(&mut layouts, "layout Steam {");
        assert!(inside.contains(&"memory field".to_owned()));
        assert!(!inside.contains(&"named layout".to_owned()));
        assert_eq!(
            labels(&mut layouts, "/// outer marker\n    "),
            vec!["named layout"]
        );

        let source =
            "enum Edition { BaseGame }\nstate \"game.exe\" {\n    layout {\n        \n    }\n}";
        let mut dimensions = CompilerDatabase::new(source);
        assert_eq!(
            labels(&mut dimensions, "layout {"),
            vec!["layout dimension"]
        );
        let candidates = labels(&mut dimensions, "layout {\n        ");
        assert_eq!(candidates, vec!["layout dimension"]);
    }

    #[test]
    fn state_completion_finishes_pointer_paths_without_hiding_expressions() {
        let source = "state \"game.exe\" {\n    value: i32 at 0x1000 a\n}";
        let mut pointer = CompilerDatabase::new(source);
        assert_eq!(labels(&mut pointer, "0x1000 a"), vec!["as"]);

        let source = "state GBA {\n    value: i32 at 0x1000 \n}";
        let mut gba = CompilerDatabase::new(source);
        assert_eq!(labels(&mut gba, "0x1000 "), vec!["if"]);

        let source = "state \"game.exe\" { value = process.rea }";
        let mut expression = CompilerDatabase::new(source);
        assert!(labels(&mut expression, "process.rea").contains(&"read".to_owned()));
    }

    #[test]
    fn top_level_completion_contains_only_declarations_and_lifecycle_blocks() {
        let source = "state \"game.exe\" {}\n\n";
        let mut database = CompilerDatabase::new(source);
        let candidates = labels(&mut database, source);
        for expected in [
            "fn",
            "struct",
            "enum",
            "let",
            "settings",
            "tickRate",
            "setup",
            "selectProcess",
            "onAttach",
            "whileAttached",
            "split",
        ] {
            assert!(
                candidates.contains(&expected.to_owned()),
                "missing {expected}: {candidates:#?}"
            );
        }
        for unavailable in [
            "state", "print", "process", "timer", "i32", "if", "return", "current",
        ] {
            assert!(
                !candidates.contains(&unavailable.to_owned()),
                "unexpected {unavailable}: {candidates:#?}"
            );
        }

        let source = "state \"game.exe\" {}\nsettings {}\ntickRate {}\nsplit {}\n\n";
        let mut declared = CompilerDatabase::new(source);
        let candidates = labels(&mut declared, source);
        for duplicate in ["state", "settings", "tickRate", "split"] {
            assert!(!candidates.contains(&duplicate.to_owned()));
        }
    }

    #[test]
    fn state_header_completion_guides_processes_lists_and_catalog_providers() {
        let mut target = CompilerDatabase::new("state ");
        let candidates = labels(&mut target, "state ");
        assert!(candidates.contains(&"\"game.exe\"".to_owned()));
        assert!(candidates.contains(&"[\"game.exe\", ...]".to_owned()));
        assert!(candidates.contains(&"GBA".to_owned()));
        assert!(!candidates.contains(&"Process".to_owned()));

        let mut provider = CompilerDatabase::new("state G");
        assert_eq!(
            labels(&mut provider, "state G"),
            vec!["GBA", "GCN", "Genesis"]
        );

        let mut provider_body = CompilerDatabase::new("state GBA ");
        assert_eq!(labels(&mut provider_body, "state GBA "), vec!["{"]);

        let mut process_body = CompilerDatabase::new("state [\"game.exe\", \"demo.exe\"] ");
        assert_eq!(
            labels(&mut process_body, "state [\"game.exe\", \"demo.exe\"] "),
            vec!["{"]
        );
    }

    #[test]
    fn completes_domains_catalogs_and_inferred_members() {
        let source = r#"
struct Position {
    x: f32
}

enum Mode {
    Idle,
    Active(i32)
}

fn Position.coordinate() {
    return self.x
}

fn moduleAddress(module: Module) {
    module.ad
}

state "game.exe" {
    position: Position = process.read(0)
}

settings {
    "General" {
        "Enabled" => enabled: true
    }
}

whileAttached {
    let number: i32 = 4
    let snapshot = current
    let capturedSettings = settings
    current.po
    snapshot.po
    capturedSettings.en
    settings.en
    Mode.Ac
    process.re
    process.na
    process.main
    process.cl
    number.cl
    current.position.co
    current.position.x
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "current.po").contains(&"position".to_owned()));
        assert!(labels(&mut database, "snapshot.po").contains(&"position".to_owned()));
        assert!(labels(&mut database, "capturedSettings.en").contains(&"enabled".to_owned()));
        assert!(labels(&mut database, "settings.en").contains(&"enabled".to_owned()));
        assert!(labels(&mut database, "Mode.Ac").contains(&"Active".to_owned()));
        assert!(labels(&mut database, "process.re").contains(&"read".to_owned()));
        assert!(labels(&mut database, "process.na").contains(&"name".to_owned()));
        assert!(labels(&mut database, "process.main").contains(&"mainModule".to_owned()));
        assert!(labels(&mut database, "process.cl").contains(&"closed".to_owned()));
        assert!(labels(&mut database, "number.cl").contains(&"clamp".to_owned()));
        assert!(labels(&mut database, "module.ad").contains(&"address".to_owned()));
        assert!(labels(&mut database, "current.position.co").contains(&"coordinate".to_owned()));
        assert!(labels(&mut database, "current.position.x").contains(&"x".to_owned()));

        let mut bare = CompilerDatabase::new(
            "state \"game.exe\" {}\nwhileAttached { let number: i32 = 4\nnumber.\n}",
        );
        assert!(labels(&mut bare, "number.").contains(&"min".to_owned()));

        let mut float_type =
            CompilerDatabase::new("state \"game.exe\" {}\nsetup { let value = f32. }");
        let completions = float_type
            .completions(float_type.source().find("f32.").unwrap() + "f32.".len())
            .expect("float associated items should complete");
        for name in ["NaN", "positiveInfinity", "negativeInfinity"] {
            let item = completions
                .items
                .iter()
                .find(|item| item.label == name)
                .unwrap_or_else(|| panic!("missing `{name}`: {completions:#?}"));
            assert_eq!(item.kind, CompletionKind::Constant);
            assert_eq!(item.insert_text, name);
            assert!(!item.is_snippet);
        }

        let mut keyed_settings = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings { \"Flag\" => flag key \"flag\": true }\nwhileAttached { settings.en }",
        );
        assert!(labels(&mut keyed_settings, "settings.en").contains(&"enabled".to_owned()));

        let mut generated_settings = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings { for level in 2..=4 { `{level}`: true } }\nwhileAttached { settings._setting }",
        );
        assert!(
            labels(&mut generated_settings, "settings._setting").is_empty(),
            "compile-time family implementation names must not become members"
        );
    }

    #[test]
    fn completes_methods_after_fields_on_expression_receivers() {
        let source = r#"
struct Path {
    address: address
}

fn Path.resolve() {
    return self.address
}

struct Layout {
    isLoading: Path
}

fn selectedLayout() {
    return Layout {
        isLoading: Path { address: 0x1000 }
    }
}

state "game.exe" {
    loading: bool = selectedLayout().isLoading.resolve()
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "selectedLayout().isLoading.resolve")
                .contains(&"resolve".to_owned())
        );
    }

    #[test]
    fn completes_transactional_snapshots_on_live_managed_references() {
        let source = r#"
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        i32 points;
    }
}
state Unity ["game.exe"] {}
fn capture(manager: GameManager.Ref) {
    manager.snap
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("manager.snap").unwrap() + "manager.snap".len();
        let completions = database.completions(offset).unwrap();
        let snapshot = completions
            .items
            .iter()
            .find(|item| item.label == "snapshot")
            .expect("live managed references should complete `snapshot`");
        assert_eq!(snapshot.kind, CompletionKind::Method);
        assert_eq!(
            snapshot.detail.as_deref(),
            Some("GameManager.Ref.snapshot() -> GameManager!")
        );
        assert_eq!(snapshot.insert_text, "snapshot()");
        assert!(
            snapshot
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("transactionally"))
        );
    }

    #[test]
    fn completes_cooperative_instance_discovery_on_managed_classes() {
        let source = r#"
image "Assembly-CSharp" {
    class Enemy {
        i32 health;
    }
}
state Unity ["game.exe"] {}
onAttach {
    let enemies = await Enemy.inst
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("Enemy.inst").unwrap() + "Enemy.inst".len();
        let completions = database.completions(offset).unwrap();
        let instances = completions
            .items
            .iter()
            .find(|item| item.label == "instances")
            .expect("managed classes should complete `instances`");
        assert_eq!(instances.kind, CompletionKind::Method);
        assert_eq!(
            instances.detail.as_deref(),
            Some("Enemy.instances() -> async [Enemy.Ref]")
        );
        assert_eq!(instances.insert_text, "instances()");
        assert!(
            instances
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("Cooperatively"))
        );
    }

    #[test]
    fn completes_typed_components_on_unity_game_objects() {
        let source = r#"
image "Assembly-CSharp" {
    class PlayerController {
        i32 health;
    }
}
state Unity ["game.exe"] {}
whileAttached {
    let scene = unity.scenes.active() else return
    let player = scene.find("World/Player") else return
    player.comp
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("player.comp").unwrap() + "player.comp".len();
        let completions = database.completions(offset).unwrap();
        let component = completions
            .items
            .iter()
            .find(|item| item.label == "component")
            .expect("Unity GameObjects should complete `component<T>()`");
        assert_eq!(component.kind, CompletionKind::Method);
        assert_eq!(
            component.detail.as_deref(),
            Some("UnityGameObject.component<T>() -> T.Ref!")
        );
        assert_eq!(component.insert_text, "component<${1:Class}>()");
    }

    #[test]
    fn completes_catalog_methods_on_existing_expression_receiver_calls() {
        let source = r#"
struct Layout {
    isLoading: MemoryPath
}

fn selectedLayout(layout: Layout) {
    return layout
}

fn inspect(layout: Layout) {
    selectedLayout(layout).isLoading.resolve()
}

state "game.exe" {}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "selectedLayout(layout).isLoading.resolve")
                .contains(&"resolve".to_owned())
        );
    }

    #[test]
    fn completes_array_members_and_indexed_elements_on_snapshot_fields() {
        let source = include_str!("../examples/minish_cap.split");
        let element_source = source.replacen("old.inventory[5]", "old.inventory[5].", 1);
        let mut database = CompilerDatabase::new(element_source);
        let element = labels(&mut database, "old.inventory[5].");
        assert!(element.contains(&"min".to_owned()), "{element:#?}");
        assert!(element.contains(&"max".to_owned()), "{element:#?}");
        assert!(element.contains(&"clamp".to_owned()), "{element:#?}");
        for array_method in ["set", "length", "isEmpty", "contains", "indexOf"] {
            assert!(
                !element.contains(&array_method.to_owned()),
                "array method `{array_method}` leaked onto u8 completion: {element:#?}"
            );
        }

        let incomplete = r#"
state "game.exe" {
    inventory: [u8; 6] at 0x1000
}

split {
    old.inventory.
}
"#;
        let mut database = CompilerDatabase::new(incomplete);
        let completions = labels(&mut database, "old.inventory.");
        assert!(!completions.contains(&"get".to_owned()));
        assert!(completions.contains(&"set".to_owned()));
        assert!(completions.contains(&"length".to_owned()));
        assert!(completions.contains(&"isEmpty".to_owned()));
        assert!(completions.contains(&"contains".to_owned()));
        assert!(completions.contains(&"indexOf".to_owned()));
        assert!(!completions.contains(&"push".to_owned()));
        assert!(!completions.contains(&"extend".to_owned()));
        assert!(!completions.contains(&"removeAt".to_owned()));
        assert!(!completions.contains(&"remove".to_owned()));
        assert!(!completions.contains(&"pop".to_owned()));
        assert!(!completions.contains(&"clear".to_owned()));

        let growable = r#"
state "game.exe" {}

split {
    let values: [u8] = []
    values.
}
"#;
        let mut database = CompilerDatabase::new(growable);
        let completions = labels(&mut database, "values.");
        assert!(completions.contains(&"push".to_owned()));
        assert!(completions.contains(&"extend".to_owned()));
        assert!(completions.contains(&"removeAt".to_owned()));
        assert!(completions.contains(&"remove".to_owned()));
        assert!(completions.contains(&"pop".to_owned()));
        assert!(completions.contains(&"clear".to_owned()));
    }

    #[test]
    fn completes_string_members_in_match_arms_and_on_inferred_locals() {
        let source = include_str!("../examples/minish_cap.split");

        let literal_source = source.replacen("2 => \"½\"", "2 => \"½\".", 1);
        let mut database = CompilerDatabase::new(literal_source);
        let literal = labels(&mut database, "2 => \"½\".");
        for member in [
            "byteLength",
            "isEmpty",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "toAsciiLowerCase",
            "toAsciiUpperCase",
            "trimAsciiWhitespace",
            "replaceAll",
            "split",
            "parse",
            "slice",
        ] {
            assert!(literal.contains(&member.to_owned()), "{literal:#?}");
        }

        let local_source = source.replacen("return fraction", "return fraction.", 1);
        let mut database = CompilerDatabase::new(local_source);
        let local = labels(&mut database, "return fraction.");
        for member in [
            "byteLength",
            "isEmpty",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "toAsciiLowerCase",
            "toAsciiUpperCase",
            "trimAsciiWhitespace",
            "replaceAll",
            "split",
            "parse",
            "slice",
        ] {
            assert!(local.contains(&member.to_owned()), "{local:#?}");
        }
    }

    #[test]
    fn member_completion_uses_token_boundaries_across_trivia_and_eof() {
        for expression in ["number.", "number.   ", "number. /* gap */ cl"] {
            let source = format!(
                "state \"game.exe\" {{}}\nsetup {{\n    let number: i32 = 4\n    {expression}\n}}"
            );
            let offset = source.find(expression).unwrap() + expression.len();
            let mut database = CompilerDatabase::new(source);
            let completion = database
                .completions(offset)
                .expect("member completion should survive trivia and token boundaries");
            assert!(
                completion.items.iter().any(|item| item.label == "clamp"),
                "{expression}: {completion:#?}"
            );
        }
    }

    #[test]
    fn completes_only_the_refined_layout_fields_in_layout_match_arms() {
        let source = r#"
state "game.exe" {
    layout V8 { loading: i32 at 0x100; bike: i16 at 0x104; },
    layout V9 { loading: i32 at 0x200; bike: u16 at 0x204; video: u8 at 0x206; },
}
onAttach { return StateLayout.V8 }
split {
    return match layout {
        StateLayout.V8 => current.,
        StateLayout.V9 => old.,
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let v8 = labels(&mut database, "StateLayout.V8 => current.");
        assert!(v8.contains(&"loading".to_owned()));
        assert!(v8.contains(&"bike".to_owned()), "{v8:#?}");
        assert!(!v8.contains(&"video".to_owned()));

        let mut database = CompilerDatabase::new(source);
        let v9 = labels(&mut database, "StateLayout.V9 => old.");
        assert!(v9.contains(&"loading".to_owned()));
        assert!(v9.contains(&"bike".to_owned()));
        assert!(v9.contains(&"video".to_owned()));

        let outside = format!("{source}\nwhileAttached {{ current. }}");
        let mut database = CompilerDatabase::new(outside);
        let outside = labels(&mut database, "whileAttached { current.");
        assert!(outside.contains(&"loading".to_owned()));
        assert!(!outside.contains(&"bike".to_owned()), "{outside:#?}");
        assert!(!outside.contains(&"video".to_owned()));
    }

    #[test]
    fn generic_type_arguments_complete_on_captured_process_values() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let attached = process
    attached.read<i
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "attached.read<i");
        assert!(completions.contains(&"i32".to_owned()));
        assert!(completions.contains(&"i64".to_owned()));
    }

    #[test]
    fn constrained_generic_parameters_complete_their_available_methods() {
        let source = r#"
state "game.exe" {}
fn smaller(value, other) {
    let result = value.min(other)
    value.
    return result
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "\n    value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(!completions.contains(&"toString".to_owned()));
        assert!(!completions.contains(&"length".to_owned()));
    }

    #[test]
    fn inherited_capabilities_complete_super_capability_methods() {
        let source = r#"
state "game.exe" {}
fn masked(value) {
    let result = value & 1
    value.
    return result
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "\n    value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(completions.contains(&"toString".to_owned()));
    }

    #[test]
    fn gba_states_expose_only_the_typed_provider_value() {
        let source = "state GBA { room: u8 = gba.re }";
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "gba.re").contains(&"read".to_owned()));

        let root_source = "state GBA {}\nwhileAttached { gb }";
        let mut database = CompilerDatabase::new(root_source);
        let provider_items = labels(&mut database, " gb");
        assert!(provider_items.contains(&"gba".to_owned()));

        let process_source = "state GBA {}\nwhileAttached { pro }";
        let mut database = CompilerDatabase::new(process_source);
        assert!(!labels(&mut database, " pro").contains(&"process".to_owned()));

        let native_source = "state \"game.exe\" {}\nwhileAttached { pro }";
        let mut database = CompilerDatabase::new(native_source);
        assert!(labels(&mut database, " pro").contains(&"process".to_owned()));
    }

    #[test]
    fn provider_values_complete_only_with_an_attached_process() {
        for (state, prefix, provider) in [
            ("state \"game.exe\" {}", "pro", "process"),
            ("state GBA {}", "gb", "gba"),
        ] {
            for action in ["setup", "onDetach", "onStart", "onReset"] {
                let detached = format!("{state}\n{action} {{ {prefix} }}");
                let mut database = CompilerDatabase::new(detached);
                assert!(
                    !labels(&mut database, &format!("{{ {prefix}")).contains(&provider.to_owned()),
                    "{provider} must not complete in {action}"
                );
            }

            for action in ["onAttach", "whileAttached", "split"] {
                let attached = format!("{state}\n{action} {{ {prefix} }}");
                let mut database = CompilerDatabase::new(attached);
                assert!(
                    labels(&mut database, &format!("{{ {prefix}")).contains(&provider.to_owned()),
                    "{provider} should complete in {action}"
                );
            }
        }
    }

    #[test]
    fn process_selection_completes_the_native_candidate_before_provider_setup() {
        let source = "state Unity [\"game.exe\"] {}\nselectProcess { pro }";
        let mut database = CompilerDatabase::new(source);
        let roots = labels(&mut database, "{ pro");
        assert!(roots.contains(&"process".to_owned()));
        assert!(!roots.contains(&"unity".to_owned()));

        let source = "state Unity [\"game.exe\"] {}\nselectProcess { process.pa }";
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "process.pa").contains(&"path".to_owned()));
    }

    #[test]
    fn detached_completion_filters_transitive_attached_process_functions() {
        let declarations = r#"
state "game.exe" {}

fn readsProcess() {
    let value: i32 = process.read<i32>(0x100) else return
}

fn relay() {
    readsProcess()
}

fn safe() {
    print("safe")
}
"#;

        let detached_relay = format!("{declarations}\nonDetach {{ rel }}");
        let mut database = CompilerDatabase::new(detached_relay);
        assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let detached_direct = format!("{declarations}\nonDetach {{ rea }}");
        let mut database = CompilerDatabase::new(detached_direct);
        assert!(!labels(&mut database, "{ rea").contains(&"readsProcess".to_owned()));

        let detached_safe = format!("{declarations}\nonDetach {{ sa }}");
        let mut database = CompilerDatabase::new(detached_safe);
        assert!(labels(&mut database, "{ sa").contains(&"safe".to_owned()));

        let setup_relay = format!("{declarations}\nsetup {{ rel }}");
        let mut database = CompilerDatabase::new(setup_relay);
        assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let attached = format!("{declarations}\nonAttach {{ rel }}");
        let mut database = CompilerDatabase::new(attached);
        assert!(labels(&mut database, "{ rel").contains(&"relay".to_owned()));
    }

    #[test]
    fn completion_scopes_snapshot_roots_and_snapshot_dependent_functions() {
        let declarations = r#"
state "game.exe" { level: u32 at 0x100 }

fn changed() {
    return old.level != current.level
}

fn relay() {
    return changed()
}
"#;
        for action in ["setup", "onAttach", "onDetach", "onStart", "onReset"] {
            let source = format!("{declarations}\n{action} {{ cur }}");
            let mut database = CompilerDatabase::new(source);
            assert!(!labels(&mut database, "{ cur").contains(&"current".to_owned()));

            let source = format!("{declarations}\n{action} {{ rel }}");
            let mut database = CompilerDatabase::new(source);
            assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));
        }

        let source = format!("{declarations}\nsplit {{ cur }}");
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "{ cur").contains(&"current".to_owned()));

        let source = format!("{declarations}\nsplit {{ rel }}");
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let function = format!("{declarations}\nfn another() {{ cur }}");
        let mut database = CompilerDatabase::new(function);
        assert!(labels(&mut database, "{ cur").contains(&"current".to_owned()));
    }

    #[test]
    fn root_completion_comes_from_language_standard_library_and_source() {
        let source = "state \"game.exe\" {}\nlet customValue = 0\nwhileAttached { pri }";
        let mut database = CompilerDatabase::new(source);
        let labels = labels(&mut database, "pri");
        assert!(labels.contains(&"print".to_owned()));

        let offset = source.find("customValue").unwrap() + 2;
        let all = database.completions(offset).unwrap();
        assert!(all.items.iter().any(|item| item.label == "customValue"));

        let empty_prefix = source.find(" pri").unwrap() + 1;
        let all = database.completions(empty_prefix).unwrap();
        assert!(all.items.iter().any(|item| item.label == "whileAttached"));
        assert!(all.items.iter().all(|item| item.label != "utf8"));
    }

    #[test]
    fn provider_context_completion_follows_attachment_availability_and_type() {
        let mut database = CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { uni }");
        let attached = labels(&mut database, "uni }");
        assert!(attached.contains(&"unity".to_owned()), "{attached:#?}");

        let mut database =
            CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { unity. }");
        let context = labels(&mut database, "unity.");
        assert!(context.contains(&"scenes".to_owned()), "{context:#?}");

        let mut database =
            CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { unity.scenes. }");
        let scenes = labels(&mut database, "unity.scenes.");
        for method in ["active", "loaded", "persistent"] {
            assert!(
                scenes.contains(&method.to_owned()),
                "missing `{method}`: {scenes:#?}"
            );
        }

        let source = "state Unity [\"game.exe\"] {}\nonDetach { uni }";
        let mut database = CompilerDatabase::new(source);
        let detached_offset = source.rfind("uni }").unwrap() + "uni".len();
        let detached = database.completions(detached_offset).unwrap();
        assert!(detached.items.iter().all(|item| item.label != "unity"));
    }

    #[test]
    fn completes_value_loops_with_language_documentation() {
        let source = "state \"game.exe\" {}\nfn choose() { let value = loo }";
        let mut database = CompilerDatabase::new(source);
        let completions = database
            .completions(source.find("loo }").unwrap() + 3)
            .unwrap();
        let item = completions
            .items
            .iter()
            .find(|item| item.label == "loop")
            .expect("loop should complete in expression position");

        assert_eq!(item.kind, CompletionKind::Snippet);
        assert_eq!(item.insert_text, "loop {\n    $0\n}");
        assert!(item.is_snippet);
        assert!(item.documentation.as_deref().unwrap().contains("break"));
        assert_eq!(item.documentation_uri.as_deref(), Some("/language/loop.md"));
    }

    #[test]
    fn named_layouts_complete_the_generated_type_value_and_variants() {
        let source = r#"
state "game.exe" {
    layout Steam { level: u32 at 0x100 },
    layout GOG { level: u32 at 0x200 }
}
onAttach { return StateLayout. }
split { lay }
"#;
        let mut database = CompilerDatabase::new(source);
        let variants = labels(&mut database, "StateLayout.");
        assert!(variants.contains(&"Steam".to_owned()));
        assert!(variants.contains(&"GOG".to_owned()));
        assert!(labels(&mut database, "lay }").contains(&"layout".to_owned()));

        let missing_selector = r#"state "game.exe" {
    layout Steam { level: u32 at 0x100 },
    layout GOG { level: u32 at 0x200 },
}
onA"#;
        let mut database = CompilerDatabase::new(missing_selector);
        let completions = database.completions(missing_selector.len()).unwrap();
        let selector = completions
            .items
            .iter()
            .find(|item| item.label == "onAttach")
            .expect("named layouts should offer a safe selector snippet");
        assert!(selector.is_snippet);
        assert!(selector.insert_text.contains("return StateLayout.Steam"));
        assert!(selector.insert_text.contains("return StateLayout.GOG"));
        assert!(selector.insert_text.ends_with("await process.closed()\n}"));
    }

    #[test]
    fn state_decoder_completion_is_contextual_and_inserts_its_bound() {
        let source = "state \"game.exe\" { mapName at 0x100 as ut }";
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("ut }").unwrap() + 2;
        let completions = database.completions(offset).unwrap();
        let decoder = completions
            .items
            .iter()
            .find(|item| item.label == "utf8")
            .expect("the pointer-state decoder should complete after `as`");
        assert_eq!(decoder.kind, CompletionKind::Function);
        assert_eq!(decoder.insert_text, "utf8(${1:maxBytes})");
        assert!(decoder.is_snippet);
        let wide = completions
            .items
            .iter()
            .find(|item| item.label == "utf16le")
            .expect("the UTF-16LE decoder should share the state-decoder completion");
        assert_eq!(wide.insert_text, "utf16le(${1:maxUtf16Units})");
        assert!(wide.is_snippet);

        let cast_source = "state \"game.exe\" {} whileAttached { let value = 1 as ut }";
        let mut database = CompilerDatabase::new(cast_source);
        let offset = cast_source.find("ut }").unwrap() + 2;
        assert!(
            database
                .completions(offset)
                .unwrap()
                .items
                .iter()
                .all(|item| item.label != "utf8" && item.label != "utf16le")
        );
    }

    #[test]
    fn root_completion_includes_preceding_lexical_bindings() {
        let source = r#"
state "game.exe" {}

fn inspect(parameter: i32) {
    let localValue = parameter
    loc
}

onAttach {
    let image = await unity.il2cpp()
    let gameManagerClass = await image.class("GameManager")
    gam
    let gameManagerInstance = 0
}
"#;
        let mut database = CompilerDatabase::new(source);
        let action_labels = labels(&mut database, "\n    gam");
        assert!(action_labels.contains(&"gameManagerClass".to_owned()));
        assert!(!action_labels.contains(&"gameManagerInstance".to_owned()));

        let function_labels = labels(&mut database, "\n    loc");
        assert!(function_labels.contains(&"localValue".to_owned()));

        let parameter_offset = source.find("localValue = par").unwrap() + "localValue = par".len();
        let parameter_completion = database.completions(parameter_offset).unwrap();
        assert!(
            parameter_completion.items.iter().any(
                |item| item.label == "parameter" && item.detail.as_deref() == Some("parameter")
            )
        );
    }

    #[test]
    fn managed_classes_expose_only_global_layout_refined_fields() {
        let schema = r#"
enum Edition { BaseGame, Demo }
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        u32 common;
        if layout.edition == Edition.BaseGame { u32 level; }
        else { u32 scene; }
    }
}
state Unity ["game.exe"] { layout { edition: Edition } }
onAttach { return Layout { edition: Edition.BaseGame } }
"#;

        let source = format!("{schema}\nwhileAttached {{ GameManager. }}");
        let mut database = CompilerDatabase::new(source);
        let class = labels(&mut database, "GameManager.");
        assert!(class.contains(&"instance".to_owned()), "{class:#?}");
        assert!(!class.contains(&"layout".to_owned()), "{class:#?}");
        assert!(!class.contains(&"Layout".to_owned()), "{class:#?}");

        let source = format!(
            "{schema}\nwhileAttached {{\n    let manager = GameManager.instance else return\n    if layout.edition == Edition.BaseGame {{\n        manager.\n    }}\n}}"
        );
        let mut database = CompilerDatabase::new(source);
        let fields = labels(&mut database, "        manager.");
        assert!(fields.contains(&"common".to_owned()), "{fields:#?}");
        assert!(fields.contains(&"level".to_owned()), "{fields:#?}");
        assert!(!fields.contains(&"scene".to_owned()), "{fields:#?}");

        let source = format!(
            "{schema}\nwhileAttached {{\n    let manager = GameManager.instance else return\n    if layout.edition == Edition.BaseGame {{\n        print(\"base\")\n    }} else {{\n        manager.\n    }}\n}}"
        );
        let mut database = CompilerDatabase::new(source);
        let fields = labels(&mut database, "        manager.");
        assert!(fields.contains(&"common".to_owned()), "{fields:#?}");
        assert!(!fields.contains(&"level".to_owned()), "{fields:#?}");
        assert!(fields.contains(&"scene".to_owned()), "{fields:#?}");
    }

    #[test]
    fn for_bindings_complete_only_inside_the_loop_body() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let values = [1, 2]
    for element in values {
        ele
    }
    ele
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "        ele").contains(&"element".to_owned()));
        assert!(!labels(&mut database, "    ele\n}").contains(&"element".to_owned()));
    }

    #[test]
    fn state_filter_bindings_complete_with_the_field_type() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 if value.min(7) == 7 { Err("transient") } else { value }
}"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(!completions.contains(&"length".to_owned()));
    }
}
