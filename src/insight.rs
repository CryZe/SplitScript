//! Semantic hover and signature information for editor clients.

use std::sync::Arc;

use crate::{
    ast::{Expr, ExprKind, Program, Span},
    database::{
        CompilerDatabase, DefinitionTarget, SemanticQueryResult, SemanticSnapshot,
        SourceDefinition, SourceDefinitionId,
    },
    documentation::{StandardLibraryDocumentation, language_item_uri, symbol_uri},
    effects::FunctionOperationSemantics,
    language::{LanguageCatalog, LanguageItem},
    lexer::{Token, TokenKind},
    semantic::ResolvedCall,
    stdlib::{
        StandardLibrary, StateProviderProcesses, StdlibItem, StdlibItemId, StdlibSymbolId,
        TypeRef as CatalogTypeRef,
    },
    stdlib_semantic::StandardLibrarySemanticExt,
    syntax::SourceDocument,
    type_display::display_type,
    types::{BuiltinType, TypeId, TypeKind},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub span: Span,
    pub markdown: String,
    /// Stable compiler-owned reference page for the hovered catalog symbol.
    pub documentation_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterInformation {
    pub label: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInformation {
    pub label: String,
    pub documentation: String,
    pub parameters: Vec<ParameterInformation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: usize,
    pub active_parameter: usize,
}

pub(crate) fn hover(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<HoverInfo>> {
    let offset = database.hover_query_offset(offset)?;
    let standard_library = database.context().standard_library();
    let Some(token) = database.token_at(offset)? else {
        return Ok(None);
    };
    if let Some(markdown) = float_literal_markdown(database, &token, offset)? {
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown,
            documentation_uri: None,
        }));
    }
    let target = database.definition_at_query_offset(offset)?;
    if target.is_none()
        && let Some(analysis) = database.analysis_at(offset)?
        && let Some(ExpressionResolution::ValuePath {
            root: Some(crate::semantic::ResolvedValue::Variable(value)),
            members,
        }) = analysis.resolution
        && members.is_empty()
        && let Some(context) = semantic_context(database)
        && let Some((name, description)) = state_transform_binding(value, &context)
    {
        let ty = render_type(analysis.ty, &context);
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown: source_markdown(&format!("{name}: {ty}"), description),
            documentation_uri: None,
        }));
    }
    let Some(target) = target else {
        return expression_type_hover(database, offset);
    };
    let (markdown, documentation_uri) = match target {
        DefinitionTarget::StandardLibrary(item) => {
            let type_arguments = database
                .analysis_at(offset)?
                .and_then(|analysis| match analysis.resolution {
                    Some(ExpressionResolution::Call(ResolvedCall::StandardLibrary {
                        item: resolved,
                        type_arguments,
                        ..
                    })) if resolved == item => Some(type_arguments),
                    _ => None,
                })
                .unwrap_or_default();
            let semantic = semantic_context(database);
            (
                render_stdlib_hover(
                    standard_library.clone(),
                    item,
                    substitutions(item, &type_arguments, semantic.as_ref()),
                ),
                Some(symbol_uri(StdlibSymbolId::Item(item), &standard_library)),
            )
        }
        DefinitionTarget::StandardLibrarySymbol(symbol) => (
            render_stdlib_symbol_hover(standard_library.clone(), symbol),
            Some(symbol_uri(symbol, &standard_library)),
        ),
        DefinitionTarget::Language(item) => (
            render_language_hover(LanguageCatalog::new().item(item)),
            Some(language_item_uri(item)),
        ),
        DefinitionTarget::Source(definition) => {
            if definition.id == SourceDefinitionId::State
                && let Some(provider) = database
                    .analysis_at(offset)?
                    .as_ref()
                    .and_then(provider_value_for_resolution)
            {
                return Ok(Some(HoverInfo {
                    span: token.span,
                    markdown: render_stdlib_symbol_hover(
                        standard_library.clone(),
                        StdlibSymbolId::StateProvider(provider),
                    ),
                    documentation_uri: Some(symbol_uri(
                        StdlibSymbolId::StateProvider(provider),
                        &standard_library,
                    )),
                }));
            }
            let Some(context) = semantic_context(database) else {
                return Ok(None);
            };
            let Some(markdown) = render_source_hover(&definition, &context) else {
                return Ok(None);
            };
            (markdown, None)
        }
    };
    Ok(Some(HoverInfo {
        span: token.span,
        markdown,
        documentation_uri,
    }))
}

