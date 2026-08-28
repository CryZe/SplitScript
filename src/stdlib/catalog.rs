//! Generated normalized standard-library catalog.
//!
//! The public surface is authored in `stdlib/standard.split`. This module keeps
//! only Rust-side construction helpers and compiler-checked example fixtures;
//! the build script includes the final typed declaration arrays generated from
//! that source.

use crate::catalog::{Documentation, Example};

use super::{
    declarations::{
        CapabilityBehavior, CoreTypeId, FieldVisibility, ManagedRuntimeBackend,
        RuntimeRepresentation, StateProviderAttachment, StateProviderContext,
        StateProviderProcesses, StateProviderSelector, StateProviderSelectorParameter,
        StdlibAssociatedType, StdlibAssociatedTypeDefinition, StdlibCapability, StdlibField,
        StdlibNamespace, StdlibOwner, StdlibStateProvider, StdlibType, StdlibTypeConstructor,
        StdlibTypeKind, StdlibVariant, TypeConstructorSyntax, TypeVisibility, ValueUsage,
    },
    ids::{
        IntrinsicId, StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId,
        StdlibStateProviderId, StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
    },
    schema::{
        Availability, CancellationKind, Implementation, IntrinsicContext, ItemKind, ItemVisibility,
        LibraryOverloadCase, Parameter, ParameterRule, Signature, StandardBinaryOperator,
        StandardUnaryOperator, StdlibItem, TypeParameter, TypeRef,
    },
};

const fn parameter(name: &'static str, ty: TypeRef, documentation: &'static str) -> Parameter {
    Parameter {
        name,
        ty,
        rule: ParameterRule::Value,
        documentation,
    }
}

const fn literal_parameter(
    name: &'static str,
    ty: TypeRef,
    rule: ParameterRule,
    documentation: &'static str,
) -> Parameter {
    Parameter {
        name,
        ty,
        rule,
        documentation,
    }
}

include!(concat!(env!("OUT_DIR"), "/stdlib_catalog.rs"));

#[cfg(test)]
mod tests {
    use crate::stdlib::{StandardLibrary, StdlibSymbolId};

    use super::*;

    #[test]
    fn hierarchical_declarations_generate_the_complete_owner_graph() {
        let library = StandardLibrary::new();

        let unity_fields = library
            .fields_of(StdlibTypeId::UnityClass)
            .map(|field| (field.name, field.visibility))
            .collect::<Vec<_>>();
        assert_eq!(
            unity_fields,
            vec![
                ("address", FieldVisibility::RuntimePrivate),
                ("module", FieldVisibility::RuntimePrivate),
            ]
        );

        let unity_methods = library
            .children_of(StdlibOwner::Type(StdlibTypeId::UnityClass))
            .filter_map(|symbol| match symbol {
                StdlibSymbolId::Item(item) => Some(library.item(item)),
                _ => None,
            })
            .map(|item| (item.name, item.qualified_name))
            .collect::<Vec<_>>();
        assert!(unity_methods.is_empty());

        assert_eq!(
            library
                .variants_of(StdlibTypeId::TimerState)
                .map(|variant| variant.name)
                .collect::<Vec<_>>(),
            vec!["NotRunning", "Running", "Paused", "Ended", "Unknown"]
        );
        assert_eq!(
            library.item(StdlibItemId::DurationFromSeconds).owner,
            StdlibOwner::Type(StdlibTypeId::Duration)
        );
        assert_eq!(
            library.item(StdlibItemId::NumericClamp).owner,
            StdlibOwner::Capability(StdlibCapabilityId::Numeric)
        );
        assert!(
            library
                .children_of(StdlibOwner::Type(StdlibTypeId::Process))
                .any(|child| child == StdlibSymbolId::Item(StdlibItemId::ProcessRead))
        );
    }

    #[test]
    fn the_retired_parallel_authoring_registries_do_not_return() {
        let parent = include_str!("../stdlib.rs");
        let declarations = include_str!("declarations.rs");
        for retired in [
            "declare_standard_types!",
            "declare_standard_namespaces!",
            "declare_standard_fields!",
            "declare_standard_variants!",
            "declare_standard_items!",
        ] {
            assert!(
                !parent.contains(retired),
                "found retired registry `{retired}`"
            );
            assert!(
                !declarations.contains(retired),
                "found retired registry `{retired}`"
            );
        }
    }

