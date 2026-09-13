//! Build-time catalog generation for SplitScript's privileged standard-library
//! source.
//!
//! Syntax and parsing live in `splitscript-syntax`; this crate only translates
//! the parsed declaration tree into Rust catalog data and stable identities.

mod generate;
mod validation;

pub use generate::{generate_catalog, generate_ids};
pub use splitscript_syntax::standard_library::*;

use std::collections::HashSet;

/// Returns the associated types visible inside a capability declaration,
/// together with the capability that originally declares each one.
///
/// Capabilities have an implicit receiver and inherit associated type names
/// through their super-capabilities. Keeping that traversal here gives
/// validation and catalog emission one definition of the capability scope.
pub(crate) fn visible_capability_associated_types<'a>(
    library: &'a Library,
    capability: &'a CallableOwnerDeclaration,
) -> Vec<(&'a str, &'a AssociatedTypeDeclaration)> {
    fn collect<'a>(
        library: &'a Library,
        capability: &'a CallableOwnerDeclaration,
        visited: &mut HashSet<&'a str>,
        output: &mut Vec<(&'a str, &'a AssociatedTypeDeclaration)>,
    ) {
        if !visited.insert(capability.name.as_str()) {
            return;
        }
        output.extend(
            capability
                .associated_types
                .iter()
                .map(|associated| (capability.name.as_str(), associated)),
        );
        for super_capability in capability
            .type_parameters
            .first()
            .into_iter()
            .flat_map(|receiver| &receiver.constraints)
        {
            let Some(declaration) =
                library
                    .declarations
                    .iter()
                    .find_map(|declaration| match declaration {
                        Declaration::Capability(candidate)
                            if candidate.name == *super_capability =>
                        {
                            Some(candidate)
                        }
                        _ => None,
                    })
            else {
                continue;
            };
            collect(library, declaration, visited, output);
        }
    }

    let mut output = Vec::new();
    collect(library, capability, &mut HashSet::new(), &mut output);
    output
}
