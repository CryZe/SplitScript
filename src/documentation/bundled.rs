//! Self-contained maintained guides bundled into every documentation frontend.

use super::{DocumentationIndexEntry, DocumentationPage, reference::relative_document_link};

#[derive(Debug, Clone, Copy)]
struct BundledGuide {
    uri: &'static str,
    title: &'static str,
    summary: &'static str,
    source: &'static str,
}

const GUIDES: &[BundledGuide] = &[BundledGuide {
    uri: "/guides/asl-porting.md",
    title: "Porting ASL to SplitScript",
    summary: "Canonical compiler-checked recipes for migrating legacy ASL autosplitters.",
    source: include_str!("../../docs/ASL_PORTING.md"),
}];

pub(super) fn index() -> impl Iterator<Item = DocumentationIndexEntry> {
    GUIDES.iter().map(|guide| DocumentationIndexEntry {
        uri: guide.uri.to_owned(),
        title: guide.title.to_owned(),
        kind: "guide",
        summary: guide.summary,
        signature: None,
    })
}

pub(super) fn page(uri: &str) -> Option<DocumentationPage> {
    let guide = GUIDES.iter().find(|guide| guide.uri == uri)?;
    Some(DocumentationPage {
        uri: guide.uri.to_owned(),
        title: guide.title.to_owned(),
        markdown: format!(
            "[SplitScript reference]({}) / {}\n\n{}",
            relative_document_link(guide.uri, "/index.md"),
            guide.title,
            guide.source,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_guides_have_no_repository_document_or_source_dependencies() {
        for guide in GUIDES {
            for target in markdown_link_targets(guide.source) {
                assert!(
                    target.contains("://") || target.starts_with('#'),
                    "bundled guide {} depends on local resource `{target}`",
                    guide.uri,
                );
            }
            assert!(!guide.source.contains("examples/"));
        }
    }

    fn markdown_link_targets(markdown: &str) -> impl Iterator<Item = &str> {
        markdown
            .split("](")
            .skip(1)
            .filter_map(|tail| tail.split_once(')').map(|(target, _)| target))
    }
}