fn expression_type_hover(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<HoverInfo>> {
    let Some(analysis) = database.analysis_at(offset)? else {
        return Ok(None);
    };
    if matches!(analysis.type_kind, TypeKind::Error) {
        return Ok(None);
    }
    let Some(context) = semantic_context(database) else {
        return Ok(None);
    };
    Ok(Some(HoverInfo {
        span: analysis.span,
        markdown: format!(
            "```splitscript\n{}\n```",
            render_type(analysis.ty, &context)
        ),
        documentation_uri: None,
    }))
}

fn provider_value_for_resolution(
    analysis: &crate::database::PositionAnalysis,
) -> Option<crate::stdlib::StdlibStateProviderId> {
    let root = match analysis.resolution.as_ref()? {
        ExpressionResolution::ValuePath { root, .. } => *root,
        ExpressionResolution::Call(call) => call.receiver()?.path().map(|(root, _)| root),
        ExpressionResolution::Member { .. }
        | ExpressionResolution::RecordLiteral { .. }
        | ExpressionResolution::EnumConstructor { .. } => None,
    }?;
    match root {
        crate::semantic::ResolvedValue::ProviderValue(provider) => Some(provider),
        crate::semantic::ResolvedValue::Variable(_)
        | crate::semantic::ResolvedValue::CurrentSnapshot
        | crate::semantic::ResolvedValue::OldSnapshot
        | crate::semantic::ResolvedValue::SettingsView
        | crate::semantic::ResolvedValue::OldSettingsView
        | crate::semantic::ResolvedValue::CurrentState(_)
        | crate::semantic::ResolvedValue::OldState(_)
        | crate::semantic::ResolvedValue::Setting(_)
        | crate::semantic::ResolvedValue::OldSetting(_) => None,
    }
}

fn float_literal_markdown(
    database: &mut CompilerDatabase,
    token: &Token,
    offset: usize,
) -> SemanticQueryResult<Option<String>> {
    let TokenKind::Float(spelling) = &token.kind else {
        return Ok(None);
    };
    let Some(analysis) = database.analysis_at(offset)? else {
        return Ok(None);
    };
    let normalized = spelling.replace('_', "");
    let (ty, bits) = match analysis.type_kind {
        TypeKind::Builtin(BuiltinType::F32) => {
            let Ok(value) = normalized.parse::<f32>() else {
                return Ok(None);
            };
            ("f32", format!("0x{:08x}", value.to_bits()))
        }
        TypeKind::Builtin(BuiltinType::F64) => {
            let Ok(value) = normalized.parse::<f64>() else {
                return Ok(None);
            };
            ("f64", format!("0x{:016x}", value.to_bits()))
        }
        _ => return Ok(None),
    };
    Ok(Some(format!(
        "```splitscript\n{spelling}: {ty}\n```\n\n**Rounded IEEE-754 bits:** `{bits}`"
    )))
}

fn state_transform_binding(
    value: crate::ast::ValueId,
    context: &SemanticContext,
) -> Option<(&'static str, &'static str)> {
    context
        .syntax()
        .state
        .as_ref()?
        .all_fields()
        .find_map(|field| {
            let transform = field.transform.as_ref()?;
            if transform.value == value {
                Some(("value", "Raw candidate for this state field."))
            } else {
                None
            }
        })
}

pub(crate) fn signature_help(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<SignatureHelp>> {
    let compiler_context = database.context();
    let standard_library = compiler_context.standard_library();
    let recovered = database.recovering_parse()?;
    let document = recovered.source_document().clone();
    let syntax = recovered.syntax().clone();
    let Some(call_site) = active_call(&document, offset) else {
        return Ok(None);
    };
    let mut semantic = semantic_context(database);
    let resolved = semantic.as_ref().and_then(|context| {
        call_expression_at(&syntax, call_site.open, &call_site.callee)
            .and_then(|expression| context.semantics().call(expression.id))
            .and_then(|call| match call {
                ResolvedCall::StandardLibrary {
                    item,
                    type_arguments,
                    ..
                } => Some((*item, type_arguments.clone())),
                _ => None,
            })
    });
    let mut selected = resolved.or_else(|| {
        standard_library
            .resolve_path(&call_site.callee)
            .map(|candidate| (candidate.item.id, Vec::new()))
    });
    if selected.is_none()
        && let Some((item, type_arguments, probe_semantic)) =
            infer_method_call(document.source(), &call_site, &compiler_context)
    {
        selected = Some((item, type_arguments));
        semantic = Some(probe_semantic);
    }
    let Some((item, type_arguments)) = selected else {
        return Ok(None);
    };
    let substitutions = substitutions(item, &type_arguments, semantic.as_ref());
    let documentation = StandardLibraryDocumentation::generate_with_library(
        &standard_library,
        item,
        &substitutions,
    );
    let signature = SignatureInformation {
        label: documentation.signature.clone(),
        documentation: documentation.details_markdown(),
        parameters: documentation
            .parameters
            .iter()
            .map(|parameter| ParameterInformation {
                label: parameter.name.to_owned(),
                documentation: parameter.documentation.to_owned(),
            })
            .collect(),
    };
    let active_parameter = call_site
        .active_parameter
        .min(signature.parameters.len().saturating_sub(1));
    Ok(Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: 0,
        active_parameter,
    }))
}

use crate::hir::ExpressionResolution;

struct SemanticContext {
    standard_library: StandardLibrary,
    snapshot: Arc<SemanticSnapshot>,
}

fn semantic_context(database: &mut CompilerDatabase) -> Option<SemanticContext> {
    let snapshot = database.semantic_snapshot().ok()?;
    Some(SemanticContext {
        standard_library: snapshot.context().standard_library(),
        snapshot,
    })
}

impl SemanticContext {
    fn syntax(&self) -> &crate::ast::Program {
        self.snapshot.syntax()
    }

    fn semantics(&self) -> &crate::semantic::SemanticModel {
        self.snapshot.semantics()
    }

    fn effects(&self) -> Option<&crate::effects::OperationAnalysis> {
        self.snapshot.effects()
    }
}

fn substitutions(
    item: StdlibItemId,
    arguments: &[TypeId],
    context: Option<&SemanticContext>,
) -> Vec<(&'static str, String)> {
    let Some(context) = context else {
        return Vec::new();
    };
    context
        .standard_library
        .item(item)
        .signature
        .type_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, ty)| (parameter.name, render_type(*ty, context)))
        .collect()
}

fn render_type(ty: TypeId, context: &SemanticContext) -> String {
    display_type(ty, &context.snapshot)
}

fn syntax_for_binding(
    syntax: &crate::ast::Program,
    value: crate::ast::ValueId,
) -> Option<&crate::ast::ForBinding> {
    struct Finder<'ast> {
        value: crate::ast::ValueId,
        found: Option<&'ast crate::ast::ForBinding>,
    }

    impl<'ast> crate::visit::Visitor<'ast> for Finder<'ast> {
        fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
            if binding.id == self.value {
                self.found = Some(binding);
            }
        }
    }

    let mut finder = Finder { value, found: None };
    crate::visit::Visitor::visit_program(&mut finder, syntax);
    finder.found
}

