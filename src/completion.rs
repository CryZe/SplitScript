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
    language::{LanguageCatalog, LanguageItem, LanguageItemKind},
    stdlib::{ItemKind, StandardLibrary, StdlibItem, StdlibNamespace},
    stdlib_semantic::StandardLibrarySemanticExt,
    types::TypeKind,
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
    if let Some(context) = member_context(&source, offset) {
        Ok(complete_member(&source, &syntax, context, compiler_context))
    } else {
        Ok(complete_root(&source, &syntax, offset, standard_library))
    }
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
    let provider = syntax
        .state
        .as_ref()
        .and_then(|state| state.provider.as_ref())
        .and_then(|provider| standard_library.state_provider_by_name(&provider.name));
    add_root_standard_library(&mut builder, &standard_library, provider.is_some());
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
                for field in &state.fields {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::StateField,
                        "state field",
                    ));
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
            if let Some(enumeration) = syntax.enums.iter().find(|item| item.name == *name) {
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
        }
        _ => {}
    }

    add_standard_library_path_members(&mut builder, &path, &standard_library);

    if let Some((receiver, probe_syntax)) = infer_receiver(source, &context, compiler_context) {
        add_inferred_fields(&mut builder, &probe_syntax, &receiver, &standard_library);
        add_inferred_methods(&mut builder, &probe_syntax, &receiver, &standard_library);
    }
    builder.finish()
}

fn language_completion(item: &LanguageItem) -> Option<CompletionItem> {
    let (kind, insert_text, is_snippet) = match item.kind {
        LanguageItemKind::Action(_) => (
            CompletionKind::Snippet,
            format!("{} {{\n    $0\n}}", item.name),
            true,
        ),
        LanguageItemKind::BuiltinType(_) => (CompletionKind::Type, item.name.to_owned(), false),
        LanguageItemKind::SnapshotRoot => (CompletionKind::Variable, item.name.to_owned(), false),
        LanguageItemKind::Keyword | LanguageItemKind::Declaration => {
            (CompletionKind::Keyword, item.name.to_owned(), false)
        }
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

fn add_root_standard_library(
    builder: &mut CompletionBuilder,
    library: &StandardLibrary,
    hide_process: bool,
) {
    for namespace in library.namespaces().iter().filter(|namespace| {
        namespace.path.len() == 1 && !(hide_process && namespace.path == ["process"])
    }) {
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

    for item in library.items() {
        if !matches!(item.kind, ItemKind::TypedFunction { .. }) {
            continue;
        }
        if library
            .item_path(item)
            .is_some_and(|declared| declared == prefix)
        {
            for ty in memory_read_type_names() {
                let documentation =
                    StandardLibraryDocumentation::generate_with_library(library, item.id, &[]);
                builder.add(CompletionItem {
                    label: (*ty).to_owned(),
                    kind: CompletionKind::Type,
                    detail: Some(format!("{}.{ty}", item.qualified_name)),
                    documentation: Some(documentation.summary_markdown()),
                    insert_text: function_snippet(ty, item),
                    is_snippet: true,
                });
            }
        }
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
    for enumeration in &syntax.enums {
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
        ExprKind::Enum { payload, .. } => {
            if let Some(payload) = payload {
                add_expression_bindings(builder, payload, offset);
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
        ExprKind::Propagate(value)
        | ExprKind::Member {
            receiver: value, ..
        }
        | ExprKind::Unary { expr: value, .. } => {
            add_expression_bindings(builder, value, offset);
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
        | TypeKind::Array { .. }
        | TypeKind::Option { .. }
        | TypeKind::Result { .. } => {}
    }
}

fn add_inferred_methods(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    receiver: &TypeKind,
    standard_library: &StandardLibrary,
) {
    for item in standard_library.methods_for_type(receiver) {
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
        (SyntaxTypeRef::Record(expected), TypeKind::Record(actual)) => expected == *actual,
        (SyntaxTypeRef::Enum(expected), TypeKind::Enum(actual)) => expected == *actual,
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
        (SyntaxTypeRef::Standard(expected), TypeKind::Standard(actual)) => expected == *actual,
        (syntax, TypeKind::Builtin(actual)) => syntax.core_type() == Some(*actual),
        _ => false,
    }
}

fn infer_receiver(
    source: &str,
    context: &MemberContext,
    compiler_context: crate::CompilerContext,
) -> Option<(TypeKind, Program)> {
    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..context.dot]);
    probe_source.push_str(&source[context.replacement.end..]);
    let mut probe = CompilerDatabase::with_context(compiler_context, probe_source);
    let receiver_offset = context.dot.checked_sub(1)?;
    let receiver = probe.analysis_at(receiver_offset).ok()??.type_kind;
    let syntax = probe.recovering_parse().ok()?.syntax().clone();
    Some((receiver, syntax))
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

fn memory_read_type_names() -> &'static [&'static str] {
    &[
        "bool", "i8", "u8", "i16", "u16", "i32", "u32", "i64", "u64", "f32", "f64", "address",
    ]
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
    Idle
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
    current.po
    settings.en
    Mode.Ac
    process.re
    process.read.i
    number.cl
    current.position.co
    current.position.x
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "current.po").contains(&"position".to_owned()));
        assert!(labels(&mut database, "settings.en").contains(&"enabled".to_owned()));
        assert!(labels(&mut database, "Mode.Ac").contains(&"Active".to_owned()));
        assert!(labels(&mut database, "process.re").contains(&"read".to_owned()));
        assert!(labels(&mut database, "process.read.i").contains(&"i32".to_owned()));
        assert!(labels(&mut database, "number.cl").contains(&"clamp".to_owned()));
        assert!(labels(&mut database, "module.ad").contains(&"address".to_owned()));
        assert!(labels(&mut database, "current.position.co").contains(&"coordinate".to_owned()));
        assert!(labels(&mut database, "current.position.x").contains(&"x".to_owned()));

        let mut bare = CompilerDatabase::new(
            "state \"game.exe\" {}\nwhileAttached { let number: i32 = 4\nnumber.\n}",
        );
        assert!(labels(&mut bare, "number.").contains(&"min".to_owned()));
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
}
