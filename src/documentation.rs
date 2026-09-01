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
    use super::prose_markdown;

    #[test]
    fn prose_omits_empty_and_repeated_details() {
        assert_eq!(prose_markdown("Summary.", ""), "Summary.");
        assert_eq!(prose_markdown("Summary.", "Summary."), "Summary.");
        assert_eq!(
            prose_markdown("Summary.", "Useful details."),
            "Summary.\n\nUseful details."
        );
    }
}
