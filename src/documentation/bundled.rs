//! Self-contained maintained guides bundled into every documentation frontend.

use std::collections::HashSet;

use crate::{
    CompilerContext,
    migration::{
        MigrationCatalog, MigrationConcept, MigrationConceptId, MigrationTarget, markdown_anchor,
    },
};

use super::{
    DocumentationIndexEntry, DocumentationPage, code,
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
    lifecycle_matrix: bool,
}

const GUIDES: &[BundledGuide] = &[
    BundledGuide {
        uri: "/guides/getting-started.md",
        title: "Getting started",
        summary: "Build a first SplitScript autosplitter with the VS Code extension or native CLI.",
        source: include_str!("../../docs/GETTING_STARTED.md"),
        migration_overview: false,
        lifecycle_matrix: false,
    },
    BundledGuide {
        uri: "/guides/asl-porting.md",
        title: "Porting ASL to SplitScript",
        summary: "Canonical compiler-checked recipes for migrating legacy ASL autosplitters.",
        source: include_str!("../../docs/ASL_PORTING.md"),
        migration_overview: true,
        lifecycle_matrix: false,
    },
    BundledGuide {
        uri: "/guides/from-csharp.md",
        title: "SplitScript for C# authors",
        summary: "A concise guide to SplitScript's types, control flow, errors, and autosplitter lifecycle for C# authors.",
        source: include_str!("../../docs/FROM_CSHARP.md"),
        migration_overview: false,
        lifecycle_matrix: false,
    },
    BundledGuide {
        uri: "/guides/from-javascript.md",
        title: "SplitScript for JavaScript authors",
        summary: "A concise guide to SplitScript's static types, fixed-width numbers, errors, and autosplitter lifecycle for JavaScript authors.",
        source: include_str!("../../docs/FROM_JAVASCRIPT.md"),
        migration_overview: false,
        lifecycle_matrix: false,
    },
    BundledGuide {
        uri: "/guides/from-rust.md",
        title: "SplitScript for Rust authors",
        summary: "A concise guide to SplitScript's inference, capabilities, error values, async behavior, and autosplitter lifecycle for Rust authors.",
        source: include_str!("../../docs/FROM_RUST.md"),
        migration_overview: false,
        lifecycle_matrix: false,
    },
    BundledGuide {
        uri: "/guides/decision-guides.md",
        title: "Decision guides",
        summary: "Choose a lifecycle block, state-field form, failure boundary, or string unit.",
        source: include_str!("../../docs/DECISION_GUIDES.md"),
        migration_overview: false,
        lifecycle_matrix: true,
    },
];

/// High-frequency ASL concepts that porters should be able to discover before
/// reading the cookbook linearly. Identities, summaries, targets, and links
/// still come from the migration and public-symbol catalogs.
const COMMON_ASL_CONCEPTS: &[MigrationConceptId] = &[
    MigrationConceptId::new("asl.state.attachment"),
    MigrationConceptId::new("asl.state.version-label"),
    MigrationConceptId::new("asl.timer.state"),
    MigrationConceptId::new("asl.timer.current-split-index"),
    MigrationConceptId::new("asl.lifecycle.exit-game-time-cleanup"),
    MigrationConceptId::new("asl.runtime.refresh-rate"),
    MigrationConceptId::new("asl.memory.primitive-types"),
    MigrationConceptId::new("asl.process.modules"),
    MigrationConceptId::new("asl.settings.dynamic-lookup"),
    MigrationConceptId::new("asl.settings.finite-family"),
    MigrationConceptId::new("asl.collection.list"),
];

pub(super) fn index() -> impl Iterator<Item = DocumentationIndexEntry> {
    GUIDES.iter().map(|guide| DocumentationIndexEntry {
        uri: guide.uri.to_owned(),
        title: guide.title.to_owned(),
        kind: "guide",
        summary: guide.summary.to_owned(),
        raw_summary: guide.summary,
        search_text: format!("{} {}", guide.summary, guide.source),
        signature: None,
    })
}

