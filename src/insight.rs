//! Semantic hover and signature information for editor clients.

use crate::{
    ast::{EnumDecl, Expr, ExprKind, Program, Span},
    database::{
        CompilerDatabase, DefinitionTarget, SemanticQueryResult, SourceDefinition,
        SourceDefinitionId,
    },
    documentation::StandardLibraryDocumentation,
    effects::{FunctionOperationSemantics, OperationAnalysis},
    language::{LanguageCatalog, LanguageItem},
    lexer::{Token, TokenKind},
    semantic::{ResolvedCall, SemanticModel},
    stdlib::{StandardLibrary, StdlibItem, StdlibItemId, TypeRef as CatalogTypeRef},
    syntax::SourceDocument,
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
                item,
                substitutions(item, &type_arguments, semantic.as_ref()),
            )
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
    let recovered = database.recovering_parse()?;
    let document = recovered.source_document().clone();
    let syntax = recovered.syntax().clone();
    let Some(call_site) = active_call(&document, offset) else {
        return Ok(None);
    };
    let mut semantic = semantic_context(database);
    let resolved = semantic.as_ref().and_then(|context| {
        call_expression_at(&syntax, call_site.open, &call_site.callee)
            .and_then(|expression| context.semantics.call(expression.id))
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
        StandardLibrary::new()
            .resolve_path(&call_site.callee)
            .map(|candidate| (candidate.item.id, Vec::new()))
    });
    if selected.is_none()
        && let Some((item, type_arguments, probe_semantic)) =
            infer_method_call(document.source(), &call_site)
    {
        selected = Some((item, type_arguments));
        semantic = Some(probe_semantic);
    }
    let Some((item, type_arguments)) = selected else {
        return Ok(None);
    };
    let substitutions = substitutions(item, &type_arguments, semantic.as_ref());
    let documentation = StandardLibraryDocumentation::generate(item, &substitutions);
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
    syntax: Program,
    semantics: SemanticModel,
    enum_types: Vec<EnumDecl>,
    effects: Option<OperationAnalysis>,
}

fn semantic_context(database: &mut CompilerDatabase) -> Option<SemanticContext> {
    match database.check() {
        Ok(checked) => Some(SemanticContext {
            syntax: checked.syntax().clone(),
            semantics: checked.semantics().clone(),
            enum_types: checked.enum_types().to_vec(),
            effects: Some(checked.effects().clone()),
        }),
        Err(_) => database
            .recovering_check()
            .ok()
            .map(|checked| SemanticContext {
                syntax: checked.syntax().clone(),
                semantics: checked.semantics().clone(),
                enum_types: checked.enum_types().to_vec(),
                effects: checked.effects().cloned(),
            }),
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
    StandardLibrary::new()
        .item(item)
        .signature
        .type_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, ty)| (parameter.name, render_type(*ty, context)))
        .collect()
}

fn render_type(ty: TypeId, context: &SemanticContext) -> String {
    let types = context.semantics.types();
    match types.kind(ty) {
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Record(id) => context
            .syntax
            .records
            .iter()
            .find(|record| record.id == *id)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| format!("record#{}", id.index())),
        TypeKind::Enum(id) => context
            .enum_types
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{}", id.index())),
        TypeKind::Array { element, .. } => {
            format!("Array<{}>", render_type(*element, context))
        }
        TypeKind::Option { value, .. } => format!("{}?", render_type(*value, context)),
        TypeKind::Result { value, .. } => format!("{}!", render_type(*value, context)),
    }
}

