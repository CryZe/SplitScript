//! Injection boundary for ordinary SplitScript standard-library bodies.
//!
//! Bodies are authored in the privileged catalog source, but statements and
//! expressions are parsed only after they have been appended to an ordinary
//! compilation unit. This keeps one body grammar and one semantic pipeline.

use crate::{Diagnostic, ast::Program, lexer, parser};

use super::{Implementation, ItemKind, Signature, StandardLibrary, StdlibItem, TypeRef};

pub(crate) const RESERVED_FUNCTION_PREFIX: &str = "__splitscript_stdlib_";

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
        TypeRef::Core(_) | TypeRef::Standard(_) => false,
    }
}

pub(crate) fn augment_program_with_library_bodies(
    user_source: &str,
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
