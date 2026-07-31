//! Indexed ownership and lookup graph built from authored stdlib declarations.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use super::{
    CoreType, CoreTypeId, FieldVisibility, ItemKind, StdlibCapability, StdlibCapabilityId,
    StdlibField, StdlibFieldId, StdlibItem, StdlibItemId, StdlibNamespace, StdlibNamespaceId,
    StdlibOwner, StdlibStateProvider, StdlibStateProviderId, StdlibSymbolId, StdlibType,
    StdlibTypeConstructor, StdlibTypeConstructorId, StdlibTypeId, StdlibVariant, StdlibVariantId,
    catalog::{
        CAPABILITIES, FIELDS, ITEMS, NAMESPACES, STATE_PROVIDERS, TYPE_CONSTRUCTORS, TYPES,
        VARIANTS,
    },
    declarations::CORE_TYPES,
};

/// Structurally validated storage behind the public `StandardLibrary` handle.
/// Flat declarations remain stable iteration views, while identity, paths,
/// ownership, and member lookup are indexed here exactly once.
#[derive(Debug)]
pub(super) struct StandardLibraryGraph {
    pub(super) core_types: HashMap<CoreTypeId, &'static CoreType>,
    pub(super) state_providers: HashMap<StdlibStateProviderId, &'static StdlibStateProvider>,
    pub(super) state_providers_by_name: HashMap<&'static str, &'static StdlibStateProvider>,
    pub(super) capabilities: HashMap<StdlibCapabilityId, &'static StdlibCapability>,
    pub(super) type_constructors: HashMap<StdlibTypeConstructorId, &'static StdlibTypeConstructor>,
    pub(super) namespaces: HashMap<StdlibNamespaceId, &'static StdlibNamespace>,
    pub(super) namespaces_by_name: HashMap<&'static str, &'static StdlibNamespace>,
    pub(super) namespaces_by_path: HashMap<Vec<&'static str>, &'static StdlibNamespace>,
    pub(super) types: HashMap<StdlibTypeId, &'static StdlibType>,
    pub(super) types_by_name: HashMap<&'static str, &'static StdlibType>,
    pub(super) fields: HashMap<StdlibFieldId, &'static StdlibField>,
    pub(super) fields_by_owner: HashMap<StdlibTypeId, Vec<&'static StdlibField>>,
    pub(super) public_fields: HashMap<(StdlibTypeId, &'static str), &'static StdlibField>,
    pub(super) variants: HashMap<StdlibVariantId, &'static StdlibVariant>,
    pub(super) variants_by_owner: HashMap<StdlibTypeId, Vec<&'static StdlibVariant>>,
    pub(super) items: HashMap<StdlibItemId, &'static StdlibItem>,
    pub(super) items_by_name: HashMap<&'static str, &'static StdlibItem>,
    pub(super) methods: Vec<&'static StdlibItem>,
    pub(super) methods_by_name: HashMap<&'static str, Vec<&'static StdlibItem>>,
    pub(super) children_by_owner: HashMap<StdlibOwner, Vec<StdlibSymbolId>>,
}

impl StandardLibraryGraph {
    pub(super) fn build() -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let core_types = index(CORE_TYPES, |value| value.id, "core type ID", &mut errors);
        let state_providers = index(
            STATE_PROVIDERS,
            |value| value.id,
            "state provider ID",
            &mut errors,
        );
        let state_providers_by_name = index(
            STATE_PROVIDERS,
            |value| value.name,
            "state provider name",
            &mut errors,
        );
        let capabilities = index(CAPABILITIES, |value| value.id, "capability ID", &mut errors);
        let type_constructors = index(
            TYPE_CONSTRUCTORS,
            |value| value.id,
            "type-constructor ID",
            &mut errors,
        );
        let namespaces = index(NAMESPACES, |value| value.id, "namespace ID", &mut errors);
        let namespaces_by_name = index(
            NAMESPACES
                .iter()
                .filter(|namespace| namespace.path.len() == 1),
            |value| value.name,
            "root namespace name",
            &mut errors,
        );
        let namespaces_by_path = index(
            NAMESPACES,
            |value| value.path.to_vec(),
            "namespace path",
            &mut errors,
        );
        for namespace in NAMESPACES {
            if namespace.path.is_empty() {
                errors.push(format!(
                    "namespace `{:?}` has an empty source path",
                    namespace.id
                ));
            } else if namespace.path.last().copied() != Some(namespace.name) {
                errors.push(format!(
                    "namespace `{:?}` has name `{}` but path `{}`",
                    namespace.id,
                    namespace.name,
                    namespace.path.join(".")
                ));
            }
        }

        let types = index(TYPES, |value| value.id, "standard type ID", &mut errors);
        let types_by_name = index(TYPES, |value| value.name, "standard type name", &mut errors);
        let fields = index(FIELDS, |value| value.id, "standard field ID", &mut errors);
        let public_fields = index(
            FIELDS
                .iter()
                .filter(|field| field.visibility == FieldVisibility::Public),
            |value| (value.owner, value.name),
            "public field owner/name",
            &mut errors,
        );
        let variants = index(
            VARIANTS,
            |value| value.id,
            "standard variant ID",
            &mut errors,
        );
        let items = index(ITEMS, |value| value.id, "standard item ID", &mut errors);
        let items_by_name = index(
            ITEMS,
            |value| value.qualified_name,
            "standard item name",
            &mut errors,
        );

