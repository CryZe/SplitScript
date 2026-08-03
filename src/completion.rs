//! Compiler-owned completion candidates shared by editor frontends.

use std::collections::BTreeMap;

use crate::{
    ast::{
        Block, Expr, ExprKind, MatchPattern, Program, SettingKind, Span, Stmt,
        TypeRef as SyntaxTypeRef,
    },
    catalog::Documentation,
    database::{CompilerDatabase, SemanticQueryResult},
    documentation::StandardLibraryDocumentation,
    hir::ExpressionResolution,
    language::{LanguageCatalog, LanguageItem, LanguageItemId, LanguageItemKind},
    lexer::{self, TokenKind},
    semantic::ResolvedCall,
    stdlib::{ItemKind, StandardLibrary, StdlibCapabilityId, StdlibItem, StdlibNamespace, TypeRef},
    stdlib_semantic::StandardLibrarySemanticExt,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionList {
    pub replacement: Span,
    pub items: Vec<CompletionItem>,
}

pub(crate) fn complete(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<CompletionList> {
    let source = database.source().to_owned();
    let compiler_context = database.context();
    let standard_library = compiler_context.standard_library();
    let offset = floor_char_boundary(&source, offset.min(source.len()));
    let syntax = database.recovering_parse()?.syntax().clone();
    if let Some(completions) = complete_state_decoder(&source, offset) {
        Ok(completions)
    } else if let Some(completions) =
        complete_type_argument(&source, &syntax, offset, &standard_library)
    {
        Ok(completions)
    } else if let Some(context) = member_context(&source, offset) {
        Ok(complete_member(&source, &syntax, context, compiler_context))
    } else {
        Ok(complete_root(&source, &syntax, offset, standard_library))
    }
}

fn complete_type_argument(
    source: &str,
    syntax: &Program,
    offset: usize,
    library: &StandardLibrary,
) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let before = &source[..replacement.start];
    let open = before.rfind('<')?;
    let name_end = open;
    let mut name_start = name_end;
    while name_start > 0 && is_identifier_byte(source.as_bytes()[name_start - 1]) {
        name_start -= 1;
    }
    if name_start == name_end {
        return None;
    }
    let name = &source[name_start..name_end];
    let is_generic_call = library
        .items()
        .iter()
        .any(|item| item.name == name && item.signature.explicit_type_parameters != 0);
    if !is_generic_call || source[open + 1..replacement.start].contains(['>', '(', ')', '{', '}']) {
        return None;
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    for ty in library.core_types() {
        builder.add(simple_completion(
            ty.name,
            CompletionKind::Type,
            "primitive type",
        ));
    }
    for ty in library.types() {
        builder.add(simple_completion(
            ty.name,
            CompletionKind::Type,
            "standard-library type",
        ));
    }
    for record in &syntax.records {
        builder.add(simple_completion(
            &record.name,
            CompletionKind::Struct,
            "record type",
        ));
    }
    for enumeration in &syntax.enums {
        builder.add(simple_completion(
            &enumeration.name,
            CompletionKind::Enum,
            "enum type",
        ));
    }
    Some(builder.finish())
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
    Some(builder.finish())
}

#[derive(Debug, Clone)]
struct MemberContext {
    receiver_path: Vec<String>,
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
) -> CompletionList {
    let replacement = identifier_span(source, offset);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);

    for item in LanguageCatalog::new().items() {
        if let Some(completion) = language_completion(item) {
            builder.add(completion);
        }
    }
    let provider = selected_provider(syntax, &standard_library);
    add_root_standard_library(&mut builder, &standard_library);
    if let Some(provider) = provider {
        let ty = standard_library.type_decl(provider.process_type);
        builder.add(CompletionItem {
            label: provider.value_name.to_owned(),
            kind: CompletionKind::Variable,
            detail: Some(ty.name.to_owned()),
            documentation: Some(render_documentation(&provider.documentation)),
            insert_text: provider.value_name.to_owned(),
            is_snippet: false,
        });
    }
    add_source_declarations(&mut builder, syntax);
    add_visible_bindings(&mut builder, syntax, offset);
    builder.finish()
}

fn complete_member(
    source: &str,
    syntax: &Program,
    context: MemberContext,
    compiler_context: crate::CompilerContext,
) -> CompletionList {
    let standard_library = compiler_context.standard_library();
    let mut builder = CompletionBuilder::new(context.prefix.clone(), context.replacement);
    let path = context
        .receiver_path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    match path.as_slice() {
        ["current"] | ["old"] => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::StateField,
                        "state field",
                    ));
                }
                if let Some(layout) = active_state_layout(syntax, source, context.dot) {
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
            }
        }
        ["settings"] | ["oldSettings"] => {
            for setting in &syntax.settings {
                if !matches!(setting.kind, SettingKind::Title { .. }) {
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
                        insert_text,
                        is_snippet,
                    });
                }
            }
            if let Some(provider) = selected_provider(syntax, &standard_library)
                && provider.value_name == *name
            {
                add_inferred_fields(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &standard_library,
                );
                add_inferred_methods(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &[],
                    &standard_library,
                );
            }
        }
        _ => {}
    }

    if !path.is_empty() {
        add_standard_library_path_members(&mut builder, &path, &standard_library);
    }

    if let Some((receiver, constraints, probe_syntax)) =
        infer_receiver(source, &context, compiler_context)
    {
        add_inferred_fields(&mut builder, &probe_syntax, &receiver, &standard_library);
        add_inferred_methods(
            &mut builder,
            &probe_syntax,
            &receiver,
            &constraints,
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
                .then(|| standard_library.source_state_provider())
                .flatten()
        })
}

