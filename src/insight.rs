//! Semantic hover and signature information for editor clients.

use std::sync::Arc;

use crate::{
    ast::{Expr, ExprKind, Program, Span},
    database::{
        CompilerDatabase, DefinitionTarget, SemanticQueryResult, SemanticSnapshot,
        SourceDefinition, SourceDefinitionId,
    },
    documentation::StandardLibraryDocumentation,
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
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub span: Span,
    pub markdown: String,
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
    let standard_library = database.context().standard_library();
    let Some(token) = database.token_at(offset)? else {
        return Ok(None);
    };
    let Some(target) = database.definition_at(offset)? else {
        return Ok(None);
    };
    let markdown = match target {
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
            render_stdlib_hover(
                standard_library,
                item,
                substitutions(item, &type_arguments, semantic.as_ref()),
            )
        }
        DefinitionTarget::StandardLibrarySymbol(symbol) => {
            render_stdlib_symbol_hover(standard_library, symbol)
        }
        DefinitionTarget::Language(item) => {
            render_language_hover(LanguageCatalog::new().item(item))
        }
        DefinitionTarget::Source(definition) => {
            let Some(context) = semantic_context(database) else {
                return Ok(None);
            };
            let Some(markdown) = render_source_hover(&definition, &context) else {
                return Ok(None);
            };
            markdown
        }
    };
    Ok(Some(HoverInfo {
        span: token.span,
        markdown,
    }))
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
            "Read-only transactional state values. `current` contains the latest committed state and `old` contains the preceding committed state.",
        )),
        SourceDefinitionId::Settings => Some(source_markdown(
            "settings / oldSettings: settings snapshot",
            "Read-only settings values. `settings` contains the latest host settings and `oldSettings` contains their values from the preceding update.",
        )),
        SourceDefinitionId::Value(value) => {
            let ty = semantics.value_type(value)?;
            let ty = render_type(ty, context);
            let (signature, description) = if let Some(global) =
                syntax.globals.iter().find(|global| global.id == value)
            {
                (
                    format!("let {}: {ty}", definition.name),
                    documented_description("Global variable", global.documentation.as_deref()),
                )
            } else if let Some(field) = syntax
                .state
                .as_ref()
                .and_then(|state| state.fields.iter().find(|field| field.id == value))
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
                    let constraints = semantics.generic_parameter_constraints(*parameter);
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
                .enums
                .iter()
                .find(|candidate| candidate.id == enumeration)?;
            Some(source_markdown(
                &format!("enum {}", enumeration.name),
                &documented_description("Enum type", enumeration.documentation.as_deref()),
            ))
        }
        SourceDefinitionId::EnumVariant(variant) => {
            let enumeration = syntax.enums.iter().find(|enumeration| {
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
            description
                .push_str("; requires an attached process and is unavailable in `onDetached`");
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
            (declaration.name.to_owned(), &declaration.documentation)
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
                    library.render_declared_type(declaration.ty)
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
        documentation.summary, documentation.details
    );
    append_examples(&mut markdown, documentation.examples);
    markdown
}

fn render_language_hover(item: &LanguageItem) -> String {
    let mut markdown = format!(
        "```splitscript\n{}\n```\n\n{}\n\n{}",
        item.form, item.documentation.summary, item.documentation.details
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

struct CallSite {
    open: Span,
    callee: Vec<String>,
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
    let (callee, method_dot) = callee_before(&tokens, open_index)?;
    let active_parameter = active_parameter(&tokens[open_index + 1..]);
    Some(CallSite {
        open: tokens[open_index].span,
        callee,
        method_dot,
        active_parameter,
    })
}

fn callee_before(tokens: &[&Token], open: usize) -> Option<(Vec<String>, Option<usize>)> {
    let method_dot = open
        .checked_sub(2)
        .filter(|index| tokens[*index].kind == TokenKind::Dot)
        .map(|index| tokens[index].span.start);
    let mut cursor = open.checked_sub(1)?;
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
        (reversed, method_dot)
    })
}

fn infer_method_call(
    source: &str,
    call: &CallSite,
    compiler_context: &crate::CompilerContext,
) -> Option<(StdlibItemId, Vec<TypeId>, SemanticContext)> {
    let selector_candidate = if call.callee.len() >= 3 {
        let selector = call.callee.last()?;
        let method = &call.callee[call.callee.len() - 2];
        compiler_context
            .standard_library()
            .method_candidates_with_selector(method, Some(selector))
            .into_iter()
            .next()
            .map(|candidate| (method.as_str(), candidate))
    } else {
        None
    };
    let selector_dot = call.method_dot?;
    let method_dot = if selector_candidate.is_some() {
        source[..selector_dot].rfind('.')?
    } else {
        selector_dot
    };
    let line_end = source[call.open.start..]
        .find('\n')
        .map_or(source.len(), |relative| call.open.start + relative);
    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..method_dot]);
    probe_source.push_str(&source[line_end..]);
    let mut probe = CompilerDatabase::with_context(compiler_context.clone(), probe_source);
    let analysis = probe.analysis_at(method_dot.checked_sub(1)?).ok()??;
    let semantic = semantic_context(&mut probe)?;
    let applicable = compiler_context
        .standard_library()
        .methods_for_type(&analysis.type_kind);
    let (item, type_arguments) = if let Some((method_name, candidate)) = selector_candidate {
        let item = candidate.item;
        if item.name != method_name || !applicable.iter().any(|method| method.id == item.id) {
            return None;
        }
        let type_arguments = candidate
            .type_arguments
            .iter()
            .map(|(_, ty)| semantic.semantics().types().id_for_core(*ty))
            .collect();
        (item, type_arguments)
    } else {
        let method_name = call.callee.last()?;
        let item = applicable.into_iter().find(|item| {
            matches!(
                item.kind,
                crate::stdlib::ItemKind::Method { .. }
                    | crate::stdlib::ItemKind::TypedMethod { .. }
            ) && item.name == method_name
        })?;
        let type_arguments = inferred_method_type_arguments(item, analysis.ty, &analysis.type_kind);
        (item, type_arguments)
    };
    Some((item.id, type_arguments, semantic))
}

