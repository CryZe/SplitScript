//! Injection boundary for ordinary SplitScript standard-library bodies.
//!
//! Bodies are authored in the privileged catalog source, but statements and
//! expressions are parsed only after they have been appended to an ordinary
//! compilation unit. This keeps one body grammar and one semantic pipeline.

use crate::{Diagnostic, ast::Program, lexer, parser};

use super::{Implementation, ItemKind, StandardLibrary, StdlibItem, TypeRef};

pub(crate) const RESERVED_FUNCTION_PREFIX: &str = "__splitscript_stdlib_";

fn body_source(item: &StdlibItem, library: &StandardLibrary) -> String {
    let Implementation::LibraryBody {
        function_name,
        body,
    } = item.implementation
    else {
        unreachable!("body source is only generated for source-defined library items")
    };
    let mut parameters = Vec::new();
    if let ItemKind::Method { receiver } = item.kind {
        parameters.push(render_parameter("self", receiver, library));
    }
    parameters.extend(
        item.signature
            .parameters
            .iter()
            .map(|parameter| render_parameter(parameter.name, parameter.ty, library)),
    );
    let result = if contains_type_parameter(item.signature.result) {
        String::new()
    } else {
        format!(" -> {}", item.signature.result.render(library))
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
        TypeRef::Parameter(_) => true,
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
    for item in library.items() {
        if !matches!(item.implementation, Implementation::LibraryBody { .. }) {
            continue;
        }
        let source = body_source(item, library);
        if combined.is_empty() {
            combined.reserve(user_source.len() + source.len() + 2);
            combined.push_str(user_source);
            combined.push('\n');
        }
        combined.push_str(&source);
        combined.push('\n');
    }
    if combined.is_empty() {
        return Ok(None);
    }
    let lexed = lexer::lex_lossless(&combined).map_err(|error| vec![error])?;
    let output = parser::parse_recovering(&combined, lexed.tokens().cloned().collect());
    if output.diagnostics.is_empty() {
        Ok(Some(output.program))
    } else {
        Err(output.diagnostics)
    }
}
