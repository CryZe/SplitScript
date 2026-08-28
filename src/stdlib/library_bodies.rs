//! Injection boundary for ordinary SplitScript standard-library bodies.
//!
//! Bodies are authored in the privileged catalog source, but statements and
//! expressions are parsed only after they have been appended to an ordinary
//! compilation unit. This keeps one body grammar and one semantic pipeline.

use crate::{
    Diagnostic,
    ast::{Expr, ExprKind, ManagedClassDecl, ManagedClassId, ManagedItemDecl, Program},
    lexer, parser,
    visit::{self, Visitor},
};

use super::{
    Implementation, ItemKind, ManagedRuntimeBackend, Signature, StandardLibrary, StdlibItem,
    TypeRef,
};

pub(crate) const RESERVED_FUNCTION_PREFIX: &str = "__splitscript_stdlib_";
pub(crate) const PROVIDER_PREPARATION_FUNCTION: &str =
    "__splitscript_stdlib_selected_provider_preparation";
pub(crate) const PROVIDER_BINDINGS_TYPE: &str = "__splitscript_stdlib_provider_bindings";
pub(crate) const MANAGED_POINTER_SIZE_FIELD: &str = "__pointer_size";

struct SelectedProviderContext {
    index: usize,
    ty: &'static str,
    function_name: &'static str,
}

fn body_source(
    item: &StdlibItem,
    signature: Signature,
    function_name: &str,
    body: &str,
    library: &StandardLibrary,
) -> String {
    let mut parameters = Vec::new();
    if let ItemKind::Method { receiver } = item.kind {
        parameters.push(render_parameter("self", receiver, library));
    }
    parameters.extend(
        signature
            .parameters
            .iter()
            .map(|parameter| render_parameter(parameter.name, parameter.ty, library)),
    );
    let result = if contains_type_parameter(signature.result) {
        String::new()
    } else {
        format!(
            " -> {}{}",
            if signature.result_is_async {
                "async "
            } else {
                ""
            },
            signature.result.render(library)
        )
    };
    format!(
        "fn {}({}){} {}",
        function_name,
        parameters.join(", "),
        result,
        body,
    )
}

fn render_parameter(name: &str, ty: TypeRef, library: &StandardLibrary) -> String {
    if contains_type_parameter(ty) {
        name.to_owned()
    } else {
        format!("{name}: {}", ty.render(library))
    }
}

fn contains_type_parameter(ty: TypeRef) -> bool {
    match ty {
        TypeRef::Parameter(_) | TypeRef::Associated(_) => true,
        TypeRef::Application { arguments, .. } => {
            arguments.iter().copied().any(contains_type_parameter)
        }
        TypeRef::FixedArray { element, .. } => contains_type_parameter(*element),
        TypeRef::Callable { parameters, result } => {
            parameters.iter().copied().any(contains_type_parameter)
                || contains_type_parameter(*result)
        }
        TypeRef::Core(_) | TypeRef::Standard(_) => false,
    }
}

pub(crate) fn augment_program_with_library_bodies(
    user_source: &str,
    user_program: &Program,
    library: &StandardLibrary,
) -> Result<Option<Program>, Vec<Diagnostic>> {
    let mut combined = String::new();
    let mut body_ranges = Vec::new();
    for item in library.all_items() {
        let bodies = match item.implementation {
            Implementation::Intrinsic(_) | Implementation::CapabilityRequirement => Vec::new(),
            Implementation::LibraryBody {
                function_name,
                body,
            } => vec![(item.signature, function_name, body)],
            Implementation::LibraryOverloads { cases, .. } => cases
                .iter()
                .map(|case| (case.signature, case.function_name, case.body))
                .collect(),
        };
        for (signature, function_name, body) in bodies {
            let source = body_source(item, signature, function_name, body, library);
            if combined.is_empty() {
                combined.reserve(user_source.len() + source.len() + 2);
                combined.push_str(user_source);
                combined.push('\n');
            }
            let start = combined.len();
            combined.push_str(&source);
            body_ranges.push((start..combined.len(), item.qualified_name));
            combined.push('\n');
        }
    }
    if let Some(selected) = selected_provider_preparation(user_source, user_program, library) {
        if combined.is_empty() {
            combined.reserve(user_source.len() + selected.function_name.len() + 128);
            combined.push_str(user_source);
            combined.push('\n');
        }
        let source = managed_preparation_source(
            user_program,
            selected.function_name,
            &selected.arguments.join(", "),
            selected.managed_backend,
            &selected.contexts,
        );
        let start = combined.len();
        combined.push_str(&source);
        body_ranges.push((
            start..combined.len(),
            "the selected state-provider preparation",
        ));
        combined.push('\n');
    }
    if combined.is_empty() {
        return Ok(None);
    }
    let lexed = lexer::lex_lossless(&combined).map_err(|error| vec![error])?;
    let output = parser::parse_recovering(&combined, lexed.tokens().cloned().collect());
    if !output
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::DiagnosticSeverity::Error)
    {
        Ok(Some(output.program))
    } else {
        Err(output
            .diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                let origin = if diagnostic.span.start <= user_source.len() {
                    "the already-parsed user program"
                } else {
                    body_ranges
                        .iter()
                        .find(|(range, _)| range.contains(&diagnostic.span.start))
                        .map_or("a generated standard-library boundary", |(_, name)| *name)
                };
                diagnostic.notes.push(format!(
                    "combined standard-library parsing failed inside {origin}"
                ));
                diagnostic
            })
            .collect())
    }
}