fn language_completion(item: &LanguageItem) -> Option<CompletionItem> {
    if item.id == LanguageItemId::NativeStringDecoder {
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
        insert_text,
        is_snippet,
    }
}

fn add_root_standard_library(builder: &mut CompletionBuilder, library: &StandardLibrary) {
    for namespace in library
        .namespaces()
        .iter()
        .filter(|namespace| namespace.path.len() == 1)
    {
        builder.add(stdlib_namespace_completion(namespace));
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
            insert_text: ty.name.to_owned(),
            is_snippet: false,
        });
    }
    for item in library.items() {
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() == 1 {
            builder.add(stdlib_completion(
                item.name,
                item,
                CompletionKind::Function,
                library,
            ));
        }
    }
}

fn stdlib_namespace_completion(namespace: &StdlibNamespace) -> CompletionItem {
    CompletionItem {
        label: namespace.name.to_owned(),
        kind: CompletionKind::Namespace,
        detail: Some("standard-library namespace".to_owned()),
        documentation: Some(render_documentation(&namespace.documentation)),
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
                insert_text: variant.name.to_owned(),
                is_snippet: false,
            });
        }
    }

    for item in library.items() {
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() <= prefix.len() || path[..prefix.len()] != *prefix {
            continue;
        }
        let label = path[prefix.len()];
        if path.len() == prefix.len() + 1 {
            builder.add(stdlib_completion(
                label,
                item,
                CompletionKind::Function,
                library,
            ));
        }
    }

    for namespace in library.namespaces().iter().filter(|namespace| {
        namespace.path.len() == prefix.len() + 1 && namespace.path[..prefix.len()] == *prefix
    }) {
        builder.add(stdlib_namespace_completion(namespace));
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
        insert_text: function_snippet(label, item),
        is_snippet: true,
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

fn add_source_declarations(builder: &mut CompletionBuilder, syntax: &Program) {
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
            insert_text: format!("{}({parameters})", function.name),
            is_snippet: true,
        });
    }
    for record in &syntax.records {
        builder.add(simple_completion(
            &record.name,
            CompletionKind::Struct,
            "record type",
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
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::For { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::Return { .. }
        | Stmt::Throw { .. }
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
            add_expression_bindings(builder, &variable.value, offset);
        }
        Stmt::Assign { value, .. }
        | Stmt::Suspend { value, .. }
        | Stmt::Throw { error: value, .. }
        | Stmt::Expression(value) => {
            add_expression_bindings(builder, value, offset);
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
        Stmt::Return {
            value: Some(value), ..
        } => {
            add_expression_bindings(builder, value, offset);
        }
        Stmt::Return { value: None, .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
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
        ExprKind::Record { fields, .. } => {
            for (_, value) in fields {
                add_expression_bindings(builder, value, offset);
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
            match fallback {
                crate::ast::FallbackBranch::Value(value)
                | crate::ast::FallbackBranch::Return {
                    value: Some(value), ..
                } => add_expression_bindings(builder, value, offset),
                crate::ast::FallbackBranch::Return { value: None, .. }
                | crate::ast::FallbackBranch::Break { .. }
                | crate::ast::FallbackBranch::Continue { .. } => {}
            }
        }
        ExprKind::Suspend { value, .. }
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
        ExprKind::Error
        | ExprKind::None
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
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
        | MatchPattern::ResultSuccess(binding)
        | MatchPattern::ResultError(binding) => binding.as_ref(),
        MatchPattern::Bool(_)
        | MatchPattern::Int { .. }
        | MatchPattern::None
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
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Return { span, .. }
        | Stmt::Throw { span, .. }
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
) {
    match receiver {
        TypeKind::StateSnapshot => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "state field",
                    ));
                }
            }
        }
        TypeKind::SettingsView => {
            for setting in &syntax.settings {
                if !matches!(setting.kind, SettingKind::Title { .. }) {
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
        TypeKind::Record(id) => {
            if let Some(record) = syntax.records.iter().find(|record| record.id == *id) {
                for field in &record.fields {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "record field",
                    ));
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
                    insert_text: field.name.to_owned(),
                    is_snippet: false,
                });
            }
        }
        TypeKind::Builtin(_)
        | TypeKind::Enum(_)
        | TypeKind::GenericParameter { .. }
        | TypeKind::Array { .. }
        | TypeKind::Option { .. }
        | TypeKind::Result { .. }
        | TypeKind::Async { .. } => {}
    }
}

