//! Validated migration knowledge and human-readable capability reporting.

pub use splitscript_syntax::migration::{
    ASL_SETTINGS_ADD_DIAGNOSTIC, ForeignSpelling, ForeignSpellingContext,
    ForeignSpellingReplacement, MigrationConcept, MigrationConceptId, MigrationDiagnostic,
    MigrationDiagnosticId, MigrationSupport, MigrationTarget, SourceLanguage,
    diagnostic as migration_diagnostic, foreign_spelling, legacy_array_field_diagnostic,
    legacy_set_field_diagnostic, legacy_static_call_diagnostic, legacy_string_field_diagnostic,
    legacy_string_method_diagnostic, legacy_type_diagnostic, legacy_value_path_diagnostic,
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
            let summary = crate::documentation::strip_intra_doc_links(concept.summary);
            let sources = concept
                .sources
                .iter()
                .map(|source| source.name())
                .collect::<Vec<_>>()
                .join(", ");
            let targets = concept
                .targets
                .iter()
                .map(|target| format!("`{}`", self.target_display(*target)))
                .collect::<Vec<_>>()
                .join(", ");
            let direction = if concept.targets.is_empty() {
                concept.cookbook_anchor.map_or_else(
                    || summary.clone(),
                    |anchor| format!("{summary} [Recipe](ASL_PORTING.md#{anchor})."),
                )
            } else if let Some(anchor) = concept.cookbook_anchor {
                format!(
                    "{} Canonical targets: {}. [Recipe](ASL_PORTING.md#{anchor}).",
                    summary, targets
                )
            } else {
                format!("{} Canonical targets: {}.", summary, targets)
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
    fn markdown_heading_anchor_matches_the_cookbook_links() {
        assert_eq!(
            markdown_anchor("Bounded native `stringN` state"),
            "bounded-native-stringn-state"
        );
    }
}