        let fields_by_owner = group(FIELDS, |field| field.owner);
        let variants_by_owner = group(VARIANTS, |variant| variant.owner);
        let methods = ITEMS
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::Method { .. }))
            .collect();
        let methods_by_name = group(
            ITEMS
                .iter()
                .filter(|item| matches!(item.kind, ItemKind::Method { .. })),
            |item| item.name,
        );

        let mut graph = Self {
            core_types,
            state_providers,
            state_providers_by_name,
            capabilities,
            type_constructors,
            namespaces,
            namespaces_by_name,
            namespaces_by_path,
            types,
            types_by_name,
            fields,
            fields_by_owner,
            public_fields,
            variants,
            variants_by_owner,
            items,
            items_by_name,
            methods,
            methods_by_name,
            children_by_owner: HashMap::new(),
        };
        graph.validate_references_and_index_ownership(&mut errors);
        errors.is_empty().then_some(graph).ok_or(errors)
    }

    fn validate_references_and_index_ownership(&mut self, errors: &mut Vec<String>) {
        for namespace in NAMESPACES {
            let owner = if namespace.path.len() == 1 {
                Some(StdlibOwner::Root)
            } else {
                self.namespaces_by_path
                    .get(&namespace.path[..namespace.path.len().saturating_sub(1)])
                    .map(|parent| StdlibOwner::Namespace(parent.id))
            };
            if let Some(owner) = owner {
                self.push_child(owner, StdlibSymbolId::Namespace(namespace.id));
            } else {
                errors.push(format!(
                    "namespace `{}` has no declared parent namespace",
                    namespace.path.join(".")
                ));
            }
        }
        for capability in CAPABILITIES {
            self.push_child(StdlibOwner::Root, StdlibSymbolId::Capability(capability.id));
        }
        for provider in STATE_PROVIDERS {
            if !self.types.contains_key(&provider.process_type) {
                errors.push(format!(
                    "state provider `{}` has missing process type `{:?}`",
                    provider.name, provider.process_type
                ));
            }
            self.push_child(
                StdlibOwner::Root,
                StdlibSymbolId::StateProvider(provider.id),
            );
        }
        for constructor in TYPE_CONSTRUCTORS {
            self.push_child(
                StdlibOwner::Root,
                StdlibSymbolId::TypeConstructor(constructor.id),
            );
        }
        for ty in TYPES {
            self.push_child(StdlibOwner::Root, StdlibSymbolId::Type(ty.id));
        }
        for field in FIELDS {
            if !self.types.contains_key(&field.owner) {
                errors.push(format!(
                    "field `{:?}` has missing owner `{:?}`",
                    field.id, field.owner
                ));
            }
            self.push_child(
                StdlibOwner::Type(field.owner),
                StdlibSymbolId::Field(field.id),
            );
        }
        for variant in VARIANTS {
            if !self.types.contains_key(&variant.owner) {
                errors.push(format!(
                    "variant `{:?}` has missing owner `{:?}`",
                    variant.id, variant.owner
                ));
            }
            self.push_child(
                StdlibOwner::Type(variant.owner),
                StdlibSymbolId::Variant(variant.id),
            );
        }
        for item in ITEMS {
            if !self.owner_exists(item.owner) {
                errors.push(format!(
                    "item `{}` has missing owner `{:?}`",
                    item.qualified_name, item.owner
                ));
            }
            self.push_child(item.owner, StdlibSymbolId::Item(item.id));
        }
    }

    fn owner_exists(&self, owner: StdlibOwner) -> bool {
        match owner {
            StdlibOwner::Root => true,
            StdlibOwner::Namespace(id) => self.namespaces.contains_key(&id),
            StdlibOwner::Type(id) => self.types.contains_key(&id),
            StdlibOwner::Core(id) => self.core_types.contains_key(&id),
            StdlibOwner::Capability(id) => self.capabilities.contains_key(&id),
            StdlibOwner::TypeConstructor(id) => self.type_constructors.contains_key(&id),
        }
    }

    fn push_child(&mut self, owner: StdlibOwner, child: StdlibSymbolId) {
        self.children_by_owner.entry(owner).or_default().push(child);
    }
}

fn index<K, V: 'static>(
    values: impl IntoIterator<Item = &'static V>,
    key: impl Fn(&V) -> K,
    description: &str,
    errors: &mut Vec<String>,
) -> HashMap<K, &'static V>
where
    K: std::fmt::Debug + Eq + std::hash::Hash,
{
    let mut result = HashMap::new();
    for value in values {
        let key = key(value);
        let rendered = format!("{key:?}");
        if result.insert(key, value).is_some() {
            errors.push(format!("duplicate {description} `{rendered}`"));
        }
    }
    result
}

fn group<K, V: 'static>(
    values: impl IntoIterator<Item = &'static V>,
    key: impl Fn(&V) -> K,
) -> HashMap<K, Vec<&'static V>>
where
    K: Eq + std::hash::Hash,
{
    let mut result = HashMap::<_, Vec<_>>::new();
    for value in values {
        result.entry(key(value)).or_default().push(value);
    }
    result
}

static DEFAULT_STANDARD_LIBRARY: OnceLock<Arc<StandardLibraryGraph>> = OnceLock::new();

pub(super) fn default_standard_library_graph() -> Arc<StandardLibraryGraph> {
    DEFAULT_STANDARD_LIBRARY
        .get_or_init(|| {
            Arc::new(StandardLibraryGraph::build().unwrap_or_else(|errors| {
                panic!(
                    "the bundled standard-library graph is invalid:\n{}",
                    errors.join("\n")
                )
            }))
        })
        .clone()
}
