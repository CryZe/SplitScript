//! Self-contained maintained guides bundled into every documentation frontend.

use std::collections::HashSet;

use crate::{
    CompilerContext,
    migration::{MigrationCatalog, MigrationConcept, MigrationTarget, markdown_anchor},
};

use super::{
    DocumentationIndexEntry, DocumentationPage,
    reference::{
        append_reference_table_header, escape_markdown_table_cell, migration_concept_uri,
        migration_target_uri, relative_document_link,
    },
};

#[derive(Debug, Clone, Copy)]
struct BundledGuide {
    uri: &'static str,
    title: &'static str,
    summary: &'static str,
    source: &'static str,
    migration_overview: bool,
}

const GUIDES: &[BundledGuide] = &[
    BundledGuide {
        uri: "/guides/asl-porting.md",
        title: "Porting ASL to SplitScript",
        summary: "Canonical compiler-checked recipes for migrating legacy ASL autosplitters.",
        source: include_str!("../../docs/ASL_PORTING.md"),
        migration_overview: true,
    },
    BundledGuide {
        uri: "/guides/from-csharp.md",
        title: "SplitScript for C# authors",
        summary: "A concise guide to SplitScript's types, control flow, errors, and autosplitter lifecycle for C# authors.",
        source: include_str!("../../docs/FROM_CSHARP.md"),
        migration_overview: false,
    },
    BundledGuide {
        uri: "/guides/from-javascript.md",
        title: "SplitScript for JavaScript authors",
        summary: "A concise guide to SplitScript's static types, fixed-width numbers, errors, and autosplitter lifecycle for JavaScript authors.",
        source: include_str!("../../docs/FROM_JAVASCRIPT.md"),
        migration_overview: false,
    },
    BundledGuide {
        uri: "/guides/from-rust.md",
        title: "SplitScript for Rust authors",
        summary: "A concise guide to SplitScript's inference, capabilities, error values, async behavior, and autosplitter lifecycle for Rust authors.",
        source: include_str!("../../docs/FROM_RUST.md"),
        migration_overview: false,
    },
];

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
    let source = rendered_guide_source(guide.source);
    let source = if guide.migration_overview {
        insert_migration_overview(guide.uri, &source, guide.source)
    } else {
        source
    };
    Some(DocumentationPage {
        uri: guide.uri.to_owned(),
        title: guide.title.to_owned(),
        markdown: format!(
            "[SplitScript reference]({}) / {}\n\n{}",
            relative_document_link(guide.uri, "/index.md"),
            guide.title,
            source,
        ),
    })
}

/// Adds a compact catalog-owned map ahead of the detailed recipes. Grouping by
/// cookbook heading keeps this useful as orientation rather than reproducing
/// the exhaustive migration index, while canonical target links still resolve
/// through the same language and standard-library catalogs as hover and
/// completion.
fn insert_migration_overview(uri: &str, rendered: &str, source: &str) -> String {
    let Some((introduction, recipes)) = rendered.split_once("\n## ") else {
        return rendered.to_owned();
    };
    let overview = migration_overview(uri, source);
    format!("{introduction}\n\n{overview}\n\n## {recipes}")
}

fn migration_overview(uri: &str, source: &str) -> String {
    let context = CompilerContext::new();
    let library = context.standard_library();
    let migration = MigrationCatalog::new(context);
    let mut markdown = String::from(
        "## Quick migration map\n\n\
         Each row links a legacy source pattern to its focused recipe and the canonical \
         SplitScript symbols used by that recipe. Open a symbol for its complete API \
         documentation.\n",
    );
    append_reference_table_header(&mut markdown, &["Legacy source", "Canonical SplitScript"]);

    for (heading, anchor) in cookbook_headings(source) {
        let concepts = migration
            .concepts()
            .iter()
            .filter(|concept| concept.cookbook_anchor == Some(anchor.as_str()))
            .collect::<Vec<_>>();
        if concepts.is_empty() {
            continue;
        }

        let recipe = format!("[{}](#{anchor})", escape_markdown_table_cell(&heading),);
        let targets = overview_targets(uri, &migration, &library, &concepts);
        markdown.push_str(&format!("\n| {recipe} | {targets} |"));
    }
    markdown
}

fn cookbook_headings(source: &str) -> impl Iterator<Item = (String, String)> + '_ {
    source.lines().filter_map(|line| {
        let heading = line.strip_prefix("## ")?;
        Some((heading.to_owned(), markdown_anchor(heading)))
    })
}

fn overview_targets(
    uri: &str,
    migration: &MigrationCatalog,
    library: &crate::stdlib::StandardLibrary,
    concepts: &[&MigrationConcept],
) -> String {
    let mut seen = HashSet::new();
    let mut seen_unavailable = HashSet::new();
    let mut entries = Vec::new();
    for concept in concepts {
        for target in concept.targets {
            if !seen.insert(*target) {
                continue;
            }
            entries.push(overview_target(uri, migration, library, *target));
        }
    }

    for concept in concepts.iter().filter(|concept| concept.targets.is_empty()) {
        if !seen_unavailable.insert(concept.id) {
            continue;
        }
        entries.push(format!(
            "{}: [{}]({})",
            concept.support.label(),
            escape_markdown_table_cell(concept.name),
            relative_document_link(uri, &migration_concept_uri(concept.id)),
        ));
    }
    entries.join(", ")
}

fn overview_target(
    uri: &str,
    migration: &MigrationCatalog,
    library: &crate::stdlib::StandardLibrary,
    target: MigrationTarget,
) -> String {
    let label = escape_markdown_table_cell(&migration.target_display(target));
    migration_target_uri(target, library).map_or_else(
        || format!("`{label}`"),
        |target_uri| format!("[{label}]({})", relative_document_link(uri, &target_uri),),
    )
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

    #[test]
    fn asl_guide_has_a_catalog_owned_migration_map() {
        let page = page("/guides/asl-porting.md").expect("ASL guide exists");
        assert!(page.markdown.contains("## Quick migration map"));
        assert!(
            page.markdown
                .contains("[Attachment state declarations](#attachment-state-declarations)")
        );
        assert!(page.markdown.contains("../language/state.md"));
        assert!(
            page.markdown
                .contains("[Process.scan](../stdlib/types/Process/methods/scan.md)")
        );
        assert!(
            page.markdown
                .contains("Planned: [shutdown lifecycle block](")
        );
    }

    #[test]
    fn language_background_guides_are_bundled_without_an_asl_migration_map() {
        for uri in [
            "/guides/from-csharp.md",
            "/guides/from-javascript.md",
            "/guides/from-rust.md",
        ] {
            let page = page(uri).unwrap_or_else(|| panic!("missing bundled guide `{uri}`"));
            assert!(page.markdown.starts_with("[SplitScript reference]"));
            assert!(!page.markdown.contains("## Quick migration map"));
        }
    }

    fn markdown_link_targets(markdown: &str) -> impl Iterator<Item = &str> {
        markdown
            .split("](")
            .skip(1)
            .filter_map(|tail| tail.split_once(')').map(|(target, _)| target))
    }
}