fn active_state_layout<'a>(
    syntax: &'a Program,
    source: &str,
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
                .is_some_and(|start| cursor_is_inside_braces(source, start, offset))
        })
        .or_else(|| {
            let before = source.get(..offset)?;
            let match_start = before.rfind("match layout")?;
            if !cursor_is_inside_braces(source, match_start, offset) {
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

fn cursor_is_inside_braces(source: &str, start: usize, offset: usize) -> bool {
    let Ok(lexed) = lexer::lex_lossless(source) else {
        return false;
    };
    let tokens = lexed.tokens().collect::<Vec<_>>();
    let Some(open) = tokens
        .iter()
        .position(|token| token.span.start >= start && matches!(token.kind, TokenKind::LBrace))
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
    let methods = standard_library
        .methods_for_type(receiver)
        .into_iter()
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
        (SyntaxTypeRef::Named(name), TypeKind::Record(actual)) => syntax
            .records
            .iter()
            .any(|record| record.name == syntax.type_name(name) && record.id == *actual),
        (SyntaxTypeRef::Named(name), TypeKind::Enum(actual)) => syntax
            .enums
            .iter()
            .any(|item| item.name == syntax.type_name(name) && item.id == *actual),
        (syntax, TypeKind::Builtin(actual)) => syntax.core_type() == Some(*actual),
        _ => false,
    }
}

fn infer_receiver(
    source: &str,
    context: &MemberContext,
    compiler_context: crate::CompilerContext,
) -> Option<(TypeKind, Vec<StdlibCapabilityId>, Program)> {
    let receiver_offset = context.dot.checked_sub(1)?;
    // When completing `receiver.method`, analysis at the end of `receiver`
    // can resolve the surrounding call. Its recorded receiver type is the
    // type whose methods we need. For `receiver.method().`, however, the
    // completed call is itself the new receiver, so its result type is the
    // correct one. Non-path receivers have no textual receiver segments.
    let use_resolved_call_receiver = !context.receiver_path.is_empty();
    if let Some(receiver) = analyze_receiver_source(
        source.to_owned(),
        receiver_offset,
        compiler_context.clone(),
        use_resolved_call_receiver,
    ) {
        return Some(receiver);
    }

    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..context.dot]);
    let suffix_start = completion_probe_suffix_end(source, context.replacement.end);
    probe_source.push_str(&source[suffix_start..]);
    analyze_receiver_source(probe_source, receiver_offset, compiler_context, false)
}

