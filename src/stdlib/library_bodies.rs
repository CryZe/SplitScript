//! Injection boundary for ordinary SplitScript standard-library bodies.
//!
//! Bodies are authored in the privileged catalog source, but statements and
//! expressions are parsed only after they have been appended to an ordinary
//! compilation unit. This keeps one body grammar and one semantic pipeline.

use crate::{
    Diagnostic,
    ast::{ManagedClassDecl, ManagedItemDecl, Program},
    lexer, parser,
};

use super::{Implementation, ItemKind, Signature, StandardLibrary, StdlibItem, TypeRef};

pub(crate) const RESERVED_FUNCTION_PREFIX: &str = "__splitscript_stdlib_";
pub(crate) const PROVIDER_PREPARATION_FUNCTION: &str =
    "__splitscript_stdlib_selected_provider_preparation";
pub(crate) const MANAGED_BINDINGS_TYPE: &str = "__splitscript_stdlib_managed_bindings";
pub(crate) const MANAGED_POINTER_SIZE_FIELD: &str = "__pointer_size";

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
    if let Some((function_name, arguments)) =
        selected_provider_preparation(user_source, user_program, library)
    {
        if combined.is_empty() {
            combined.reserve(user_source.len() + function_name.len() + 128);
            combined.push_str(user_source);
            combined.push('\n');
        }
        let source = managed_preparation_source(user_program, function_name, &arguments.join(", "));
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

fn managed_preparation_source(program: &Program, preparation: &str, arguments: &str) -> String {
    let classes = schema_classes(program);
    if classes.is_empty() {
        return format!(
            "fn {PROVIDER_PREPARATION_FUNCTION}() {{ return await {preparation}({arguments}) }}"
        );
    }

    let mut source = format!("record {MANAGED_BINDINGS_TYPE} {{\n");
    source.push_str(&format!("    {MANAGED_POINTER_SIZE_FIELD}: u32,\n"));
    for class in &classes {
        if class_fields(class.class).any(|field| field.is_static) {
            source.push_str(&format!(
                "    {}: address,\n",
                managed_static_table_name(class.class.id.index())
            ));
        }
        if !class.class.layouts.is_empty() {
            source.push_str(&format!(
                "    {}: u32,\n",
                managed_layout_index_name(class.class.id.index())
            ));
        }
        for field in class_fields(class.class) {
            source.push_str(&format!(
                "    {}: u32,\n",
                managed_field_offset_name(field.id.index())
            ));
        }
    }
    source.push_str("}\n");
    source.push_str(&format!(
        "fn {PROVIDER_PREPARATION_FUNCTION}() {{\n    let __runtime = await {preparation}({arguments})\n    return match __runtime.backend {{\n"
    ));
    source.push_str("        UnityRuntimeBackend.Il2cpp => {\n");
    source.push_str("            let __module = __runtime.il2cpp else await process.closed()\n");
    source.push_str(&managed_backend_binding_source(
        &classes,
        "__module",
        "__module.pointerSize",
    ));
    source.push_str("        },\n        UnityRuntimeBackend.Mono => {\n");
    source.push_str("            let __module = __runtime.mono else await process.closed()\n");
    source.push_str(&managed_backend_binding_source(
        &classes,
        "__module",
        "match __module.pointerSize { PointerSize.Bit32 => 4, PointerSize.Bit64 => 8 }",
    ));
    source.push_str("        },\n    }\n}\n");
    source
}

fn managed_backend_binding_source(
    classes: &[SchemaClass<'_>],
    module: &str,
    pointer_size: &str,
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
        if class_fields(class.class).any(|field| field.is_static) {
            source.push_str(&format!(
                "            let {} = await {class_local}.staticTable()\n",
                managed_static_table_name(class.class.id.index())
            ));
        }
        for field in &class.class.fields {
            push_required_managed_field_binding(&mut source, &class_local, field);
        }
        if !class.class.layouts.is_empty() {
            push_managed_layout_binding(&mut source, &class_local, class.class);
        }
    }
    source.push_str(&format!("            {MANAGED_BINDINGS_TYPE} {{\n"));
    source.push_str(&format!(
        "                {MANAGED_POINTER_SIZE_FIELD}: {MANAGED_POINTER_SIZE_FIELD},\n"
    ));
    for class in classes {
        if class_fields(class.class).any(|field| field.is_static) {
            let name = managed_static_table_name(class.class.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
        if !class.class.layouts.is_empty() {
            let name = managed_layout_index_name(class.class.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
        for field in class_fields(class.class) {
            let name = managed_field_offset_name(field.id.index());
            source.push_str(&format!("                {name}: {name},\n"));
        }
    }
    source.push_str("            }\n");
    source
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

fn push_managed_layout_binding(source: &mut String, class_local: &str, class: &ManagedClassDecl) {
    let selected = format!("__class_{}_selected_layout", class.id.index());
    for layout in &class.layouts {
        for field in &layout.fields {
            let offset = managed_field_offset_name(field.id.index());
            source.push_str(&format!("            let {offset}: u32 = 0\n"));
        }
    }
    source.push_str(&format!("            let {selected}: u32? = None\n"));
    for (layout_index, layout) in class.layouts.iter().enumerate() {
        let probes = layout
            .fields
            .iter()
            .map(|field| {
                let probe = format!("__field_{}_probe", field.id.index());
                let candidates = managed_field_candidates(field);
                source.push_str(&format!(
                    "            let {probe} = await {class_local}.probeFieldAny([{candidates}])\n"
                ));
                probe
            })
            .collect::<Vec<_>>();
        let condition = if probes.is_empty() {
            "true".to_owned()
        } else {
            probes
                .iter()
                .map(|probe| format!("match {probe} {{ Some(_) => true, None => false }}"))
                .collect::<Vec<_>>()
                .join(" && ")
        };
        source.push_str(&format!("            if {condition} {{\n"));
        source.push_str(&format!(
            "                if match {selected} {{ Some(_) => true, None => false }} {{ await process.closed() }}\n"
        ));
        source.push_str(&format!("                {selected} = {layout_index}\n"));
        for (field, probe) in layout.fields.iter().zip(probes) {
            let value = format!("__field_{}_selected", field.id.index());
            let offset = managed_field_offset_name(field.id.index());
            source.push_str(&format!(
                "                let {value} = {probe} else await process.closed()\n"
            ));
            source.push_str(&format!("                {offset} = {value}.offset\n"));
        }
        source.push_str("            }\n");
    }
    let layout = managed_layout_index_name(class.id.index());
    source.push_str(&format!(
        "            let {layout} = {selected} else await process.closed()\n"
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
    class
        .fields
        .iter()
        .chain(class.layouts.iter().flat_map(|layout| &layout.fields))
}

pub(crate) fn managed_field_offset_name(field: usize) -> String {
    format!("__field_{field}_offset")
}

pub(crate) fn managed_static_table_name(class: usize) -> String {
    format!("__class_{class}_static_table")
}

pub(crate) fn managed_layout_index_name(class: usize) -> String {
    format!("__class_{class}_layout")
}

fn selected_provider_preparation<'source>(
    user_source: &'source str,
    user_program: &Program,
    library: &StandardLibrary,
) -> Option<(&'static str, Vec<&'source str>)> {
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
    let (preparation, arguments) = if let Some(reference) = &state.provider
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
        )
    } else {
        (provider.preparation?, Vec::new())
    };
    let item = library.item(preparation);
    let Implementation::LibraryBody { function_name, .. } = item.implementation else {
        return None;
    };
    Some((function_name, arguments))
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
}