struct SchemaClass<'ast> {
    image: &'ast str,
    namespace: Vec<&'ast str>,
    class: &'ast ManagedClassDecl,
}

fn managed_preparation_source(
    program: &Program,
    preparation: &str,
    arguments: &str,
    managed_backend: Option<ManagedRuntimeBackend>,
    contexts: &[SelectedProviderContext],
) -> String {
    let classes = schema_classes(program);
    let instance_classes = managed_instance_classes(program);
    if classes.is_empty() && contexts.is_empty() {
        return format!(
            "fn {PROVIDER_PREPARATION_FUNCTION}() {{ return await {preparation}({arguments}) }}"
        );
    }

    let mut source = format!("record {PROVIDER_BINDINGS_TYPE} {{\n");
    if !classes.is_empty() {
        source.push_str(&format!("    {MANAGED_POINTER_SIZE_FIELD}: u32,\n"));
    }
    for context in contexts {
        source.push_str(&format!(
            "    {}: {},\n",
            provider_context_field_name(context.index),
            context.ty
        ));
    }
    for class in &classes {
        if instance_classes.contains(&class.class.id) {
            source.push_str(&format!(
                "    {}: address,\n",
                managed_instance_header_name(class.class.id.index())
            ));
        }
        if class_fields(class.class).any(|field| field.is_static) {
            source.push_str(&format!(
                "    {}: address,\n",
                managed_static_table_name(class.class.id.index())
            ));
        }
        for field in class_fields(class.class) {
            source.push_str(&format!(
                "    {}: u32,\n",
                managed_field_offset_name(field.id.index())
            ));
        }
        for field in class
            .class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields)
        {
            source.push_str(&format!(
                "    {}: bool,\n",
                managed_field_presence_name(field.id.index())
            ));
        }
    }
    source.push_str("}\n");
    source.push_str(&format!("fn {PROVIDER_PREPARATION_FUNCTION}() {{\n"));
    if !classes.is_empty() {
        source.push_str(&format!(
            "    let __runtime = await {preparation}({arguments})\n"
        ));
    }
    for context in contexts {
        source.push_str(&format!(
            "    let {} = await {}()\n",
            provider_context_field_name(context.index),
            context.function_name,
        ));
    }
    if classes.is_empty() {
        source.push_str(&format!("    return {PROVIDER_BINDINGS_TYPE} {{\n"));
        push_provider_context_initializers(&mut source, contexts, "        ");
        source.push_str("    }\n");
        source.push_str("}\n");
        return source;
    }
    match managed_backend {
        Some(ManagedRuntimeBackend::Il2Cpp) => {
            source.push_str("    let __module = __runtime.il2cpp else await process.closed()\n");
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__module",
                "__module.pointerSize",
                false,
                contexts,
            ));
            source.push_str("    }\n");
        }
        Some(ManagedRuntimeBackend::Mono) => {
            source.push_str("    let __module = __runtime.mono else await process.closed()\n");
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__module",
                "match __module.pointerSize { PointerSize.Bit32 => 4, PointerSize.Bit64 => 8 }",
                true,
                contexts,
            ));
            source.push_str("    }\n");
        }
        None => {
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__runtime",
                "__runtime.pointerBytes()",
                true,
                contexts,
            ));
            source.push_str("    }\n");
        }
    }
    source.push_str("}\n");
    source
}

