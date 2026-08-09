//! Validated migration knowledge and human-readable capability reporting.

pub use splitscript_syntax::migration::{
    ForeignSpelling, ForeignSpellingContext, MigrationConcept, MigrationConceptId,
    MigrationDiagnostic, MigrationDiagnosticId, MigrationSupport, MigrationTarget, SourceLanguage,
    diagnostic as migration_diagnostic, foreign_spelling, legacy_value_path_diagnostic,
};

use crate::{CompilerContext, language::LanguageCatalog};

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

        errors
    }

    pub fn capability_index_markdown(&self) -> String {
        let mut output = String::from(
            "<!-- Generated from the compiler-owned migration catalog. -->\n\
# ASL migration capability index\n\n\
This index maps common source-language concepts to canonical SplitScript APIs and patterns. It does not redeclare the standard library.\n\n\
| Foreign concept | Source | Status | SplitScript direction |\n\
| --- | --- | --- | --- |\n",
        );
        for concept in self.concepts() {
            let sources = concept
                .sources
                .iter()
                .map(|source| source.name())
                .collect::<Vec<_>>()
                .join(", ");
            let targets = concept
                .targets
                .iter()
                .map(|target| format!("`{}`", target.display()))
                .collect::<Vec<_>>()
                .join(", ");
            let direction = if concept.targets.is_empty() {
                concept.cookbook_anchor.map_or_else(
                    || concept.summary.to_owned(),
                    |anchor| format!("{} [Recipe](ASL_PORTING.md#{anchor}).", concept.summary),
                )
            } else if let Some(anchor) = concept.cookbook_anchor {
                format!(
                    "{} Canonical targets: {}. [Recipe](ASL_PORTING.md#{anchor}).",
                    concept.summary, targets
                )
            } else {
                format!("{} Canonical targets: {}.", concept.summary, targets)
            };
            output.push_str(&format!(
                "| `{}` — {} | {} | {} | {} |\n",
                concept.id.as_str(),
                concept.name,
                sources,
                concept.support.label(),
                direction
            ));
        }
        output
    }
}

fn cookbook_anchors() -> impl Iterator<Item = String> {
    include_str!("../docs/ASL_PORTING.md")
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(markdown_anchor)
}

fn markdown_anchor(heading: &str) -> String {
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
    fn markdown_heading_anchor_matches_the_cookbook_links() {
        assert_eq!(
            markdown_anchor("Bounded native `stringN` state"),
            "bounded-native-stringn-state"
        );
    }
}