pub(super) fn page(
    uri: &str,
    library: &crate::stdlib::StandardLibrary,
    semantic_examples: bool,
) -> Option<DocumentationPage> {
    let guide = GUIDES.iter().find(|guide| guide.uri == uri)?;
    let source = rendered_guide_source(guide.source, guide.uri, library, semantic_examples);
    let source = if guide.migration_overview {
        insert_migration_overview(guide.uri, &source, guide.source)
    } else {
        source
    };
    let source = if guide.lifecycle_matrix {
        insert_lifecycle_matrix(guide.uri, &source)
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

fn insert_lifecycle_matrix(uri: &str, source: &str) -> String {
    let language = crate::language::LanguageCatalog::new();
    let mut matrix = String::new();
    append_reference_table_header(
        &mut matrix,
        &[
            "Action",
            "Timing",
            "Available context",
            "Suspension",
            "Result",
            "Fallthrough",
        ],
    );
    for action in [
        crate::ast::ActionKind::Setup,
        crate::ast::ActionKind::SelectProcess,
        crate::ast::ActionKind::OnStart,
        crate::ast::ActionKind::OnReset,
        crate::ast::ActionKind::OnAttach,
        crate::ast::ActionKind::OnStateReady,
        crate::ast::ActionKind::WhileAttached,
        crate::ast::ActionKind::Start,
        crate::ast::ActionKind::IsLoading,
        crate::ast::ActionKind::GameTime,
        crate::ast::ActionKind::Reset,
        crate::ast::ActionKind::Split,
        crate::ast::ActionKind::OnDetach,
    ] {
        let item = language.action(action);
        let facts = language.action_reference_facts(action);
        let target = super::reference::language_item_uri(item.id);
        matrix.push_str(&format!(
            "\n| [{}]({}) | {} | {} | {} | <code>{}</code> | {} |",
            item.name,
            relative_document_link(uri, &target),
            facts.timing,
            facts.available_context,
            facts.suspension,
            facts.result,
            facts.fallthrough,
        ));
    }
    source.replace("<!-- lifecycle-matrix -->", &matrix)
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

    markdown.push_str(
        "\n\n### Common ASL concepts\n\n\
         Use this checklist before concluding that a familiar ASL facility is missing.\n",
    );
    append_reference_table_header(&mut markdown, &["ASL concept", "Canonical SplitScript"]);
    for id in COMMON_ASL_CONCEPTS {
        let concept = migration
            .concept(*id)
            .unwrap_or_else(|| panic!("missing common ASL concept `{}`", id.as_str()));
        let concept_link = concept.cookbook_anchor.map_or_else(
            || {
                format!(
                    "[{}]({})",
                    escape_markdown_table_cell(concept.name),
                    relative_document_link(uri, &migration_concept_uri(concept.id)),
                )
            },
            |anchor| format!("[{}](#{anchor})", escape_markdown_table_cell(concept.name),),
        );
        let targets = overview_targets(uri, &migration, &library, &[concept]);
        markdown.push_str(&format!("\n| {concept_link} | {targets} |"));
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
fn rendered_guide_source(
    source: &str,
    uri: &str,
    library: &crate::stdlib::StandardLibrary,
    semantic_examples: bool,
) -> String {
    let (rendered, examples) = guide_parts(source);
    render_guide_examples(&rendered, &examples, uri, library, semantic_examples)
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

fn render_guide_examples(
    source: &str,
    validation_examples: &[String],
    uri: &str,
    library: &crate::stdlib::StandardLibrary,
    semantic_examples: bool,
) -> String {
    let mut rendered = String::with_capacity(source.len());
    let mut current_example = None::<String>;
    let mut validations = validation_examples.iter();

    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if current_example.is_none() && content == "```splitscript" {
            current_example = Some(String::new());
            continue;
        }
        if let Some(example) = current_example.as_mut() {
            if content == "```" {
                let example = current_example.take().expect("example is present");
                let validation = validations
                    .next()
                    .expect("every visible guide example has validation source");
                rendered.push_str(&code::checked_fragment(
                    &example,
                    validation,
                    uri,
                    library,
                    semantic_examples,
                ));
                rendered.push('\n');
            } else {
                example.push_str(line);
            }
            continue;
        }
        rendered.push_str(line);
    }

    assert!(
        current_example.is_none(),
        "unclosed rendered SplitScript guide fence"
    );
    assert!(
        validations.next().is_none(),
        "every validation example is rendered exactly once"
    );
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_guides_have_no_repository_document_or_source_dependencies() {
        for guide in GUIDES {
            for target in markdown_link_targets(guide.source) {
                assert!(
                    target.contains("://")
                        || target.starts_with('#')
                        || target.starts_with("method@")
                        || target.starts_with("fn@")
                        || target.starts_with("keyword@")
                        || target.starts_with("operator@")
                        || target.starts_with("type@")
                        || target.starts_with("syntax@")
                        || target.starts_with("field@")
                        || target.starts_with("variant@")
                        || target.starts_with("capability@")
                        || target.starts_with("namespace@")
                        || target.starts_with("provider@"),
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
        let rendered = rendered_guide_source(
            source,
            "/guides/test.md",
            &crate::stdlib::StandardLibrary::new(),
            true,
        );
        assert!(rendered.starts_with("before\n<pre class=\"hljs splitscript-code\">"));
        assert!(rendered.contains(">print</span>"));
        assert!(!rendered.contains("game.exe"));
        assert!(!rendered.contains("onAttach"));
        assert!(rendered.ends_with("after\n"));
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
    fn bundled_guides_render_semantic_code_with_symbol_links() {
        let page = page(
            "/guides/from-rust.md",
            &crate::stdlib::StandardLibrary::new(),
            true,
        )
        .expect("Rust guide exists");
        assert!(
            page.markdown
                .contains("<pre class=\"hljs splitscript-code\">")
        );
        assert!(page.markdown.contains("data-splitscript-token=\"keyword\""));
        assert!(page.markdown.contains("href=\"../language/"));
        assert!(!page.markdown.contains("```splitscript"));
    }

    #[test]
    fn asl_guide_has_a_catalog_owned_migration_map() {
        let page = page(
            "/guides/asl-porting.md",
            &crate::stdlib::StandardLibrary::new(),
            true,
        )
        .expect("ASL guide exists");
        assert!(page.markdown.contains("## Quick migration map"));
        assert!(page.markdown.contains("### Common ASL concepts"));
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
        assert!(
            page.markdown
                .contains("[refreshRate](../migration/asl/runtime/refresh-rate.md)")
        );
    }

    #[test]
    fn language_background_guides_are_bundled_without_an_asl_migration_map() {
        for uri in [
            "/guides/from-csharp.md",
            "/guides/from-javascript.md",
            "/guides/from-rust.md",
        ] {
            let page = page(uri, &crate::stdlib::StandardLibrary::new(), true)
                .unwrap_or_else(|| panic!("missing bundled guide `{uri}`"));
            assert!(page.markdown.starts_with("[SplitScript reference]"));
            assert!(!page.markdown.contains("## Quick migration map"));
        }
    }

    #[test]
    fn decision_guide_uses_the_catalog_owned_lifecycle_matrix() {
        let page = page(
            "/guides/decision-guides.md",
            &crate::stdlib::StandardLibrary::new(),
            true,
        )
        .expect("decision guide is bundled");
        assert!(!page.markdown.contains("<!-- lifecycle-matrix -->"));
        for heading in [
            "## Choose a lifecycle block",
            "## Choose a state field form",
            "## Choose absence, failure, retrying, or waiting",
            "## Choose the correct string unit",
        ] {
            assert!(page.markdown.contains(heading));
        }
        for action in [
            "setup",
            "onStart",
            "onReset",
            "onAttach",
            "onStateReady",
            "whileAttached",
            "start",
            "isLoading",
            "gameTime",
            "reset",
            "split",
            "onDetach",
        ] {
            assert!(
                page.markdown.contains(&format!("[{action}](")),
                "missing lifecycle row for `{action}`"
            );
        }
        let is_loading = page.markdown.find("[isLoading](").unwrap();
        let game_time = page.markdown.find("[gameTime](").unwrap();
        let reset = page.markdown.find("[reset](").unwrap();
        let split = page.markdown.find("[split](").unwrap();
        assert!(is_loading < game_time && game_time < reset && reset < split);
    }

    fn markdown_link_targets(markdown: &str) -> impl Iterator<Item = &str> {
        markdown
            .split("](")
            .skip(1)
            .filter_map(|tail| tail.split_once(')').map(|(target, _)| target))
    }
}