fn render_source_hover(definition: &SourceDefinition, context: &SemanticContext) -> Option<String> {
    let syntax = context.syntax();
    let semantics = context.semantics();
    match definition.id {
        SourceDefinitionId::State => Some(source_markdown(
            "current / old: state snapshot",
            "`current` contains the latest accepted state and permits direct field replacement; `old` is the read-only preceding snapshot. Failed fields retain their accepted values.",
        )),
        SourceDefinitionId::Settings => Some(source_markdown(
            "settings / oldSettings: settings view",
            "Read-only, allocation-free views. `settings` selects the latest host settings and `oldSettings` selects their values from the preceding update.",
        )),
        SourceDefinitionId::Value(value) => {
            let ty = semantics.value_type(value)?;
            let ty = render_type(ty, context);
            let (signature, description) = if syntax
                .state
                .as_ref()
                .is_some_and(|state| state.layout_value == Some(value))
            {
                (
                    format!("layout: {ty}"),
                    "Read-only memory layout selected for the attached game build.".to_owned(),
                )
            } else if let Some(global) = syntax.globals.iter().find(|global| global.id == value) {
                (
                    format!("let {}: {ty}", definition.name),
                    documented_description("Global variable", global.documentation.as_deref()),
                )
            } else if let Some(field) = syntax
                .state
                .as_ref()
                .and_then(|state| state.all_fields().find(|field| field.id == value))
            {
                (
                    format!("current.{}: {ty}", definition.name),
                    documented_description(
                        "Transactional state field",
                        field.documentation.as_deref(),
                    ),
                )
            } else if let Some(setting) = syntax.settings.iter().find(|setting| setting.id == value)
            {
                let mut description = format!("Setting: {}", setting.description);
                if let Some(tooltip) = &setting.tooltip {
                    description.push_str("\n\n");
                    description.push_str(tooltip);
                }
                (format!("settings.{}: {ty}", definition.name), description)
            } else if syntax
                .functions
                .iter()
                .flat_map(|function| &function.params)
                .any(|parameter| parameter.id == value)
            {
                (format!("{}: {ty}", definition.name), "Parameter".to_owned())
            } else if syntax_for_binding(syntax, value).is_some() {
                (
                    format!("{}: {ty}", definition.name),
                    "Read-only loop binding".to_owned(),
                )
            } else {
                (
                    format!("let {}: {ty}", definition.name),
                    "Local variable".to_owned(),
                )
            };
            Some(source_markdown(&signature, &description))
        }
        SourceDefinitionId::RecordField(field) => {
            let ty = semantics.record_field_type(field)?;
            let ty = render_type(ty, context);
            let record = syntax
                .records
                .iter()
                .find(|record| record.fields.iter().any(|candidate| candidate.id == field))?;
            let field = record
                .fields
                .iter()
                .find(|candidate| candidate.id == field)?;
            Some(source_markdown(
                &format!("{}.{}: {ty}", record.name, definition.name),
                &documented_description("Record field", field.documentation.as_deref()),
            ))
        }
        SourceDefinitionId::Function(function) => {
            let function = syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)?;
            let receiver = function.method_of.and_then(|_| {
                function
                    .params
                    .first()
                    .and_then(|parameter| semantics.value_type(parameter.id))
                    .map(|ty| render_type(ty, context))
            });
            let parameters = function
                .params
                .iter()
                .filter(|parameter| receiver.is_none() || parameter.name != "self")
                .map(|parameter| {
                    let ty = semantics.value_type(parameter.id)?;
                    Some(format!("{}: {}", parameter.name, render_type(ty, context)))
                })
                .collect::<Option<Vec<_>>>()?;
            let result = semantics.function_result(function.id)?;
            let bounds = semantics
                .function_type_parameters(function.id)
                .iter()
                .filter_map(|parameter| {
                    let constraints = context
                        .standard_library
                        .minimal_capabilities(semantics.generic_parameter_constraints(*parameter));
                    (!constraints.is_empty()).then(|| {
                        format!(
                            "{}: {}",
                            render_type(*parameter, context),
                            constraints
                                .iter()
                                .map(|constraint| {
                                    context.standard_library.capability(*constraint).name
                                })
                                .collect::<Vec<_>>()
                                .join(" + ")
                        )
                    })
                })
                .collect::<Vec<_>>();
            let name = receiver.map_or_else(
                || function.name.clone(),
                |receiver| format!("{receiver}.{}", function.name),
            );
            let description = source_function_description(
                function.method_of.is_some(),
                function.debug_only,
                function.documentation.as_deref(),
                context
                    .effects()
                    .map(|effects| effects.function(function.id)),
            );
            Some(source_markdown(
                &format!(
                    "fn {name}({}) -> {}{}",
                    parameters.join(", "),
                    render_type(result, context),
                    if bounds.is_empty() {
                        String::new()
                    } else {
                        format!(" where {}", bounds.join(", "))
                    }
                ),
                &description,
            ))
        }
        SourceDefinitionId::Record(record) => {
            let record = syntax
                .records
                .iter()
                .find(|candidate| candidate.id == record)?;
            Some(source_markdown(
                &format!("record {}", record.name),
                &documented_description("Record type", record.documentation.as_deref()),
            ))
        }
        SourceDefinitionId::Enum(enumeration) => {
            let enumeration = syntax
                .enum_declarations()
                .find(|candidate| candidate.id == enumeration)?;
            Some(source_markdown(
                &format!("enum {}", enumeration.name),
                &documented_description("Enum type", enumeration.documentation.as_deref()),
            ))
        }
        SourceDefinitionId::EnumVariant(variant) => {
            let enumeration = syntax.enum_declarations().find(|enumeration| {
                enumeration
                    .variants
                    .iter()
                    .any(|candidate| candidate.id == variant)
            })?;
            let payload = semantics
                .enum_variant_payload(variant)
                .map(|ty| format!("({})", render_type(ty, context)));
            let variant_documentation = enumeration
                .variants
                .iter()
                .find(|candidate| candidate.id == variant)
                .and_then(|variant| variant.documentation.as_deref());
            Some(source_markdown(
                &format!(
                    "{}.{}{}",
                    enumeration.name,
                    definition.name,
                    payload.as_deref().unwrap_or_default()
                ),
                &documented_description("Enum variant", variant_documentation),
            ))
        }
    }
}

fn source_function_description(
    is_method: bool,
    debug_only: bool,
    documentation: Option<&str>,
    operation: Option<FunctionOperationSemantics>,
) -> String {
    let mut description =
        documented_description(if is_method { "Method" } else { "Function" }, documentation);
    if debug_only {
        description.push_str("\n\n**Build availability:** Debug builds only");
    }
    if let Some(operation) = operation {
        description.push_str("\n\n**Effects:** ");
        description.push_str(
            &operation
                .effects
                .iter()
                .map(|effect| effect.name())
                .collect::<Vec<_>>()
                .join(", "),
        );
        description.push_str("\n\n**Runtime behavior:** ");
        description.push_str(match operation.suspension {
            crate::stdlib::SuspensionKind::None => "synchronous",
            crate::stdlib::SuspensionKind::Retryable => "await retries until successful",
            crate::stdlib::SuspensionKind::Suspends => "suspends",
        });
        if operation.availability == crate::stdlib::Availability::OnAttach {
            description.push_str("; available only in `onAttach`");
        }
        if operation.requires_attached_process {
            description.push_str("; requires an attached process and is unavailable in `onDetach`");
        }
        if operation.requires_state_snapshots {
            description.push_str("; requires committed `old` and `current` state snapshots");
        }
        if operation.cancellation == crate::stdlib::CancellationKind::ProcessClose {
            description.push_str("; cancels when the process closes");
        }
    }
    description
}

fn documented_description(kind: &str, documentation: Option<&str>) -> String {
    let mut description = kind.to_owned();
    if let Some(documentation) = documentation {
        description.push_str("\n\n");
        description.push_str(documentation);
    }
    description
}

fn source_markdown(signature: &str, description: &str) -> String {
    format!("```splitscript\n{signature}\n```\n\n{description}")
}

fn render_stdlib_hover(
    standard_library: StandardLibrary,
    item: StdlibItemId,
    substitutions: Vec<(&str, String)>,
) -> String {
    StandardLibraryDocumentation::generate_with_library(&standard_library, item, &substitutions)
        .hover_markdown()
}