    #[test]
    fn source_generated_ids_schema_and_normalized_data_have_one_way_dependencies() {
        let source = include_str!("../../stdlib/standard.split");
        let ids = include_str!("ids.rs");
        let build = include_str!("../../build.rs");
        let generator = include_str!("../../crates/splitscript-stdlib-loader/src/generate.rs");
        let schema = include_str!("schema.rs");
        let declarations = include_str!("declarations.rs");
        let catalog = include_str!("catalog.rs");

        assert!(source.contains("stateProvider GBA as gba"));
        assert!(source.contains("intrinsic type String"));
        assert!(ids.contains("/stdlib_ids.rs"));
        assert!(!ids.contains("with_standard_library!("));
        assert!(build.contains("generate_ids"));
        assert!(build.contains("generate_catalog"));
        assert!(catalog.contains("/stdlib_catalog.rs"));
        assert!(generator.contains("pub fn generate_catalog"));
        let retired_macro = ["macro_rules! ", "standard_library"].concat();
        let retired_invocation = ["standard_", "library!"].concat();
        assert!(!catalog.contains(&retired_macro));
        assert!(!generator.contains(&retired_invocation));
        assert!(!ids.contains("super::declarations"));
        assert!(!ids.contains("super::catalog"));
        assert!(!schema.contains("super::catalog"));
        assert!(!schema.contains("super::graph"));
        assert!(!declarations.contains("super::catalog"));
        assert!(!declarations.contains("super::graph"));
        let retired_closed_type_id = ["pub enum ", "StdlibTypeId"].concat();
        assert!(!catalog.contains(&retired_closed_type_id));
        assert!(ids.contains("pub struct $name(u32)"));
    }

    #[test]
    fn every_callable_is_authored_once_in_privileged_source() {
        let source = include_str!("../../stdlib/standard.split");

        for item in ITEMS {
            match item.implementation {
                Implementation::Intrinsic(intrinsic) => {
                    let annotation = format!("@intrinsic({})", intrinsic.name());
                    assert_eq!(
                        source.matches(&annotation).count(),
                        1,
                        "`{}` must have exactly one source declaration",
                        item.name
                    );
                }
                Implementation::LibraryBody { .. } | Implementation::LibraryOverloads { .. } => {
                    let authored = if item.kind == ItemKind::Constant {
                        source.contains(&format!("const {}:", item.name))
                    } else {
                        let declaration = format!("fn {}", item.name);
                        source.match_indices(&declaration).any(|(position, _)| {
                            matches!(
                                source.as_bytes().get(position + declaration.len()),
                                Some(b'(' | b'<')
                            )
                        })
                    };
                    assert!(
                        authored,
                        "`{}` must have a source body",
                        item.qualified_name
                    );
                }
                Implementation::CapabilityRequirement => {
                    let declaration = format!("fn {}", item.name);
                    assert!(
                        source.contains(&declaration),
                        "capability requirement `{}` must have a source declaration",
                        item.qualified_name
                    );
                }
            }
        }
    }

    #[test]
    fn declaration_type_expressions_use_catalog_constructor_identities() {
        let library = StandardLibrary::new();

        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Array)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Option)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Result)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Set)
                .parameters[0]
                .constraints,
            [StdlibCapabilityId::Equatable]
        );
        assert_eq!(
            library.render_signature(StdlibItemId::ProcessRead),
            "Process.read<T>(address: address) -> T! where T: MemoryReadable"
        );
        assert!(library.validate().is_empty());
    }

    #[test]
    fn capability_bounds_and_behavior_are_catalog_facts() {
        let library = StandardLibrary::new();

        assert_eq!(
            library
                .item(StdlibItemId::NumericMin)
                .signature
                .type_parameters[0]
                .constraints,
            [StdlibCapabilityId::Numeric]
        );
        assert_eq!(
            library.capability(StdlibCapabilityId::Equatable).behavior,
            CapabilityBehavior::StructuralEquality
        );
        assert_eq!(
            library
                .capability(StdlibCapabilityId::MemoryReadable)
                .behavior,
            CapabilityBehavior::StructuralMemoryLayout
        );
        assert_eq!(
            library.capability(StdlibCapabilityId::Display).behavior,
            CapabilityBehavior::StructuralMethods
        );
        assert_eq!(
            library.item(StdlibItemId::DisplayToString).implementation,
            Implementation::CapabilityRequirement
        );
        assert_eq!(
            library.capability(StdlibCapabilityId::Numeric).behavior,
            CapabilityBehavior::Declared
        );
        assert_eq!(
            library
                .capability(StdlibCapabilityId::Numeric)
                .super_capabilities,
            [StdlibCapabilityId::Equatable]
        );
        assert_eq!(
            library
                .capability(StdlibCapabilityId::Integer)
                .super_capabilities,
            [StdlibCapabilityId::Numeric, StdlibCapabilityId::Display]
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Numeric)
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Display)
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Equatable)
        );
        assert!(library.capability_implies(StdlibCapabilityId::Float, StdlibCapabilityId::Numeric));
        assert!(library.capability_implies(StdlibCapabilityId::Float, StdlibCapabilityId::Display));
        assert_eq!(
            library.minimal_capabilities(&[
                StdlibCapabilityId::Integer,
                StdlibCapabilityId::Numeric,
                StdlibCapabilityId::Equatable,
                StdlibCapabilityId::Display,
            ]),
            [StdlibCapabilityId::Integer]
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Numeric)
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Display)
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Equatable)
        );
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Numeric));
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Display));
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Equatable));
        assert!(library.core_type_has_capability(CoreTypeId::Bool, StdlibCapabilityId::Display));

        let schema = include_str!("schema.rs");
        let retired_constraint = ["enum Type", "Constraint"].concat();
        assert!(!schema.contains(&retired_constraint));
    }
}