fn analyze_receiver_source(
    source: String,
    receiver_offset: usize,
    compiler_context: crate::CompilerContext,
    use_resolved_call_receiver: bool,
) -> Option<(TypeKind, Vec<StdlibCapabilityId>, Program)> {
    let mut database = CompilerDatabase::with_context(compiler_context, source);
    let analysis = database.analysis_at(receiver_offset).ok()??;
    let snapshot = database.semantic_snapshot().ok()?;
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
    let receiver = snapshot.semantics().types().kind(receiver_type).clone();
    let syntax = database.recovering_parse().ok()?.syntax().clone();
    Some((receiver, constraints, syntax))
}

/// Removes an already-written call and propagation suffix from a completion
/// probe. When completion is manually requested on `receiver.method()`, the
/// identifier replacement alone would leave `receiver()` and infer the wrong
/// expression (or fail to type it entirely). The probe needs the type of the
/// expression before the member dot, regardless of whether the member text is
/// partial or already followed by its postfix syntax.
fn completion_probe_suffix_end(source: &str, identifier_end: usize) -> usize {
    let Ok(lexed) = lexer::lex_lossless(source) else {
        return identifier_end;
    };
    let tokens = lexed.tokens().collect::<Vec<_>>();
    let Some(mut index) = tokens
        .iter()
        .position(|token| token.span.start >= identifier_end)
    else {
        return identifier_end;
    };
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

fn member_context(source: &str, offset: usize) -> Option<MemberContext> {
    let replacement = identifier_span(source, offset);
    let dot = replacement.start.checked_sub(1)?;
    if source.as_bytes().get(dot) != Some(&b'.') {
        return None;
    }
    let mut receiver_start = dot;
    while receiver_start > 0 {
        let byte = source.as_bytes()[receiver_start - 1];
        if is_identifier_byte(byte) || byte == b'.' {
            receiver_start -= 1;
        } else {
            break;
        }
    }
    let receiver_path = source[receiver_start..dot]
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    Some(MemberContext {
        receiver_path,
        dot,
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
    !name.is_empty() && name.bytes().all(is_identifier_byte) && !name.as_bytes()[0].is_ascii_digit()
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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
        insert_text: label.to_owned(),
        is_snippet: false,
    }
}

fn render_documentation<Id>(documentation: &Documentation<Id>) -> String {
    format!("{}\n\n{}", documentation.summary, documentation.details)
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
    fn completes_domains_catalogs_and_inferred_members() {
        let source = r#"
record Position {
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

        let mut keyed_settings = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings { \"Flag\" => flag key \"flag\": true }\nwhileAttached { settings.en }",
        );
        assert!(labels(&mut keyed_settings, "settings.en").contains(&"enabled".to_owned()));
    }

    #[test]
    fn completes_methods_after_fields_on_expression_receivers() {
        let source = r#"
record Path {
    address: address
}

fn Path.resolve() {
    return self.address
}

record Layout {
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
    fn completes_catalog_methods_on_existing_expression_receiver_calls() {
        let source = r#"
record Layout {
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
        for array_method in ["set", "length", "isEmpty"] {
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
    }

    #[test]
    fn completes_string_members_in_match_arms_and_on_inferred_locals() {
        let source = include_str!("../examples/minish_cap.split");

        let literal_source = source.replacen("2 => \"½\"", "2 => \"½\".", 1);
        let mut database = CompilerDatabase::new(literal_source);
        let literal = labels(&mut database, "2 => \"½\".");
        for member in [
            "byteLength",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "replaceAll",
            "slice",
        ] {
            assert!(literal.contains(&member.to_owned()), "{literal:#?}");
        }

        let local_source = source.replacen("return fraction", "return fraction.", 1);
        let mut database = CompilerDatabase::new(local_source);
        let local = labels(&mut database, "return fraction.");
        for member in [
            "byteLength",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "replaceAll",
            "slice",
        ] {
            assert!(local.contains(&member.to_owned()), "{local:#?}");
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

        let cast_source = "state \"game.exe\" {} whileAttached { let value = 1 as ut }";
        let mut database = CompilerDatabase::new(cast_source);
        let offset = cast_source.find("ut }").unwrap() + 2;
        assert!(
            database
                .completions(offset)
                .unwrap()
                .items
                .iter()
                .all(|item| item.label != "utf8")
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
    fn state_normalizer_bindings_complete_with_the_field_type() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 normalize value.
}"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(!completions.contains(&"length".to_owned()));
    }
}
