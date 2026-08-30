//! Validated migration knowledge and human-readable capability reporting.

pub use splitscript_syntax::migration::{
    ASL_SETTINGS_ADD_DIAGNOSTIC, ASL_SETTINGS_LOOKUP_DIAGNOSTIC, ForeignSpelling,
    ForeignSpellingContext, ForeignSpellingReplacement, MigrationConcept, MigrationConceptId,
    MigrationDiagnostic, MigrationDiagnosticId, MigrationSupport, MigrationTarget, SourceLanguage,
    diagnostic as migration_diagnostic, foreign_spelling, legacy_array_field_diagnostic,
    legacy_managed_method_diagnostic, legacy_set_field_diagnostic, legacy_static_call_diagnostic,
    legacy_string_field_diagnostic, legacy_string_method_diagnostic, legacy_type_diagnostic,
    legacy_value_path_diagnostic,
};

use crate::{CompilerContext, language::LanguageCatalog};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationNavigationGroup {
    AslAttachmentState,
    AslProcessMemory,
    AslLifecycleTimer,
    AslSettings,
    AslCollectionsText,
    AslUnityEmulators,
    AslUnsupportedHost,
    CSharp,
    JavaScript,
    Rust,
}

impl MigrationNavigationGroup {
    pub(crate) const ALL: &[Self] = &[
        Self::AslAttachmentState,
        Self::AslProcessMemory,
        Self::AslLifecycleTimer,
        Self::AslSettings,
        Self::AslCollectionsText,
        Self::AslUnityEmulators,
        Self::AslUnsupportedHost,
        Self::CSharp,
        Self::JavaScript,
        Self::Rust,
    ];

    pub(crate) const fn source(self) -> SourceLanguage {
        match self {
            Self::AslAttachmentState
            | Self::AslProcessMemory
            | Self::AslLifecycleTimer
            | Self::AslSettings
            | Self::AslCollectionsText
            | Self::AslUnityEmulators
            | Self::AslUnsupportedHost => SourceLanguage::Asl,
            Self::CSharp => SourceLanguage::CSharp,
            Self::JavaScript => SourceLanguage::JavaScript,
            Self::Rust => SourceLanguage::Rust,
        }
    }

    pub(crate) const fn task(self) -> Option<&'static str> {
        match self {
            Self::AslAttachmentState => Some("Attachment and state"),
            Self::AslProcessMemory => Some("Process and memory"),
            Self::AslLifecycleTimer => Some("Lifecycle and timer"),
            Self::AslSettings => Some("Settings"),
            Self::AslCollectionsText => Some("Collections and text"),
            Self::AslUnityEmulators => Some("Unity and emulators"),
            Self::AslUnsupportedHost => Some("Unsupported host behavior"),
            Self::CSharp | Self::JavaScript | Self::Rust => None,
        }
    }
}

pub(crate) fn migration_navigation_group(concept: &MigrationConcept) -> MigrationNavigationGroup {
    let primary_source = concept
        .sources
        .first()
        .copied()
        .expect("migration concepts have at least one source language");
    if primary_source != SourceLanguage::Asl {
        return match primary_source {
            SourceLanguage::Asl => unreachable!(),
            SourceLanguage::CSharp => MigrationNavigationGroup::CSharp,
            SourceLanguage::JavaScript => MigrationNavigationGroup::JavaScript,
            SourceLanguage::Rust => MigrationNavigationGroup::Rust,
        };
    }

    if matches!(
        concept.support,
        MigrationSupport::Planned | MigrationSupport::SandboxNonGoal
    ) {
        return MigrationNavigationGroup::AslUnsupportedHost;
    }

    let id = concept.id.as_str();
    if id.starts_with("asl.unity.") || id.starts_with("asr.emulator.") {
        MigrationNavigationGroup::AslUnityEmulators
    } else if id.starts_with("asl.settings.") {
        MigrationNavigationGroup::AslSettings
    } else if id.starts_with("asl.collection.") || id.starts_with("iteration.") {
        MigrationNavigationGroup::AslCollectionsText
    } else if id.starts_with("asl.lifecycle.")
        || id.starts_with("asl.timer.")
        || id.starts_with("asl.runtime.")
        || id.starts_with("asl.time.")
    {
        MigrationNavigationGroup::AslLifecycleTimer
    } else if id.starts_with("asl.process.")
        || id.starts_with("asl.memory.")
        || id.starts_with("asl.async.")
    {
        MigrationNavigationGroup::AslProcessMemory
    } else if id.starts_with("asl.state.") {
        MigrationNavigationGroup::AslAttachmentState
    } else {
        panic!(
            "ASL migration concept `{}` needs an explicit navigation task",
            concept.id.as_str()
        )
    }
}