fn inferred_method_type_arguments(
    item: &StdlibItem,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Vec<TypeId> {
    let declared = match item.kind {
        crate::stdlib::ItemKind::Method { receiver }
        | crate::stdlib::ItemKind::TypedMethod { receiver, .. } => receiver,
        crate::stdlib::ItemKind::Function | crate::stdlib::ItemKind::TypedFunction { .. } => {
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
                .contains("let visibleStage = stage.clamp(1, 7)")
        );
        assert!(!hover.markdown.contains("value.min"));
        assert!(!hover.markdown.contains("value.max"));
        assert!(!hover.markdown.contains("setTickRate"));
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
    fn language_hover_is_served_from_the_language_catalog() {
        let source = "state \"game.exe\" {}\nwhileAttached { retry process.read.i32(0) }";
        let offset = source.find("retry").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("language hover");
        assert!(
            hover
                .markdown
                .contains("let value = retry resultExpression")
        );
        assert!(hover.markdown.contains("Retries a Result expression"));
        assert!(
            hover
                .markdown
                .contains("let player = retry process.follow(module.address, [0x100, 0x20])")
        );
        assert!(!hover.markdown.contains("fn readMarker"));
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
        assert!(hover.markdown.contains("typed handle"));
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
                assert!(!hover.markdown.contains("onDetached {}"));
            }
        }
    }

    #[test]
    fn source_hover_renders_inferred_value_and_field_types() {
        let source = r#"
record Point { x: i32 }
let global = 1
state "game.exe" {
    point: Point = process.read(0)
    inventory: [u8; 6] at 0x100
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
                "fn inspect(point: Point) -> void",
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
    fn source_function_hover_renders_propagated_effects_after_semantic_validation_errors() {
        let source = r#"
fn readValue() -> f32! {
    return process.read.f32(0)
}
fn bar() -> f32! {
    return readValue()
}
state "game.exe" {}
onDetached {
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
            "**Runtime behavior:** synchronous; requires an attached process and is unavailable in `onDetached`"
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
    fn signature_help_probes_typed_methods_on_captured_process_values() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let attached = process\n",
            "    let value = attached.read.u32(\n",
            "}\n"
        );
        let offset = source.find("read.u32(").unwrap() + "read.u32(".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("typed method signature help");
        assert!(help.signatures[0].label.starts_with("Process.read"));
        assert!(help.signatures[0].label.contains("-> u32!"));
    }
}