fn render_stdlib_symbol_hover(library: StandardLibrary, symbol: StdlibSymbolId) -> String {
    let (form, documentation) = match symbol {
        StdlibSymbolId::StateProvider(id) => {
            let declaration = library.state_provider(id);
            let value_type = library.type_decl(declaration.process_type);
            let state_form = match declaration.processes {
                StateProviderProcesses::SourceState => "state \"game.exe\" { ... }".to_owned(),
                StateProviderProcesses::Declared(_) => {
                    format!("state {} {{ ... }}", declaration.name)
                }
            };
            (
                format!(
                    "{state_form}\n{}: {}",
                    declaration.value_name, value_type.name
                ),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Namespace(id) => {
            let declaration = library.namespace(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::Capability(id) => {
            let declaration = library.capability(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::TypeConstructor(id) => {
            let declaration = library.type_constructor(id);
            let mut form = library.render_type_constructor(id);
            if declaration.syntax == crate::stdlib::TypeConstructorSyntax::Named
                && !declaration.parameters.is_empty()
            {
                form = declaration.name.to_owned();
                form.push('<');
                for (index, parameter) in declaration.parameters.iter().enumerate() {
                    if index != 0 {
                        form.push_str(", ");
                    }
                    form.push_str(parameter.name);
                    let constraints = library.minimal_capabilities(parameter.constraints);
                    if !constraints.is_empty() {
                        form.push_str(": ");
                        for (constraint_index, constraint) in constraints.iter().enumerate() {
                            if constraint_index != 0 {
                                form.push_str(" + ");
                            }
                            form.push_str(library.capability(*constraint).name);
                        }
                    }
                }
                form.push('>');
            }
            (form, &declaration.documentation)
        }
        StdlibSymbolId::Type(id) => {
            let declaration = library.type_decl(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::Field(id) => {
            let declaration = library.field(id);
            let owner = library.type_decl(declaration.owner);
            (
                format!(
                    "{}.{}: {}",
                    owner.name,
                    declaration.name,
                    library.render_type(declaration.ty)
                ),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Variant(id) => {
            let declaration = library.variant(id);
            let owner = library.type_decl(declaration.owner);
            (
                format!("{}.{}", owner.name, declaration.name),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Item(id) => return render_stdlib_hover(library, id, Vec::new()),
    };
    let mut markdown = format!(
        "```splitscript\n{form}\n```\n\n{}\n\n{}",
        crate::documentation::strip_intra_doc_links(documentation.summary),
        crate::documentation::strip_intra_doc_links(documentation.details)
    );
    append_examples(&mut markdown, documentation.examples);
    markdown
}

fn render_language_hover(item: &LanguageItem) -> String {
    let mut markdown = format!(
        "```splitscript\n{}\n```\n\n{}\n\n{}",
        item.form,
        crate::documentation::strip_intra_doc_links(item.documentation.summary),
        crate::documentation::strip_intra_doc_links(item.documentation.details)
    );
    append_examples(&mut markdown, item.documentation.examples);
    markdown
}

fn append_examples(markdown: &mut String, examples: &[crate::catalog::Example]) {
    if examples.is_empty() {
        return;
    }
    markdown.push_str("\n\n**Examples**");
    for example in examples {
        markdown.push_str(&format!(
            "\n\n_{}_\n\n```splitscript\n{}\n```",
            example.title, example.source
        ));
    }
}

#[derive(Debug)]
struct CallSite {
    open: Span,
    callee: Vec<String>,
    explicit_type_arguments: Vec<String>,
    method_dot: Option<usize>,
    active_parameter: usize,
}

fn active_call(document: &SourceDocument, offset: usize) -> Option<CallSite> {
    let tokens = document
        .tokens()
        .filter(|token| token.span.start < offset && token.kind != TokenKind::Eof)
        .collect::<Vec<_>>();
    let mut parentheses = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen => parentheses.push(index),
            TokenKind::RParen => {
                parentheses.pop();
            }
            _ => {}
        }
    }
    let open_index = *parentheses.last()?;
    let (callee, explicit_type_arguments, method_dot) = callee_before(&tokens, open_index)?;
    let active_parameter = active_parameter(&tokens[open_index + 1..]);
    Some(CallSite {
        open: tokens[open_index].span,
        callee,
        explicit_type_arguments,
        method_dot,
        active_parameter,
    })
}

fn callee_before(
    tokens: &[&Token],
    open: usize,
) -> Option<(Vec<String>, Vec<String>, Option<usize>)> {
    let mut cursor = open.checked_sub(1)?;
    let mut explicit_type_arguments = Vec::new();
    if tokens[cursor].kind == TokenKind::Gt {
        let mut depth = 1usize;
        while cursor > 0 {
            cursor -= 1;
            match tokens[cursor].kind {
                TokenKind::Gt => depth += 1,
                TokenKind::Lt => {
                    depth -= 1;
                    if depth == 0 {
                        cursor = cursor.checked_sub(1)?;
                        break;
                    }
                }
                TokenKind::Ident(ref name) if depth == 1 => {
                    explicit_type_arguments.push(name.clone())
                }
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
        explicit_type_arguments.reverse();
    }
    let method_dot = cursor
        .checked_sub(1)
        .filter(|index| tokens[*index].kind == TokenKind::Dot)
        .map(|index| tokens[index].span.start);
    let mut reversed = Vec::new();
    while let TokenKind::Ident(name) = &tokens[cursor].kind {
        reversed.push(name.clone());
        let Some(dot) = cursor.checked_sub(1) else {
            break;
        };
        if tokens[dot].kind != TokenKind::Dot {
            break;
        }
        let Some(previous) = dot.checked_sub(1) else {
            break;
        };
        cursor = previous;
    }
    (!reversed.is_empty()).then(|| {
        reversed.reverse();
        (reversed, explicit_type_arguments, method_dot)
    })
}

fn infer_method_call(
    source: &str,
    call: &CallSite,
    compiler_context: &crate::CompilerContext,
) -> Option<(StdlibItemId, Vec<TypeId>, SemanticContext)> {
    let method_dot = call.method_dot?;
    let line_end = source[call.open.start..]
        .find('\n')
        .map_or(source.len(), |relative| call.open.start + relative);
    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..method_dot]);
    probe_source.push_str(&source[line_end..]);
    let mut probe = CompilerDatabase::with_context(compiler_context.clone(), probe_source);
    let line_start = source[..method_dot]
        .rfind(['\n', '\r'])
        .map_or(0, |offset| offset + 1);
    let analysis = (line_start..method_dot)
        .rev()
        .find_map(|offset| probe.analysis_at(offset).ok().flatten());
    let analysis = analysis?;
    let semantic = semantic_context(&mut probe);
    let semantic = semantic?;
    let applicable = compiler_context
        .standard_library()
        .methods_for_type(&analysis.type_kind);
    let method_name = call.callee.last()?;
    let item = applicable.into_iter().find(|item| {
        matches!(item.kind, crate::stdlib::ItemKind::Method { .. }) && item.name == method_name
    })?;
    let type_arguments = if call.explicit_type_arguments.is_empty() {
        inferred_method_type_arguments(item, analysis.ty, &analysis.type_kind)
    } else {
        call.explicit_type_arguments
            .iter()
            .filter_map(|name| {
                compiler_context
                    .standard_library()
                    .core_types()
                    .iter()
                    .find(|ty| ty.name == name)
                    .map(|ty| semantic.semantics().types().id_for_core(ty.id))
            })
            .collect()
    };
    Some((item.id, type_arguments, semantic))
}

fn inferred_method_type_arguments(
    item: &StdlibItem,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Vec<TypeId> {
    let declared = match item.kind {
        crate::stdlib::ItemKind::Method { receiver } => receiver,
        crate::stdlib::ItemKind::Function => {
            return Vec::new();
        }
    };
    item.signature
        .type_parameters
        .iter()
        .filter_map(|parameter| {
            catalog_receiver_argument(declared, parameter.name, receiver, receiver_kind)
        })
        .collect()
}

fn catalog_receiver_argument(
    declared: CatalogTypeRef,
    parameter: &str,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Option<TypeId> {
    match (declared, receiver_kind) {
        (CatalogTypeRef::Parameter(name), _) if name == parameter => Some(receiver),
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Array {
                element: actual, ..
            },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Array && *name == parameter => {
            Some(*actual)
        }
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Option { value: actual, .. },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Option
            && *name == parameter =>
        {
            Some(*actual)
        }
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Result { value: actual, .. },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Result
            && *name == parameter =>
        {
            Some(*actual)
        }
        _ => None,
    }
}

fn active_parameter(tokens: &[&Token]) -> usize {
    let mut nesting = 0usize;
    let mut parameter = 0usize;
    for token in tokens {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => nesting += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                nesting = nesting.saturating_sub(1);
            }
            TokenKind::Comma if nesting == 0 => parameter += 1,
            _ => {}
        }
    }
    parameter
}

fn call_expression_at<'a>(syntax: &'a Program, open: Span, callee: &[String]) -> Option<&'a Expr> {
    let mut collector = CallCollector { calls: Vec::new() };
    collector.visit_program(syntax);
    collector
        .calls
        .into_iter()
        .filter(|expression| {
            expression.span.start <= open.start
                && open.end <= expression.span.end
                && matches!(&expression.kind, ExprKind::Call { callee: path, .. } if path == callee)
        })
        .min_by_key(|expression| expression.span.end - expression.span.start)
}

struct CallCollector<'ast> {
    calls: Vec<&'ast Expr>,
}

impl<'ast> Visitor<'ast> for CallCollector<'ast> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression.kind, ExprKind::Call { .. }) {
            self.calls.push(expression);
        }
        visit::walk_expr(self, expression);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_uses_resolved_catalog_signature_effects_and_examples() {
        let source = r#"
state "game.exe" {}

whileAttached {
    let value: i32 = 8
    let bounded = value.clamp(0, 7)
}
"#;
        let offset = source.find("clamp").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("catalog hover");
        assert!(hover.markdown.contains("i32.clamp"));
        assert!(hover.markdown.contains("T = i32"));
        assert!(hover.markdown.contains("**Parameters**"));
        assert!(hover.markdown.contains("**Effects:** pure"));
        assert!(!hover.markdown.contains("**Effects:** pure."));
        assert!(
            !hover
                .markdown
                .contains("**Runtime behavior:** available everywhere; synchronous.")
        );
        assert!(hover.markdown.contains("**Examples**"));
        assert!(
            hover
                .markdown
                .contains("let visibleStage = 9u32.clamp(1, 7)")
        );
        assert!(!hover.markdown.contains("value.min"));
        assert!(!hover.markdown.contains("value.max"));
        assert!(!hover.markdown.contains("setTickRate"));
    }

    #[test]
    fn hover_reflows_wrapped_doc_comments_within_one_paragraph() {
        let source = r#"
state "game.exe" {}
onDetach {
    timer.pauseGameTime()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("pauseGameTime").unwrap() + 2)
            .unwrap()
            .expect("timer hover");
        assert!(hover.markdown.contains(
            "This explicit operation is intended for lifecycle transitions such as ensuring game time does not advance after the attached process closes."
        ));
        assert!(!hover.markdown.contains("explicit\n\noperation"));
        assert!(!hover.markdown.contains("game\n\ntime"));
    }

    #[test]
    fn string_ascii_case_conversion_hover_comes_from_the_catalog() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let map = "MAP_A".toAsciiLowerCase()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("toAsciiLowerCase").unwrap() + 2)
            .unwrap()
            .expect("string conversion hover");
        assert!(
            hover
                .markdown
                .contains("String.toAsciiLowerCase() -> String")
        );
        assert!(hover.markdown.contains("Only `A` through `Z` are changed"));
        assert!(hover.markdown.contains("**Effects:** allocates"));
        assert!(
            hover
                .markdown
                .contains("let map = \"FOREST\".toAsciiLowerCase()")
        );
    }

    #[test]
    fn hover_uses_derived_effects_for_source_defined_catalog_functions() {
        let source = r#"
state "game.exe" {}
fn resolve(module: Module) -> address! {
    return module.readRelative32(0)
}
whileAttached {
    let running = timer.isRunning()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let relative = database
            .hover(source.find("readRelative32").unwrap() + 2)
            .unwrap()
            .expect("source-defined method hover");
        assert!(
            relative
                .markdown
                .contains("**Effects:** reads process memory, requires an attached process")
        );
        assert!(relative.markdown.contains(
            "**Runtime behavior:** available everywhere; synchronous; requires an attached process"
        ));

        let timer = database
            .hover(source.find("isRunning").unwrap() + 2)
            .unwrap()
            .expect("source-defined function hover");
        assert!(timer.markdown.contains("**Effects:** reads timer state"));
        assert!(
            timer
                .markdown
                .contains("**Runtime behavior:** available everywhere; synchronous")
        );
    }

    #[test]
    fn hover_shows_transitive_state_snapshot_requirements() {
        let source = r#"
state "game.exe" { level: u32 at 0x100 }

fn changed() {
    return old.level != current.level
}

fn relay() {
    return changed()
}

split {
    return relay()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.rfind("relay").unwrap() + 2)
            .unwrap()
            .expect("relay hover");
        assert!(
            hover
                .markdown
                .contains("**Effects:** requires state snapshots")
        );
        assert!(hover.markdown.contains(
            "**Runtime behavior:** synchronous; requires committed `old` and `current` state snapshots"
        ));
    }

    #[test]
    fn namespace_hover_uses_catalog_documentation() {
        let source = r#"
state "game.exe" {}

whileAttached {
    let running = timer.isRunning()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("timer").unwrap() + 2)
            .unwrap()
            .expect("namespace hover");
        let start = source.find("timer").unwrap();
        assert_eq!(
            hover.span,
            Span {
                start,
                end: start + 5
            }
        );
        assert!(hover.markdown.contains("```splitscript\ntimer\n```"));
        assert!(hover.markdown.contains("Reads information from the timer"));
        assert!(
            hover
                .markdown
                .contains("let currentTimerState = timer.state()")
        );
    }

    #[test]
    fn language_hover_is_served_from_the_language_catalog() {
        let source = "state \"game.exe\" {}\nwhileAttached { let version = v\"1.2.3.4\"; retry process.read<i32>(0) }";
        let offset = source.find("retry").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("language hover");
        assert!(
            hover
                .markdown
                .contains("let value = retry fallibleExpression")
        );
        assert!(hover.markdown.contains("Retries synchronous fallible work"));
        assert!(hover.markdown.contains("let health = retry"));
        assert!(!hover.markdown.contains("fn readMarker"));

        let version = database
            .hover(source.find("v\"").unwrap())
            .unwrap()
            .expect("version-literal hover");
        assert!(version.markdown.contains("v\"major.minor.build.private\""));
        assert!(
            version
                .markdown
                .contains("exactly four decimal `u16` components")
        );
    }

    #[test]
    fn tick_rate_policy_and_fields_share_lifecycle_documentation() {
        let source = "state \"game.exe\" {}\ntickRate { attached: 60, detached: 2, }";
        let mut database = CompilerDatabase::new(source);
        for spelling in ["tickRate", "attached", "detached"] {
            let hover = database
                .hover(source.find(spelling).unwrap() + 1)
                .unwrap()
                .expect("tick-rate hover");
            assert!(
                hover
                    .markdown
                    .contains("tickRate { attached: 60, detached: 2 }")
            );
            assert!(hover.markdown.contains("defaults to 120 Hz"));
        }
    }

    #[test]
    fn floating_point_literal_hover_shows_the_resolved_width_and_exact_bits() {
        let source = r#"state "game.exe" {}
whileAttached {
    let smallest32: f32 = 1e-45
    let smallest64: f64 = 5e-324
    let ordinary = 1.25
}"#;
        let mut database = CompilerDatabase::new(source);

        let single_offset = source.find("1e-45").unwrap();
        let single = database.hover(single_offset).unwrap().expect("f32 hover");
        assert_eq!(
            single.span,
            Span {
                start: single_offset,
                end: single_offset + 5,
            }
        );
        assert!(single.markdown.contains("1e-45: f32"));
        assert!(single.markdown.contains("`0x00000001`"));

        let double_offset = source.find("5e-324").unwrap();
        let double = database.hover(double_offset).unwrap().expect("f64 hover");
        assert!(double.markdown.contains("5e-324: f64"));
        assert!(double.markdown.contains("`0x0000000000000001`"));

        let ordinary_offset = source.find("1.25").unwrap();
        let ordinary = database
            .hover(ordinary_offset)
            .unwrap()
            .expect("default f64 hover");
        assert!(ordinary.markdown.contains("1.25: f64"));
        assert!(ordinary.markdown.contains("`0x3ff4000000000000`"));
    }

    #[test]
    fn expression_hover_shows_inferred_types_for_operators_literals_and_calls() {
        let source = r#"state "game.exe" {}
fn add(left: u64, right: u64) -> u64 { return left + right }
whileAttached {
    let value: u64 = (1 + 2) * 3
    let message = "ready"
    let enabled = true
    let sum = add(4, 5)
}"#;
        let mut database = CompilerDatabase::new(source);

        let plus = source.find("1 + 2").unwrap() + 2;
        let addition = database.hover(plus).unwrap().expect("addition hover");
        assert_eq!(&source[addition.span.start..addition.span.end], "(1 + 2)");
        assert_eq!(addition.markdown, "```splitscript\nu64\n```");

        let integer = source.find("1 + 2").unwrap();
        let integer = database
            .hover(integer)
            .unwrap()
            .expect("integer literal hover");
        assert_eq!(integer.markdown, "```splitscript\nu64\n```");

        for (spelling, expected) in [("\"ready\"", "String"), ("true", "bool")] {
            let hover = database
                .hover(source.find(spelling).unwrap() + 1)
                .unwrap()
                .unwrap_or_else(|| panic!("{spelling} literal hover"));
            assert_eq!(hover.markdown, format!("```splitscript\n{expected}\n```"));
        }

        let call = source.find("add(4, 5)").unwrap();
        let closing_parenthesis = call + "add(4, 5)".len() - 1;
        let call = database
            .hover(closing_parenthesis)
            .unwrap()
            .expect("call expression hover");
        assert_eq!(&source[call.span.start..call.span.end], "add(4, 5)");
        assert_eq!(call.markdown, "```splitscript\nu64\n```");
    }

    #[test]
    fn native_process_hover_comes_from_the_typed_provider_declaration() {
        let source = "state \"game.exe\" {}\nwhileAttached { let attached = process }";
        let offset = source.rfind("process").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process provider hover");
        assert!(hover.markdown.contains("state \"game.exe\" { ... }"));
        assert!(hover.markdown.contains("process: Process"));
        assert!(hover.markdown.contains("has type `Process`"));
    }

    #[test]
    fn matched_process_name_hover_comes_from_the_standard_library() {
        let source =
            "state [\"game.exe\", \"demo.exe\"] {}\nwhileAttached { print(process.name()) }";
        let offset = source.find("name()").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("process.name hover");
        assert!(hover.markdown.contains("Process.name() -> String"));
        assert!(
            hover
                .markdown
                .contains("configured process name that matched during attachment")
        );
        assert!(hover.markdown.contains("requires an attached process"));
    }

    #[test]
    fn main_module_hover_explains_matched_process_identity() {
        let source = "state [\"game.exe\", \"demo.exe\"] {}\nonAttach { let executable = await process.mainModule() }";
        let offset = source.find("mainModule").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process.mainModule hover");
        assert!(
            hover
                .markdown
                .contains("Process.mainModule() -> async Module")
        );
        assert!(hover.markdown.contains("main executable module"));
        assert!(hover.markdown.contains("available in onAttach; suspends"));
    }

    #[test]
    fn process_closed_hover_explains_inert_attachment_waiting() {
        let source = "state \"game.exe\" {}\nonAttach { await process.closed() }";
        let offset = source.find("closed").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process.closed hover");
        assert!(hover.markdown.contains("Process.closed() -> async Never"));
        assert!(
            hover
                .markdown
                .contains("never closes or detaches the process itself")
        );
        assert!(hover.markdown.contains(
            "available in onAttach; suspends; requires an attached process; cancels when the process closes"
        ));
    }

    #[test]
    fn generated_state_layout_type_and_value_have_source_hover() {
        let source = r#"
state "game.exe" {
    /// Steam build layout.
    layout Steam { level: u32 at 0x100 },
    layout GOG { level: u32 at 0x200 }
}
onAttach { return StateLayout.Steam }
split { return layout == StateLayout.Steam }
"#;
        let mut database = CompilerDatabase::new(source);
        let layout_type = database
            .hover(source.find("StateLayout").unwrap() + 1)
            .unwrap()
            .expect("generated layout type hover");
        assert!(layout_type.markdown.contains("enum StateLayout"));
        assert!(
            layout_type
                .markdown
                .contains("memory layout selected for the attached game build")
        );

        let layout = database
            .hover(source.find("layout ==").unwrap() + 1)
            .unwrap()
            .expect("selected layout hover");
        assert!(layout.markdown.contains("layout: StateLayout"));
        assert!(layout.markdown.contains("Read-only memory layout"));

        let variant = database
            .hover(source.rfind("Steam").unwrap() + 1)
            .unwrap()
            .expect("generated layout variant hover");
        assert!(variant.markdown.contains("StateLayout.Steam"));
        assert!(variant.markdown.contains("Steam build layout."));
    }

    #[test]
    fn settings_and_lifecycle_hover_share_the_language_catalog() {
        let source = include_str!("../examples/lso_desktop_settings.split");
        let mut database = CompilerDatabase::new(source);
        for (needle, expected) in [
            ("choice", "Declares an enum-backed setting choice"),
            ("onAttach", "Initializes one attached process"),
        ] {
            let offset = source.find(needle).unwrap() + 1;
            let hover = database.hover(offset).unwrap().expect("language hover");
            assert!(
                hover.markdown.contains(expected),
                "missing catalog documentation for {needle}"
            );
            if needle == "onAttach" {
                assert!(
                    hover
                        .markdown
                        .contains("let module = await process.module(\"GameAssembly.dll\")")
                );
                assert!(!hover.markdown.contains("onDetach {}"));
            }
        }
    }

    #[test]
    fn detach_lifecycle_hover_describes_the_exact_event() {
        let source = "state \"game.exe\" {}\nonDetach { setTickRate(1) }";
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("onDetach").unwrap() + 1)
            .unwrap()
            .expect("onDetach hover");
        assert!(
            hover
                .markdown
                .contains("once when an attached process closes")
        );
        assert!(
            hover
                .markdown
                .contains("never runs for the initial detached state")
        );
        assert!(
            hover
                .markdown
                .contains("use `setup` for one-time script initialization")
        );
    }

    #[test]
    fn source_hover_renders_inferred_value_and_field_types() {
        let source = r#"
record Point { x: i32 }
let global = 1
state "game.exe" {
    point: Point = process.read(0);
    inventory: [u8; 6] at 0x100;
}
settings {
    "General" {
        /// Controls the optional behavior.
        "Enabled" => enabled: true
    }
}
fn inspect(point) {
    let local = point.x + global
    if settings.enabled { print(local as String) }
}
whileAttached {
    let timerState = timer.state()
    let inventory = current.inventory
    for item in inventory {
        print(item as String)
    }
    inspect(current.point)
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, signature, description) in [
            (
                source.find("point) {").unwrap(),
                "point: Point",
                "Parameter",
            ),
            (
                source.find("point.x").unwrap() + "point.".len(),
                "Point.x: i32",
                "Record field",
            ),
            (
                source.find("local as").unwrap(),
                "let local: i32",
                "Local variable",
            ),
            (
                source.find("timerState =").unwrap(),
                "let timerState: TimerState",
                "Local variable",
            ),
            (
                source.find("inventory =").unwrap(),
                "let inventory: [u8; 6]",
                "Local variable",
            ),
            (
                source.find("item in inventory").unwrap(),
                "item: u8",
                "Read-only loop binding",
            ),
            (
                source.find("+ global").unwrap() + 2,
                "let global: i32",
                "Global variable",
            ),
            (
                source.find("current.point").unwrap() + "current.".len(),
                "current.point: Point",
                "Transactional state field",
            ),
            (
                source.find("current.inventory").unwrap() + "current.".len(),
                "current.inventory: [u8; 6]",
                "Transactional state field",
            ),
            (
                source.find("settings.enabled").unwrap() + "settings.".len(),
                "settings.enabled: bool",
                "Controls the optional behavior.",
            ),
            (
                source.rfind("inspect").unwrap(),
                "fn inspect(point: Point) -> None",
                "Function",
            ),
        ] {
            let hover = database.hover(offset).unwrap().expect("source hover");
            assert!(
                hover.markdown.contains(signature),
                "missing `{signature}` in {}",
                hover.markdown
            );
            assert!(
                hover.markdown.contains(description),
                "missing `{description}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn source_hover_includes_user_documentation() {
        let source = r#"
/// A point in game memory.
record Point {
    /// Horizontal position.
    x: i32
}
/// The current run mode.
enum Mode {
    /// Normal gameplay.
    Active
}
/// Accumulated collectible count.
let total = 0
state "game.exe" {
    /// Latest player position.
    point: Point at 0x100
}
/// Inspects the current state.
///
/// This is safe to call every tick.
fn inspect(point: Point, mode: Mode) {
    if mode == Mode.Active {
        print(point.x as String)
    }
}
whileAttached {
    inspect(current.point, Mode.Active)
    print(total as String)
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (needle, expected) in [
            ("record Point", "A point in game memory."),
            ("point.x", "Horizontal position."),
            ("enum Mode", "The current run mode."),
            ("Mode.Active", "Normal gameplay."),
            ("total as", "Accumulated collectible count."),
            ("current.point", "Latest player position."),
            (
                "inspect(current",
                "Inspects the current state.\n\nThis is safe to call every tick.",
            ),
        ] {
            let offset = source.find(needle).unwrap()
                + match needle {
                    "record Point" => "record ".len(),
                    "enum Mode" => "enum ".len(),
                    "point.x" => "point.".len(),
                    "Mode.Active" => "Mode.".len(),
                    "current.point" => "current.".len(),
                    _ => 0,
                };
            let hover = database.hover(offset).unwrap().expect("source hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn first_class_state_snapshot_hover_preserves_type_and_field_documentation() {
        let source = r#"
state "game.exe" {
    /// Current level number.
    level: u32 at 0x100
}
fn levelOf(snapshot) {
    return snapshot.level
}
whileAttached {
    let captured = current
    print(levelOf(captured))
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, expected) in [
            (source.find("snapshot)").unwrap(), "snapshot: StateSnapshot"),
            (
                source.find("captured =").unwrap(),
                "let captured: StateSnapshot",
            ),
            (
                source.find("snapshot.level").unwrap() + "snapshot.".len(),
                "Current level number.",
            ),
        ] {
            let hover = database.hover(offset).unwrap().expect("snapshot hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn state_filter_bindings_have_hover_documentation() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 if value == 7 { Err("transient") } else { value };
        }"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("value ==").unwrap();
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("state filter hover for value");
        assert!(
            hover.markdown.contains("value: i32"),
            "missing value type in {}",
            hover.markdown
        );
    }

    #[test]
    fn first_class_settings_view_hover_preserves_type_and_setting_documentation() {
        let source = r#"
settings {
    /// Enables automatic splitting.
    "Enabled" => enabled: true
}
state "game.exe" {}
fn isEnabled(view) {
    return view.enabled
}
whileAttached {
    let captured = settings
    print(isEnabled(captured))
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, expected) in [
            (source.find("view)").unwrap(), "view: SettingsView"),
            (
                source.find("captured =").unwrap(),
                "let captured: SettingsView",
            ),
            (
                source.find("view.enabled").unwrap() + "view.".len(),
                "Enables automatic splitting.",
            ),
        ] {
            let hover = database
                .hover(offset)
                .unwrap()
                .expect("settings view hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn settings_key_lookup_hover_comes_from_the_standard_library() {
        let source = r#"
settings {
    "Boss" => boss key "split-boss": true
}
state "game.exe" {}
whileAttached {
    let shouldSplit = settings.enabled("split-boss")
}
"#;
        let offset = source.find("enabled(").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("method hover");
        assert!(
            hover
                .markdown
                .contains("SettingsView.enabled(key: String) -> bool")
        );
        assert!(hover.markdown.contains("stable host-map key"));
        assert!(hover.markdown.contains("**Effects:** pure"));
    }

    #[test]
    fn integer_radix_hover_comes_from_the_capability_catalog() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let hexadecimal = 255u8.toString(16) else ""
    print(hexadecimal)
}
"#;
        let offset = source.find("toString(").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("method hover");
        assert!(hover.markdown.contains("toString(radix: u32) -> String!"));
        assert!(hover.markdown.contains("Radices from 2 through 36"));
        assert!(hover.markdown.contains("**Effects:** allocates"));
    }

    #[test]
    fn source_function_hover_renders_propagated_effects_after_semantic_validation_errors() {
        let source = r#"
fn readValue() -> f32! {
    return process.read<f32>(0)
}
fn bar() -> f32! {
    return readValue()
}
state "game.exe" {}
onDetach {
    let value = bar()
}
"#;
        let offset = source.rfind("bar()").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("source function hover should survive the detached-call diagnostic");
        assert!(hover.markdown.contains("fn bar() -> f32!"));
        assert!(
            hover
                .markdown
                .contains("**Effects:** reads process memory, requires an attached process")
        );
        assert!(hover.markdown.contains(
            "**Runtime behavior:** synchronous; requires an attached process and is unavailable in `onDetach`"
        ));
    }

    #[test]
    fn source_function_hover_does_not_punctuate_metadata_fragments() {
        let source = r#"
fn identity(value: i32) -> i32 {
    return value
}
state "game.exe" {}
whileAttached {
    let value = identity(1)
}
"#;
        let offset = source.rfind("identity").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("function hover");
        assert!(
            hover
                .markdown
                .contains("\n\nFunction\n\n**Effects:** pure\n\n**Runtime behavior:** synchronous")
        );
        assert!(!hover.markdown.contains("Function."));
        assert!(!hover.markdown.contains("pure."));
        assert!(!hover.markdown.contains("synchronous."));
    }

    #[test]
    fn inferred_async_function_hover_includes_async_in_the_result_type() {
        let source = r#"
state "game.exe" {}
fn loadModule() {
    let module = await process.module("game.dll")
    return module
}
onAttach {
    let module = await loadModule()
    print(module.address)
}
"#;
        let offset = source.rfind("loadModule").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("function hover");
        assert!(
            hover.markdown.contains("fn loadModule() -> async Module"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("**Runtime behavior:** suspends"));
    }

    #[test]
    fn stored_future_hover_preserves_the_async_value_type() {
        let source = r#"
state "game.exe" {}
onAttach {
    let pending = process.mainModule()
    print("created")
    let module = await pending
    print(module.address)
}
"#;
        let offset = source.rfind("pending").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("future hover");
        assert!(
            hover.markdown.contains("let pending: async Module"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("Local variable"));
    }

    #[test]
    fn bottom_future_hover_preserves_the_never_completion_type() {
        let source = r#"
state "game.exe" {}
onAttach {
    let pending = process.closed()
    await pending
}
"#;
        let offset = source.rfind("pending").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("future hover");
        assert!(
            hover.markdown.contains("let pending: async Never"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn inferred_generic_function_hover_shows_parameters_and_capability_bounds() {
        let source = r#"
fn smaller(left, right) {
    return left.min(right)
}
state "game.exe" {}
whileAttached {
    let value: u32 = smaller(3u32, 7u32)
}
"#;
        let offset = source.rfind("smaller").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("generic function hover");
        assert!(
            hover
                .markdown
                .contains("fn smaller(left: T, right: T) -> T where T: Numeric"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn string_conversion_and_interpolation_share_the_display_capability() {
        let source = r#"
fn twoDigits(value) {
    if value == 0 {
        return "00"
    }
    if value < 10 {
        return `0{value}`
    }
    return value as String
}
state "game.exe" {}
whileAttached {
    let value = twoDigits(7u32)
}
"#;
        let offset = source.rfind("twoDigits").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("generic function hover");
        assert_eq!(
            hover.markdown.lines().nth(1),
            Some("fn twoDigits(value: T) -> String where T: Numeric + Display"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn integer_hierarchy_removes_redundant_lunistice_level_text_bounds() {
        let source = include_str!("../examples/lunistice.split");
        let offset = source.find("levelText").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("levelText function hover");
        assert_eq!(
            hover.markdown.lines().nth(1),
            Some("fn levelText(level: T) -> String where T: Integer"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn signature_help_tracks_nested_arguments_and_inferred_types() {
        let source = r#"
state "game.exe" {}

whileAttached {
    let value: i32 = 8
    let bounded = value.clamp((0), 7)
}
"#;
        let offset = source.find(", 7").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("signature help");
        assert_eq!(help.active_parameter, 1);
        assert!(help.signatures[0].label.contains("i32.clamp"));
        assert_eq!(help.signatures[0].parameters[1].label, "maximum");
    }

    #[test]
    fn signature_help_probes_an_unfinished_method_call() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let value: i32 = 8\n",
            "    let bounded = value.clamp(0, \n",
            "}\n"
        );
        let offset = source.find("clamp(0, ").unwrap() + "clamp(0, ".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("incomplete calls should retain signature help");
        assert_eq!(help.active_parameter, 1);
        assert!(help.signatures[0].label.starts_with("i32.clamp"));
    }

    #[test]
    fn signature_help_resolves_explicit_types_on_captured_process_values() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let attached = process\n",
            "    let value = attached.read<u32>(\n",
            "}\n"
        );
        let offset = source.find("read<u32>(").unwrap() + "read<u32>(".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("explicit generic method signature help");
        assert!(help.signatures[0].label.starts_with("Process.read"));
        assert!(help.signatures[0].label.contains("-> u32!"));
    }

    #[test]
    fn signature_help_resolves_explicit_types_on_expression_receivers() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "fn attachedProcess() { return process }\n",
            "whileAttached {\n",
            "    let value = attachedProcess().read<u32>(\n",
            "}\n"
        );
        let offset = source.find("read<u32>(").unwrap() + "read<u32>(".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("expression receiver should retain generic method signature help");
        assert!(help.signatures[0].label.starts_with("Process.read"));
        assert!(help.signatures[0].label.contains("-> u32!"));
    }

    #[test]
    fn hover_recovers_from_untyped_state_field_member_access() {
        let source = r#"state GBA {
    pos at 0x100,
}

fn bar() {
    return current.pos.x > old.pos.y
}
"#;
        let mut database = CompilerDatabase::new(source);
        for needle in ["GBA", "current", "old"] {
            let offset = source.find(needle).unwrap() + needle.len() - 1;
            assert!(
                database.hover(offset).unwrap().is_some(),
                "{needle} should retain hover despite the recoverable separator error"
            );
        }
        assert!(
            database
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message == "expected `;` between state fields")
        );
    }

    #[test]
    fn hover_preserves_independent_semantics_across_parser_errors() {
        let source = r#"
fn retained(value: i32) -> i32 {
    return value
}

state GBA {}

split {
    let broken = 0b102
    return retained(1) == 1
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            database
                .recovering_parse()
                .unwrap()
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic
                    .message
                    .contains("not valid in a binary integer literal"))
        );

        let hover = database
            .hover(source.rfind("retained").unwrap() + 1)
            .unwrap()
            .expect("an unrelated parser error must not disable semantic hover");
        assert!(hover.markdown.contains("fn retained(value: i32) -> i32"));
    }
}
