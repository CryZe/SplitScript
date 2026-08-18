//! Rustdoc-style links from documentation prose into the generated reference.

use crate::{
    language::LanguageCatalog,
    stdlib::{FieldVisibility, StandardLibrary, StdlibSymbolId, TypeConstructorSyntax},
};

use super::reference::{core_type_uri, language_item_uri, relative_document_link, symbol_uri};

/// Resolves ``[`symbol`]`` occurrences and preserves unresolved occurrences so
/// a catalog validation error can identify the author's original spelling.
pub(super) fn render_links(source: &str, current_uri: &str, library: &StandardLibrary) -> String {
    rewrite_links(source, |label| {
        target_uri(label, library).map(|target| {
            format!(
                "[`{label}`]({})",
                relative_document_link(current_uri, &target)
            )
        })
    })
}

/// Hovers and completion details do not own reference-page-relative navigation.
/// Keep intra-doc markup readable there by reducing it to an ordinary code span.
pub(crate) fn strip_links(source: &str) -> String {
    rewrite_links(source, |label| Some(format!("`{label}`")))
}

pub(super) fn unresolved_links(source: &str) -> Vec<&str> {
    let mut unresolved = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("[`") {
        let candidate = &rest[start + 2..];
        let Some(end) = candidate.find("`]") else {
            break;
        };
        let after = &candidate[end + 2..];
        if !after.starts_with('(') && !rest[..start].ends_with('\\') {
            unresolved.push(&candidate[..end]);
        }
        rest = after;
    }
    unresolved
}

fn rewrite_links(source: &str, mut replacement: impl FnMut(&str) -> Option<String>) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("[`") {
        output.push_str(&rest[..start]);
        let candidate = &rest[start + 2..];
        let Some(end) = candidate.find("`]") else {
            output.push_str(&rest[start..]);
            return output;
        };
        let label = &candidate[..end];
        let consumed = start + 2 + end + 2;
        if label.is_empty() || rest[..start].ends_with('\\') {
            output.push_str(&rest[start..consumed]);
        } else if let Some(replacement) = replacement(label) {
            output.push_str(&replacement);
        } else {
            output.push_str(&rest[start..consumed]);
        }
        rest = &rest[consumed..];
    }
    output.push_str(rest);
    output
}

fn target_uri(label: &str, library: &StandardLibrary) -> Option<String> {
    let label = label.strip_suffix("()").unwrap_or(label);

    if let Some(item) = LanguageCatalog::new().item_for_source_token(label) {
        return Some(language_item_uri(item.id));
    }
    if let Some(ty) = library.core_types().iter().find(|ty| ty.name == label) {
        return Some(core_type_uri(ty.id, library));
    }
    if let Some(ty) = library.type_by_name(label) {
        return Some(symbol_uri(StdlibSymbolId::Type(ty.id), library));
    }
    if let Some(capability) = library
        .capabilities()
        .iter()
        .find(|capability| capability.name == label)
    {
        return Some(symbol_uri(
            StdlibSymbolId::Capability(capability.id),
            library,
        ));
    }
    if let Some(provider) = library.state_provider_by_name(label) {
        return Some(symbol_uri(
            StdlibSymbolId::StateProvider(provider.id),
            library,
        ));
    }
    if let Some(namespace) = library.namespaces().iter().find(|namespace| {
        namespace.path.join(".") == label
            || (namespace.name == label
                && library
                    .namespaces()
                    .iter()
                    .filter(|candidate| candidate.name == label)
                    .count()
                    == 1)
    }) {
        return Some(symbol_uri(StdlibSymbolId::Namespace(namespace.id), library));
    }
    if let Some(constructor) = library.type_constructors().iter().find(|constructor| {
        library.render_type_constructor(constructor.id) == label
            || (constructor.syntax == TypeConstructorSyntax::Named && constructor.name == label)
    }) {
        return Some(symbol_uri(
            StdlibSymbolId::TypeConstructor(constructor.id),
            library,
        ));
    }
    if let Some(item) = library.item_by_name(label) {
        return Some(symbol_uri(StdlibSymbolId::Item(item.id), library));
    }

    let mut symbols = library
        .fields()
        .iter()
        .filter(|field| {
            field.visibility == FieldVisibility::Public
                && format!("{}.{}", library.type_decl(field.owner).name, field.name) == label
        })
        .map(|field| StdlibSymbolId::Field(field.id))
        .chain(library.variants().iter().filter_map(|variant| {
            (format!("{}.{}", library.type_decl(variant.owner).name, variant.name) == label)
                .then_some(StdlibSymbolId::Variant(variant.id))
        }))
        .chain(
            library
                .items()
                .iter()
                .filter(|item| item.name == label)
                .map(|item| StdlibSymbolId::Item(item.id)),
        );
    let symbol = symbols.next()?;
    symbols
        .next()
        .is_none()
        .then(|| symbol_uri(symbol, library))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_language_and_standard_library_links_exactly() {
        let library = StandardLibrary::new();
        let rendered = render_links(
            "Use [`await`], [`Process.read`], and [`Duration`].",
            "/language/async.md",
            &library,
        );
        assert!(rendered.contains("[`await`](await.md)"), "{rendered}");
        assert!(
            rendered.contains("[`Process.read`](../stdlib/types/Process/methods/read.md)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("[`Duration`](../stdlib/types/Duration/index.md)"),
            "{rendered}"
        );
    }

    #[test]
    fn leaves_ambiguous_or_unknown_links_visible() {
        let rendered = render_links(
            "[`get`], [`DoesNotExist`], and [`Option`]",
            "/index.md",
            &StandardLibrary::new(),
        );
        assert_eq!(rendered, "[`get`], [`DoesNotExist`], and [`Option`]");
    }

    #[test]
    fn strips_reference_only_navigation_for_compact_markdown() {
        assert_eq!(strip_links("Use [`await`] here."), "Use `await` here.");
    }

    #[test]
    fn distinguishes_resolved_markdown_links_from_unresolved_intra_doc_links() {
        assert_eq!(
            unresolved_links("[`await`](await.md), then [`missing`]."),
            ["missing"]
        );
    }
}