#[derive(Debug, Clone)]
pub struct MigrationCatalog {
    context: CompilerContext,
}

impl Default for MigrationCatalog {
    fn default() -> Self {
        Self::new(CompilerContext::new())
    }
}

impl MigrationCatalog {
    pub fn new(context: CompilerContext) -> Self {
        Self { context }
    }

    pub fn concepts(&self) -> &'static [MigrationConcept] {
        splitscript_syntax::migration::CONCEPTS
    }

    pub fn concept(&self, id: MigrationConceptId) -> Option<&'static MigrationConcept> {
        splitscript_syntax::migration::concept(id)
    }

    pub fn diagnostics(&self) -> &'static [MigrationDiagnostic] {
        splitscript_syntax::migration::DIAGNOSTICS
    }

    /// Validates that migration metadata points into the active compiler
    /// catalogs instead of forming a stale parallel API inventory.
    pub fn validate(&self) -> Vec<String> {
        let language = LanguageCatalog::new();
        let standard_library = self.context.standard_library();
        let mut errors = Vec::new();

        for concept in self.concepts() {
            if concept.targets.is_empty()
                && matches!(
                    concept.support,
                    MigrationSupport::Direct | MigrationSupport::TypedPattern
                )
            {
                errors.push(format!(
                    "migration concept `{}` has no canonical target",
                    concept.id.as_str()
                ));
            }
            for target in concept.targets {
                let exists = match *target {
                    MigrationTarget::Language(name) => {
                        language.item_for_source_token(name).is_some()
                    }
                    MigrationTarget::StandardLibraryType(name) => {
                        standard_library.type_by_name(name).is_some()
                    }
                    MigrationTarget::StandardLibraryItem(name) => {
                        standard_library.item_by_name(name).is_some()
                    }
                    MigrationTarget::StateProvider(name) => {
                        standard_library.state_provider_by_name(name).is_some()
                    }
                };
                if !exists {
                    errors.push(format!(
                        "migration concept `{}` references missing canonical target `{}`",
                        concept.id.as_str(),
                        target.display()
                    ));
                }
            }
            if let Some(anchor) = concept.cookbook_anchor
                && !cookbook_anchors().any(|candidate| candidate == anchor)
            {
                errors.push(format!(
                    "migration concept `{}` references missing cookbook anchor `{anchor}`",
                    concept.id.as_str()
                ));
            }
        }

        for diagnostic in self.diagnostics() {
            if self.concept(diagnostic.concept).is_none() {
                errors.push(format!(
                    "migration diagnostic `{}` references missing concept `{}`",
                    diagnostic.id.as_str(),
                    diagnostic.concept.as_str(),
                ));
            }
        }

        errors
    }

    pub fn capability_index_markdown(&self) -> String {
        let mut output = String::from(
            "<!-- Generated from the compiler-owned migration catalog. -->\n\
# Migration by source\n\n\
Find the source concept or task first, then follow its canonical SplitScript direction. Exact APIs remain documented by the compiler-owned language and standard-library reference.\n",
        );
        let mut current_source = None;
        for group in MigrationNavigationGroup::ALL {
            let concepts = self
                .concepts()
                .iter()
                .filter(|concept| migration_navigation_group(concept) == *group)
                .collect::<Vec<_>>();
            if concepts.is_empty() {
                continue;
            }
            if current_source != Some(group.source()) {
                current_source = Some(group.source());
                output.push_str(&format!("\n## {}\n", group.source().name()));
                if group.source() == SourceLanguage::Asl {
                    output.push_str(
                        "\nStart with the [complete ASL porting guide](ASL_PORTING.md) for lifecycle and semantic context.\n",
                    );
                }
            }
            if let Some(task) = group.task() {
                output.push_str(&format!("\n### {task}\n"));
            }
            for concept in concepts {
                let summary = crate::documentation::strip_intra_doc_links(concept.summary);
                output.push_str(&format!(
                    "\n- **{}** (*{}*): {}",
                    concept.name,
                    concept.support.label(),
                    summary
                ));
                if !concept.targets.is_empty() {
                    let targets = concept
                        .targets
                        .iter()
                        .map(|target| format!("`{}`", self.target_display(*target)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    output.push_str(&format!(" Canonical: {targets}."));
                }
                if let Some(anchor) = concept.cookbook_anchor {
                    output.push_str(&format!(" [Porting recipe](ASL_PORTING.md#{anchor})."));
                }
                output.push('\n');
            }
        }
        output
    }

    /// Renders a canonical target using the language's public type syntax.
    pub fn target_display(&self, target: MigrationTarget) -> String {
        if let MigrationTarget::StandardLibraryItem(name) = target
            && let Some(item) = self.context.standard_library().item_by_name(name)
            && let crate::stdlib::StdlibOwner::TypeConstructor(owner) = item.owner
        {
            let constructor = self.context.standard_library().type_constructor(owner);
            if constructor.syntax != crate::stdlib::TypeConstructorSyntax::Named {
                return format!(
                    "{}.{}",
                    self.context
                        .standard_library()
                        .render_type_constructor(owner),
                    item.name
                );
            }
        }
        target.display().to_owned()
    }
}

fn cookbook_anchors() -> impl Iterator<Item = String> {
    include_str!("../docs/ASL_PORTING.md")
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(markdown_anchor)
}

pub(crate) fn markdown_anchor(heading: &str) -> String {
    let mut anchor = String::new();
    let mut pending_separator = false;
    for character in heading.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if pending_separator && !anchor.is_empty() {
                anchor.push('-');
            }
            pending_separator = false;
            anchor.push(character);
        } else if character.is_whitespace() || character == '-' {
            pending_separator = true;
        }
    }
    anchor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_migration_target_and_recipe_is_canonical() {
        let errors = MigrationCatalog::default().validate();
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn checked_in_capability_index_matches_the_catalog() {
        assert_eq!(
            include_str!("../docs/MIGRATION_CAPABILITIES.md"),
            MigrationCatalog::default().capability_index_markdown()
        );
    }

    #[test]
    fn migration_navigation_is_source_first_and_hides_catalog_ids() {
        let catalog = MigrationCatalog::default();
        let markdown = catalog.capability_index_markdown();
        let headings = ["## ASL", "## C#", "## JavaScript", "## Rust"];
        let mut previous = 0;
        for heading in headings {
            let offset = markdown
                .find(heading)
                .unwrap_or_else(|| panic!("missing source heading `{heading}`"));
            assert!(offset >= previous, "source headings are out of order");
            previous = offset;
        }
        for task in [
            "Attachment and state",
            "Process and memory",
            "Lifecycle and timer",
            "Settings",
            "Collections and text",
            "Unity and emulators",
            "Unsupported host behavior",
        ] {
            assert!(markdown.contains(&format!("### {task}")));
        }
        for concept in catalog.concepts() {
            assert!(
                !markdown.contains(&format!("`{}`", concept.id.as_str())),
                "reader-facing navigation exposed catalog id `{}`",
                concept.id.as_str()
            );
            assert!(
                MigrationNavigationGroup::ALL.contains(&migration_navigation_group(concept)),
                "migration concept `{}` has no navigation group",
                concept.id.as_str()
            );
        }
        assert!(!markdown.contains("| Foreign concept |"));
    }

    #[test]
    fn unsupported_asl_behavior_has_a_distinct_navigation_group() {
        let shutdown = MigrationCatalog::default()
            .concept(MigrationConceptId::new("asl.lifecycle.shutdown"))
            .expect("shutdown migration concept");
        assert_eq!(
            migration_navigation_group(shutdown),
            MigrationNavigationGroup::AslUnsupportedHost
        );
    }

    #[test]
    fn markdown_heading_anchor_matches_the_cookbook_links() {
        assert_eq!(
            markdown_anchor("Bounded native `stringN` state"),
            "bounded-native-stringn-state"
        );
    }
}