fn managed_backend_binding_source(
    classes: &[SchemaClass<'_>],
    instance_classes: &std::collections::HashSet<ManagedClassId>,
    module: &str,
    pointer_size: &str,
    instance_header_is_async: bool,
    contexts: &[SelectedProviderContext],
) -> String {
    let mut source = String::new();
    source.push_str(&format!(
        "            let {MANAGED_POINTER_SIZE_FIELD}: u32 = {pointer_size}\n"
    ));
    let mut images = std::collections::HashMap::new();
    for class in classes {
        let image_index = if let Some(index) = images.get(class.image) {
            *index
        } else {
            let index = images.len();
            source.push_str(&format!(
                "            let __image_{index} = await {module}.image({:?})\n",
                class.image
            ));
            images.insert(class.image, index);
            index
        };
        let candidates = class
            .class
            .metadata_name_candidates()
            .map(|(name, _)| {
                class
                    .namespace
                    .iter()
                    .copied()
                    .chain(std::iter::once(name))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let class_local = format!("__class_{}", class.class.id.index());
        source.push_str(&format!(
            "            let {class_local} = await __image_{image_index}.classAny([{candidates}])\n"
        ));
        if instance_classes.contains(&class.class.id) {
            let await_prefix = if instance_header_is_async {
                "await "
            } else {
                ""
            };
            source.push_str(&format!(
                "            let {} = {await_prefix}{class_local}.instanceHeader()\n",
                managed_instance_header_name(class.class.id.index())
            ));
        }
        if class_fields(class.class).any(|field| field.is_static) {
            source.push_str(&format!(
                "            let {} = await {class_local}.staticTable()\n",
                managed_static_table_name(class.class.id.index())
            ));
        }
        for field in &class.class.fields {
            push_required_managed_field_binding(&mut source, &class_local, field);
        }
        for group in &class.class.conditional_fields {
            for field in &group.fields {
                push_optional_managed_field_binding(&mut source, &class_local, field);
            }
        }
    }
    source.push_str(&format!("            {PROVIDER_BINDINGS_TYPE} {{\n"));
    push_provider_context_initializers(&mut source, contexts, "                ");
    source.push_str(&format!(
        "                {MANAGED_POINTER_SIZE_FIELD}: {MANAGED_POINTER_SIZE_FIELD},\n"
    ));
    for class in classes {
        if instance_classes.contains(&class.class.id) {
            let name = managed_instance_header_name(class.class.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
        if class_fields(class.class).any(|field| field.is_static) {
            let name = managed_static_table_name(class.class.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
        for field in class_fields(class.class) {
            let name = managed_field_offset_name(field.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
        for field in class
            .class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields)
        {
            let name = managed_field_presence_name(field.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
    }
    source.push_str("            }\n");
    source
}

fn push_provider_context_initializers(
    source: &mut String,
    contexts: &[SelectedProviderContext],
    indentation: &str,
) {
    for context in contexts {
        let name = provider_context_field_name(context.index);
        source.push_str(&format!("{indentation}{name}: {name},\n"));
    }
}

fn managed_field_candidates(field: &crate::ast::ManagedFieldDecl) -> String {
    field
        .binding_name_candidates()
        .into_iter()
        .map(|(name, _, _)| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_required_managed_field_binding(
    source: &mut String,
    class_local: &str,
    field: &crate::ast::ManagedFieldDecl,
) {
    let candidates = managed_field_candidates(field);
    let offset = managed_field_offset_name(field.id.index());
    source.push_str(&format!(
        "            let {offset} = (await {class_local}.fieldAny([{candidates}])).offset\n"
    ));
}

fn push_optional_managed_field_binding(
    source: &mut String,
    class_local: &str,
    field: &crate::ast::ManagedFieldDecl,
) {
    let candidates = managed_field_candidates(field);
    let offset = managed_field_offset_name(field.id.index());
    let probe = format!("__field_{}_conditional_probe", field.id.index());
    source.push_str(&format!(
        "            let {probe} = await {class_local}.probeFieldAny([{candidates}])\n"
    ));
    source.push_str(&format!(
        "            let {offset}: u32 = match {probe} {{ Some(field) => field.offset, None => 0 }}\n"
    ));
    let present = managed_field_presence_name(field.id.index());
    source.push_str(&format!(
        "            let {present} = match {probe} {{ Some(_) => true, None => false }}\n"
    ));
}

fn schema_classes(program: &Program) -> Vec<SchemaClass<'_>> {
    let mut classes = Vec::new();
    for image in &program.managed_images {
        collect_schema_classes(&mut classes, &image.name, &[], &image.items);
    }
    classes
}

fn collect_schema_classes<'ast>(
    output: &mut Vec<SchemaClass<'ast>>,
    image: &'ast str,
    namespace: &[&'ast str],
    items: &'ast [ManagedItemDecl],
) {
    for item in items {
        match item {
            ManagedItemDecl::Namespace(item) => {
                let mut nested = namespace.to_vec();
                nested.push(&item.name);
                collect_schema_classes(output, image, &nested, &item.items);
            }
            ManagedItemDecl::Class(class) => output.push(SchemaClass {
                image,
                namespace: namespace.to_vec(),
                class,
            }),
        }
    }
}

fn class_fields(class: &ManagedClassDecl) -> impl Iterator<Item = &crate::ast::ManagedFieldDecl> {
    class.fields.iter().chain(
        class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields),
    )
}

pub(crate) fn managed_field_offset_name(field: usize) -> String {
    format!("__field_{field}_offset")
}

pub(crate) fn managed_field_presence_name(field: usize) -> String {
    format!("__field_{field}_present")
}

pub(crate) fn managed_static_table_name(class: usize) -> String {
    format!("__class_{class}_static_table")
}

pub(crate) fn managed_instance_header_name(class: usize) -> String {
    format!("__class_{class}_instance_header")
}

pub(crate) fn provider_context_field_name(context: usize) -> String {
    format!("__provider_context_{context}")
}

fn managed_instance_classes(program: &Program) -> std::collections::HashSet<ManagedClassId> {
    struct Collector<'a> {
        classes: &'a [ManagedClassDecl],
        found: std::collections::HashSet<ManagedClassId>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Call {
                callee,
                receiver: None,
                ..
            } = &expression.kind
                && let [class_name, method] = callee.as_slice()
                && method == "instances"
                && let Some(class) = self.classes.iter().find(|class| class.name == *class_name)
            {
                self.found.insert(class.id);
            }
            visit::walk_expr(self, expression);
        }
    }

    let classes = program
        .managed_class_declarations()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut collector = Collector {
        classes: &classes,
        found: std::collections::HashSet::new(),
    };
    collector.visit_program(program);
    collector.found
}

struct SelectedProviderPreparation<'source> {
    function_name: &'static str,
    arguments: Vec<&'source str>,
    managed_backend: Option<ManagedRuntimeBackend>,
    contexts: Vec<SelectedProviderContext>,
}

fn selected_provider_preparation<'source>(
    user_source: &'source str,
    user_program: &Program,
    library: &StandardLibrary,
) -> Option<SelectedProviderPreparation<'source>> {
    let state = user_program.state.as_ref()?;
    let provider = state
        .provider
        .as_ref()
        .and_then(|reference| library.state_provider_by_name(&reference.name))
        .or_else(|| {
            state
                .provider
                .is_none()
                .then(|| library.default_state_provider())
                .flatten()
        })?;
    let (preparation, arguments, managed_backend) = if let Some(reference) = &state.provider
        && let Some(selected) = &reference.selector
    {
        let selector = provider
            .selectors
            .iter()
            .find(|candidate| candidate.name == selected.name)?;
        (
            selector.preparation,
            selected
                .arguments
                .iter()
                .map(|argument| &user_source[argument.span.start..argument.span.end])
                .collect(),
            selector.managed_backend,
        )
    } else {
        (provider.preparation?, Vec::new(), None)
    };
    let item = library.item(preparation);
    let Implementation::LibraryBody { function_name, .. } = item.implementation else {
        return None;
    };
    let referenced_contexts = provider_contexts_used(user_program, provider);
    let contexts = provider
        .contexts
        .iter()
        .enumerate()
        .filter(|(index, _)| referenced_contexts.contains(index))
        .map(|(index, context)| {
            let item = library.item(context.preparation);
            let Implementation::LibraryBody { function_name, .. } = item.implementation else {
                unreachable!("validated provider context preparations have source bodies")
            };
            SelectedProviderContext {
                index,
                ty: library.type_decl(context.ty).name,
                function_name,
            }
        })
        .collect();
    Some(SelectedProviderPreparation {
        function_name,
        arguments,
        managed_backend,
        contexts,
    })
}

fn provider_contexts_used(
    program: &Program,
    provider: &crate::stdlib::StdlibStateProvider,
) -> std::collections::HashSet<usize> {
    struct Collector<'a> {
        provider: &'a crate::stdlib::StdlibStateProvider,
        found: std::collections::HashSet<usize>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let root = match &expression.kind {
                ExprKind::Path(path) => path.first(),
                ExprKind::Call {
                    callee,
                    receiver: None,
                    ..
                } => callee.first(),
                _ => None,
            };
            if let Some(root) = root
                && let Some((index, _)) = self
                    .provider
                    .contexts
                    .iter()
                    .enumerate()
                    .find(|(_, context)| context.name == root)
            {
                self.found.insert(index);
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut collector = Collector {
        provider,
        found: std::collections::HashSet::new(),
    };
    collector.visit_program(program);
    collector.found
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn source_body_analysis_initializes_safely_from_parallel_callers() {
        let library = Arc::new(StandardLibrary {
            graph: Arc::new(
                crate::stdlib::graph::StandardLibraryGraph::build()
                    .expect("bundled graph is valid"),
            ),
        });
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let library = Arc::clone(&library);
                scope.spawn(move || library.initialize_source_body_operations());
            }
        });
        assert!(library.source_body_operations_are_initialized());
    }

    #[test]
    fn augmentation_parses_deterministically_under_parallel_load() {
        let library = Arc::new(StandardLibrary::new());
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let library = Arc::clone(&library);
                scope.spawn(move || {
                    for iteration in 0..16 {
                        augment_program_with_library_bodies(
                            "state \"parallel-probe.exe\" {}",
                            &crate::parse("state \"parallel-probe.exe\" {}")
                                .unwrap()
                                .into_syntax(),
                            &library,
                        )
                        .unwrap_or_else(|diagnostics| {
                            panic!("worker {worker} iteration {iteration} failed: {diagnostics:#?}")
                        });
                    }
                });
            }
        });
    }

    #[test]
    fn automatic_unity_schema_binding_targets_only_the_backend_neutral_adapter() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager {
                        static GameManager instance;
                        u32 state;
                    }
                }
                state Unity ["game.exe"] {}
            "#,
        )
        .expect("the schema fixture should parse")
        .into_syntax();

        let source = managed_preparation_source(&program, "__prepare", "", None, &[]);
        assert!(source.contains("await __runtime.image(\"Assembly-CSharp\")"));
        assert!(source.contains("await __image_0.classAny([\"GameManager\"])"));
        assert!(source.contains("__runtime.pointerBytes()"));
        assert!(!source.contains("__runtime.il2cpp"));
        assert!(!source.contains("__runtime.mono"));
        assert!(!source.contains("UnityRuntimeBackend"));
    }

    #[test]
    fn provider_context_preparation_is_demand_driven_without_managed_discovery() {
        let program = crate::parse(
            r#"
                state Unity ["game.exe"] {
                    scene = unity.scenes.active();
                }
            "#,
        )
        .expect("the provider-context fixture should parse")
        .into_syntax();
        let contexts = [SelectedProviderContext {
            index: 0,
            ty: "UnityContext",
            function_name: "__prepare_unity_context",
        }];

        let source =
            managed_preparation_source(&program, "__prepare_managed_runtime", "", None, &contexts);

        assert!(source.contains("let __provider_context_0 = await __prepare_unity_context()"));
        assert!(source.contains("__provider_context_0: UnityContext"));
        assert!(!source.contains("__prepare_managed_runtime"));
        assert!(!source.contains("let __runtime"));
    }

    #[test]
    fn provider_context_and_managed_schema_share_one_attachment_record() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager { u32 state; }
                }
                state Unity ["game.exe"] {
                    scene = unity.scenes.active();
                }
            "#,
        )
        .expect("the mixed Unity fixture should parse")
        .into_syntax();
        let contexts = [SelectedProviderContext {
            index: 0,
            ty: "UnityContext",
            function_name: "__prepare_unity_context",
        }];

        let source =
            managed_preparation_source(&program, "__prepare_managed_runtime", "", None, &contexts);

        assert!(source.contains("let __runtime = await __prepare_managed_runtime()"));
        assert!(source.contains("let __provider_context_0 = await __prepare_unity_context()"));
        assert!(source.contains("__provider_context_0: UnityContext"));
        assert!(source.contains(MANAGED_POINTER_SIZE_FIELD));
    }

    #[test]
    fn explicit_unity_schema_binding_keeps_its_prunable_concrete_backend() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager { u32 state; }
                }
                state Unity.il2cpp(2020) ["game.exe"] {}
            "#,
        )
        .expect("the schema fixture should parse")
        .into_syntax();

        let source = managed_preparation_source(
            &program,
            "__prepare",
            "2020",
            Some(ManagedRuntimeBackend::Il2Cpp),
            &[],
        );
        assert!(source.contains("let __module = __runtime.il2cpp"));
        assert!(source.contains("await __module.image(\"Assembly-CSharp\")"));
        assert!(!source.contains("__runtime.mono"));
    }
}
