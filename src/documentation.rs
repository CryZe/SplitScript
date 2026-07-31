//! Canonical generated documentation views over compiler-owned catalogs.

use crate::{
    catalog::Example,
    stdlib::{StandardLibrary, StdlibItemId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentedParameter {
    pub name: &'static str,
    pub documentation: &'static str,
}

/// One renderer-independent standard-library reference entry.
///
/// Completion, hover, signature help, machine-readable exports, and the future
/// browsable renderer consume this same generated payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardLibraryDocumentation {
    pub id: StdlibItemId,
    pub canonical_name: &'static str,
    pub signature: String,
    pub summary: &'static str,
    pub details: &'static str,
    pub parameters: Vec<DocumentedParameter>,
    pub substitutions: Vec<(String, String)>,
    pub effects: Vec<&'static str>,
    pub runtime_behavior: String,
    pub deprecation: Option<&'static str>,
    pub examples: &'static [Example],
}

impl StandardLibraryDocumentation {
    pub fn generate(id: StdlibItemId, substitutions: &[(&str, String)]) -> Self {
        Self::generate_with_library(&StandardLibrary::new(), id, substitutions)
    }

    pub fn generate_with_library(
        library: &StandardLibrary,
        id: StdlibItemId,
        substitutions: &[(&str, String)],
    ) -> Self {
        let item = library.item(id);
        Self {
            id,
            canonical_name: item.qualified_name,
            signature: library.render_signature_with(id, substitutions),
            summary: item.documentation.summary,
            details: item.documentation.details,
            parameters: item
                .signature
                .parameters
                .iter()
                .map(|parameter| DocumentedParameter {
                    name: parameter.name,
                    documentation: parameter.documentation,
                })
                .collect(),
            substitutions: substitutions
                .iter()
                .map(|(name, ty)| ((*name).to_owned(), ty.clone()))
                .collect(),
            effects: item.effects.iter().map(|effect| effect.name()).collect(),
            runtime_behavior: library.render_operation_semantics(id),
            deprecation: item.deprecation.map(|deprecation| deprecation.message),
            examples: item.documentation.examples,
        }
    }

    /// Compact documentation used beside completion candidates.
    pub fn summary_markdown(&self) -> String {
        format!("{}\n\n{}", self.summary, self.details)
    }

    /// Documentation body shared by hover and signature help.
    pub fn details_markdown(&self) -> String {
        let mut markdown = self.summary_markdown();
        if !self.substitutions.is_empty() {
            markdown.push_str("\n\n**Inferred types:** ");
            for (index, (name, ty)) in self.substitutions.iter().enumerate() {
                if index != 0 {
                    markdown.push_str(", ");
                }
                markdown.push_str(&format!("`{name} = {ty}`"));
            }
        }
        if !self.parameters.is_empty() {
            markdown.push_str("\n\n**Parameters**\n");
            for parameter in &self.parameters {
                markdown.push_str(&format!(
                    "\n- `{}` — {}",
                    parameter.name, parameter.documentation
                ));
            }
        }
        markdown.push_str("\n\n**Effects:** ");
        markdown.push_str(&self.effects.join(", "));
        markdown.push_str("\n\n**Runtime behavior:** ");
        markdown.push_str(&self.runtime_behavior);
        if let Some(deprecation) = self.deprecation {
            markdown.push_str("\n\n**Deprecated:** ");
            markdown.push_str(deprecation);
        }
        markdown
    }

    /// Full reference payload used by hover and the future browsable renderer.
    pub fn hover_markdown(&self) -> String {
        let mut markdown = format!(
            "```splitscript\n{}\n```\n\n{}",
            self.signature,
            self.details_markdown()
        );
        if !self.examples.is_empty() {
            markdown.push_str("\n\n**Examples**");
            for example in self.examples {
                markdown.push_str(&format!(
                    "\n\n_{}_\n\n```splitscript\n{}\n```",
                    example.title, example.source
                ));
            }
        }
        markdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_generic_and_resolved_views_from_one_catalog_item() {
        let generic = StandardLibraryDocumentation::generate(StdlibItemId::NumericClamp, &[]);
        let resolved = StandardLibraryDocumentation::generate(
            StdlibItemId::NumericClamp,
            &[("T", "i32".to_owned())],
        );
        assert_eq!(generic.canonical_name, resolved.canonical_name);
        assert!(generic.signature.starts_with("T.clamp"));
        assert!(resolved.signature.starts_with("i32.clamp"));
        assert_eq!(generic.summary_markdown(), resolved.summary_markdown());
        assert!(resolved.hover_markdown().contains("T = i32"));
    }
}
