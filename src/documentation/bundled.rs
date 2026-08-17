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
            rendered_guide_source(guide.source),
        ),
    })
}

/// Renders rustdoc-style hidden lines out of SplitScript examples while
/// keeping them in the checked source. A line containing only `#` contributes
/// a blank source line; `# code` contributes `code` to validation. Other
/// Markdown and non-SplitScript fences are byte-for-byte ordinary content.
fn rendered_guide_source(source: &str) -> String {
    guide_parts(source).0
}

#[cfg(test)]
fn validation_examples(source: &str) -> Vec<String> {
    guide_parts(source).1
}

fn guide_parts(source: &str) -> (String, Vec<String>) {
    let mut rendered = String::with_capacity(source.len());
    let mut examples = Vec::new();
    let mut current_example = None::<String>;

    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if current_example.is_none() && content == "```splitscript" {
            current_example = Some(String::new());
            rendered.push_str(line);
            continue;
        }
        if let Some(example) = current_example.as_mut() {
            if content == "```" {
                examples.push(current_example.take().expect("example is present"));
                rendered.push_str(line);
                continue;
            }

            if content == "#" {
                example.push('\n');
            } else if let Some(hidden) = content.strip_prefix("# ") {
                example.push_str(hidden);
                example.push('\n');
            } else {
                example.push_str(content);
                example.push('\n');
                rendered.push_str(line);
            }
            continue;
        }

        rendered.push_str(line);
    }

    assert!(
        current_example.is_none(),
        "unclosed SplitScript guide fence"
    );
    (rendered, examples)
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

    #[test]
    fn hidden_example_context_is_checked_but_not_rendered() {
        let source = "before\n```splitscript\n# state \"game.exe\" {}\n# onAttach {\nprint(7)\n# }\n```\nafter\n";
        assert_eq!(
            rendered_guide_source(source),
            "before\n```splitscript\nprint(7)\n```\nafter\n"
        );
        assert_eq!(
            validation_examples(source),
            ["state \"game.exe\" {}\nonAttach {\nprint(7)\n}\n"]
        );
    }

    #[test]
    fn bundled_splitscript_examples_compile() {
        let mut failures = Vec::new();
        for guide in GUIDES {
            for (index, example) in validation_examples(guide.source).into_iter().enumerate() {
                if let Err(diagnostics) = crate::compile(&example) {
                    let diagnostics = diagnostics
                        .iter()
                        .map(|diagnostic| {
                            format!(
                                "{:?} at {}..{}: {}",
                                diagnostic.code,
                                diagnostic.span.start,
                                diagnostic.span.end,
                                diagnostic.message,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    failures.push(format!(
                        "{} example {}:\n{diagnostics}\n--- source ---\n{example}",
                        guide.uri,
                        index + 1,
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "bundled guide examples did not compile:\n{}",
            failures.join("\n\n")
        );
    }

    fn markdown_link_targets(markdown: &str) -> impl Iterator<Item = &str> {
        markdown
            .split("](")
            .skip(1)
            .filter_map(|tail| tail.split_once(')').map(|(target, _)| target))
    }
}