fn render_source_hover(definition: &SourceDefinition, context: &SemanticContext) -> Option<String> {
    let syntax = &context.syntax;
    let semantics = &context.semantics;
    match definition.id {
        SourceDefinitionId::Value(value) => {
            let ty = semantics.value_type(value)?;
            let ty = render_type(ty, context);
            let (signature, description) = if syntax.globals.iter().any(|global| global.id == value)
            {
                (
                    format!("let {}: {ty}", definition.name),
                    "Global variable".to_owned(),
                )
            } else if syntax
                .state
                .as_ref()
                .is_some_and(|state| state.fields.iter().any(|field| field.id == value))
            {
                (
                    format!("current.{}: {ty}", definition.name),
                    "Transactional state field".to_owned(),
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
            Some(source_markdown(
                &format!("{}.{}: {ty}", record.name, definition.name),
                "Record field",
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
            let name = receiver.map_or_else(
                || function.name.clone(),
                |receiver| format!("{receiver}.{}", function.name),
            );
            let description = source_function_description(
                function.method_of.is_some(),
                function.debug_only,
                context
                    .effects
                    .as_ref()
                    .map(|effects| effects.function(function.id)),
            );
            Some(source_markdown(
                &format!(
                    "fn {name}({}) -> {}",
                    parameters.join(", "),
                    render_type(result, context)
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
                "Record type",
            ))
        }
        SourceDefinitionId::Enum(enumeration) => {
            let enumeration = syntax
                .enums
                .iter()
                .find(|candidate| candidate.id == enumeration)?;
            Some(source_markdown(
                &format!("enum {}", enumeration.name),
                "Enum type",
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
            Some(source_markdown(
                &format!(
                    "{}.{}{}",
                    enumeration.name,
                    definition.name,
                    payload.as_deref().unwrap_or_default()
                ),
                "Enum variant",
            ))
        }
    }
}

fn source_function_description(
    is_method: bool,
    debug_only: bool,
    operation: Option<FunctionOperationSemantics>,
) -> String {
    let mut description = if is_method { "Method" } else { "Function" }.to_owned();
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
        description.push_str("\n\n**Runtime behavior:** synchronous");
        if operation.requires_attached_process {
            description
                .push_str("; requires an attached process and is unavailable in `onDetached`");
        }
    }
    description
}

fn source_markdown(signature: &str, description: &str) -> String {
    format!("```splitscript\n{signature}\n```\n\n{description}")
}

fn render_stdlib_hover(item: StdlibItemId, substitutions: Vec<(&str, String)>) -> String {
    StandardLibraryDocumentation::generate(item, &substitutions).hover_markdown()
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
) -> Option<(StdlibItemId, Vec<TypeId>, SemanticContext)> {
    let method_dot = call.method_dot?;
    let line_end = source[call.open.start..]
        .find('\n')
        .map_or(source.len(), |relative| call.open.start + relative);
    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..method_dot]);
    probe_source.push_str(&source[line_end..]);
    let mut probe = CompilerDatabase::new(probe_source);
    let analysis = probe.analysis_at(method_dot.checked_sub(1)?).ok()??;
    let method_name = call.callee.last()?;
    let item = StandardLibrary::new()
        .methods_for_type(&analysis.type_kind)
        .into_iter()
        .find(|item| {
            matches!(item.kind, crate::stdlib::ItemKind::Method { name, .. } if name == method_name)
        })?;
    let type_arguments = inferred_method_type_arguments(item, analysis.ty, &analysis.type_kind);
    let semantic = semantic_context(&mut probe)?;
    Some((item.id, type_arguments, semantic))
}

fn inferred_method_type_arguments(
    item: &StdlibItem,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Vec<TypeId> {
    let crate::stdlib::ItemKind::Method {
        receiver: declared, ..
    } = item.kind
    else {
        return Vec::new();
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
        (CatalogTypeRef::Variable(name), _) if name == parameter => Some(receiver),
        (
            CatalogTypeRef::Array(element),
            TypeKind::Array {
                element: actual, ..
            },
        ) if matches!(*element, CatalogTypeRef::Variable(name) if name == parameter) => {
            Some(*actual)
        }
        (CatalogTypeRef::Result(value), TypeKind::Result { value: actual, .. }) if matches!(*value, CatalogTypeRef::Variable(name) if name == parameter) => {
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
state "game.exe" { point: Point = process.read(0) }
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
}
