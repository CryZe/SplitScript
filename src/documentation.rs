//! Canonical generated documentation views over compiler-owned catalogs.

mod bundled;
mod code;
mod entry;
mod intra_doc;
mod reference;
mod validation;

pub use entry::{DocumentedParameter, StandardLibraryDocumentation};
pub(crate) use intra_doc::strip_links as strip_intra_doc_links;
pub(crate) use reference::migration_topic_uri;
pub use reference::{DocumentationIndexEntry, DocumentationPage, DocumentationReference};
pub(crate) use reference::{language_item_uri, symbol_uri};

pub(crate) const STATE_PROVIDER_INDEX_URI: &str = "/stdlib/state-providers/index.md";

/// Escapes a compiler-generated symbol spelling for insertion into ordinary
/// Markdown text or a link label.
///
/// CommonMark delimiters that can reinterpret plain symbol text are
/// backslash-escaped. Besides protecting brackets, pipes, and formatting
/// markers, this prevents a unary generic spelling such as `Set<T>` from being
/// parsed as an inline HTML tag while a multi-parameter spelling happens to
/// survive. Keep catalog names raw everywhere else; escaping belongs
/// exclusively at the Markdown serialization boundary.
#[doc(hidden)]
pub fn escape_markdown_symbol(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Joins the short and extended prose without manufacturing an empty or
/// duplicated paragraph. Catalog producers preserve these as distinct fields,
/// while this defensive equality check also keeps externally supplied or old
/// generated catalogs readable.
pub(crate) fn prose_markdown(summary: &str, details: &str) -> String {
    let summary = summary.trim();
    let details = details.trim();
    if details.is_empty() || details == summary {
        summary.to_owned()
    } else {
        format!("{summary}\n\n{details}")
    }
}

#[cfg(test)]
mod tests {
    use super::{escape_markdown_symbol, prose_markdown};

    #[test]
    fn prose_omits_empty_and_repeated_details() {
        assert_eq!(prose_markdown("Summary.", ""), "Summary.");
        assert_eq!(prose_markdown("Summary.", "Summary."), "Summary.");
        assert_eq!(
            prose_markdown("Summary.", "Useful details."),
            "Summary.\n\nUseful details."
        );
    }

    #[test]
    fn compiler_symbols_escape_markdown_delimiters_without_rewriting_operators() {
        assert_eq!(
            escape_markdown_symbol("[T] Map<K, V>.value | T? + T!"),
            "\\[T\\] Map\\<K, V\\>.value \\| T? + T!"
        );
    }
}
