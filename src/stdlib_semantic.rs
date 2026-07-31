//! Compiler-semantic adapters for the backend-neutral standard-library graph.
//!
//! The catalog schema and graph deliberately do not depend on inference or
//! semantic `TypeKind` values. This module is the one-way adapter from those
//! compiler types into catalog candidate and applicability queries.

use crate::{
    stdlib::{
        CapabilityBehavior, ItemKind, StandardLibrary, StdlibCapabilityId, StdlibItem,
        StdlibTypeConstructorId, TypeRef,
    },
    types::{BuiltinType, TypeKind},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCandidate {
    pub item: &'static StdlibItem,
    pub type_arguments: Vec<(&'static str, BuiltinType)>,
}

impl CallCandidate {
    pub const fn receiver(&self) -> Option<TypeRef> {
        match self.item.kind {
            ItemKind::Method { receiver } => Some(receiver),
            ItemKind::Function | ItemKind::TypedFunction { .. } => None,
        }
    }
}

/// Compiler-specific queries layered over the backend-neutral catalog graph.
pub trait StandardLibrarySemanticExt {
    fn function_candidates(&self, path: &[String]) -> Vec<CallCandidate>;
    fn method_candidates(&self, name: &str) -> Vec<CallCandidate>;
    fn methods_for_type(&self, receiver: &TypeKind) -> Vec<&'static StdlibItem>;
    fn resolve_path(&self, path: &[String]) -> Option<CallCandidate>;
}

impl StandardLibrarySemanticExt for StandardLibrary {
    fn function_candidates(&self, path: &[String]) -> Vec<CallCandidate> {
        let qualified_name = path.join(".");
        if let Some(item) = self.item_by_name(&qualified_name) {
            return match item.kind {
                ItemKind::Function | ItemKind::TypedFunction { .. } => vec![CallCandidate {
                    item,
                    type_arguments: Vec::new(),
                }],
                ItemKind::Method { .. } => Vec::new(),
            };
        }

        let Some((type_name, item_path)) = path.split_last() else {
            return Vec::new();
        };
        let Some(item) = self.item_by_name(&item_path.join(".")) else {
            return Vec::new();
        };
        let ItemKind::TypedFunction { type_parameter } = item.kind else {
            return Vec::new();
        };
        let Some(argument) = memory_type(self, type_name) else {
            return Vec::new();
        };
        vec![CallCandidate {
            item,
            type_arguments: vec![(type_parameter, argument)],
        }]
    }

    fn method_candidates(&self, name: &str) -> Vec<CallCandidate> {
        self.method_items_named(name)
            .map(|item| CallCandidate {
                item,
                type_arguments: Vec::new(),
            })
            .collect()
    }

    fn methods_for_type(&self, receiver: &TypeKind) -> Vec<&'static StdlibItem> {
        self.methods()
            .filter(|item| catalog_method_accepts(self, item, receiver))
            .collect()
    }

    fn resolve_path(&self, path: &[String]) -> Option<CallCandidate> {
        self.function_candidates(path).into_iter().next()
    }
}

fn catalog_method_accepts(
    library: &StandardLibrary,
    item: &StdlibItem,
    receiver: &TypeKind,
) -> bool {
    let ItemKind::Method { receiver: declared } = item.kind else {
        return false;
    };
    match declared {
        TypeRef::Core(expected) => {
            matches!(receiver, TypeKind::Builtin(actual) if *actual == expected)
        }
        TypeRef::Application { constructor, .. } => {
            (constructor == StdlibTypeConstructorId::Array
                && matches!(receiver, TypeKind::Array { .. }))
                || (constructor == StdlibTypeConstructorId::Option
                    && matches!(receiver, TypeKind::Option { .. }))
                || (constructor == StdlibTypeConstructorId::Result
                    && matches!(receiver, TypeKind::Result { .. }))
        }
        TypeRef::Standard(expected) => {
            matches!(receiver, TypeKind::Standard(actual) if *actual == expected)
        }
        TypeRef::Parameter(name) => item
            .signature
            .type_parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .is_none_or(|parameter| {
                parameter.constraints.iter().all(|constraint| {
                    semantic_type_may_have_capability(library, receiver, *constraint)
                })
            }),
    }
}

// Candidate discovery has only a TypeKind, not the declarations required to
// prove recursive capabilities. CapabilityAnalysis performs final validation.
fn semantic_type_may_have_capability(
    library: &StandardLibrary,
    ty: &TypeKind,
    capability: StdlibCapabilityId,
) -> bool {
    let behavior = library.capability(capability).behavior;
    match ty {
        TypeKind::Builtin(builtin) => library.core_type_has_capability(*builtin, capability),
        TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
        TypeKind::Record(_) => matches!(
            behavior,
            CapabilityBehavior::StructuralEquality | CapabilityBehavior::StructuralMemoryLayout
        ),
        TypeKind::Enum(_) | TypeKind::Option { .. } | TypeKind::Result { .. } => {
            behavior == CapabilityBehavior::StructuralEquality
        }
        TypeKind::Array { .. } => false,
    }
}

fn memory_type(library: &StandardLibrary, name: &str) -> Option<BuiltinType> {
    library
        .core_types()
        .iter()
        .find(|ty| {
            ty.name == name
                && library.core_type_has_capability(ty.id, StdlibCapabilityId::MemoryReadable)
        })
        .map(|ty| ty.id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_neutral_catalog_does_not_depend_on_semantic_types() {
        for (path, source) in [
            ("stdlib.rs", include_str!("stdlib.rs")),
            ("stdlib/catalog.rs", include_str!("stdlib/catalog.rs")),
            (
                "stdlib/declarations.rs",
                include_str!("stdlib/declarations.rs"),
            ),
            ("stdlib/graph.rs", include_str!("stdlib/graph.rs")),
            ("stdlib/ids.rs", include_str!("stdlib/ids.rs")),
            ("stdlib/schema.rs", include_str!("stdlib/schema.rs")),
            ("stdlib/source.rs", include_str!("stdlib/source.rs")),
            ("stdlib/validation.rs", include_str!("stdlib/validation.rs")),
        ] {
            assert!(
                !source.contains("crate::types")
                    && !source.contains("types::TypeKind")
                    && !source.contains("BuiltinType"),
                "backend-neutral catalog module `{path}` depends on compiler semantic types"
            );
        }
    }
}
