//! Indexed ownership and lookup graph built from authored stdlib declarations.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use super::{
    CoreType, CoreTypeId, FieldVisibility, ItemKind, ItemVisibility, OperationMetadata,
    StandardBinaryOperator, StandardUnaryOperator, StdlibCapability, StdlibCapabilityId,
    StdlibField, StdlibFieldId, StdlibItem, StdlibItemId, StdlibNamespace, StdlibNamespaceId,
    StdlibOwner, StdlibStateProvider, StdlibStateProviderId, StdlibSymbolId, StdlibType,
    StdlibTypeConstructor, StdlibTypeConstructorId, StdlibTypeId, StdlibVariant, StdlibVariantId,
    TypeRef,
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
    pub(super) fields_by_owner: HashMap<StdlibOwner, Vec<&'static StdlibField>>,
    pub(super) public_fields: HashMap<(StdlibOwner, &'static str), &'static StdlibField>,
    pub(super) variants: HashMap<StdlibVariantId, &'static StdlibVariant>,
    pub(super) variants_by_owner: HashMap<StdlibTypeId, Vec<&'static StdlibVariant>>,
    pub(super) items: HashMap<StdlibItemId, &'static StdlibItem>,
    pub(super) items_by_name: HashMap<&'static str, &'static StdlibItem>,
    pub(super) all_items_by_name: HashMap<&'static str, &'static StdlibItem>,
    pub(super) methods: Vec<&'static StdlibItem>,
    pub(super) methods_by_name: HashMap<&'static str, Vec<&'static StdlibItem>>,
    pub(super) all_methods_by_name: HashMap<&'static str, Vec<&'static StdlibItem>>,
    pub(super) binary_operators: HashMap<StandardBinaryOperator, Vec<&'static StdlibItem>>,
    pub(super) unary_operators: HashMap<StandardUnaryOperator, Vec<&'static StdlibItem>>,
    pub(super) children_by_owner: HashMap<StdlibOwner, Vec<StdlibSymbolId>>,
    source_body_operations: OnceLock<HashMap<StdlibItemId, OperationMetadata>>,
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
        let all_items_by_name = index(
            ITEMS,
            |value| value.qualified_name,
            "standard item name",
            &mut errors,
        );
        let items_by_name = index(
            ITEMS
                .iter()
                .filter(|item| item.visibility == ItemVisibility::Public),
            |value| value.qualified_name,
            "public standard item name",
            &mut errors,
        );

        let fields_by_owner = group(FIELDS, |field| field.owner);
        let variants_by_owner = group(VARIANTS, |variant| variant.owner);
        let methods = ITEMS
            .iter()
            .filter(|item| {
                item.visibility == ItemVisibility::Public
                    && matches!(item.kind, ItemKind::Method { .. })
            })
            .collect();
        let methods_by_name = group(
            ITEMS.iter().filter(|item| {
                item.visibility == ItemVisibility::Public
                    && matches!(item.kind, ItemKind::Method { .. })
            }),
            |item| item.name,
        );
        let all_methods_by_name = group(
            ITEMS
                .iter()
                .filter(|item| matches!(item.kind, ItemKind::Method { .. })),
            |item| item.name,
        );
        let binary_operators = group(
            ITEMS.iter().filter(|item| item.binary_operator.is_some()),
            |item| item.binary_operator.expect("filtered operator binding"),
        );
        let unary_operators = group(
            ITEMS.iter().filter(|item| item.unary_operator.is_some()),
            |item| {
                item.unary_operator
                    .expect("filtered unary operator binding")
            },
        );
        let mut binary_operator_bindings = HashMap::new();
        for item in ITEMS.iter().filter(|item| item.binary_operator.is_some()) {
            let operator = item.binary_operator.expect("filtered operator binding");
            let ItemKind::Method { receiver } = item.kind else {
                errors.push(format!(
                    "operator implementation `{}` is not a method",
                    item.qualified_name
                ));
                continue;
            };
            let expected_result = match operator {
                StandardBinaryOperator::Add
                | StandardBinaryOperator::Subtract
                | StandardBinaryOperator::Multiply
                | StandardBinaryOperator::Divide
                | StandardBinaryOperator::Remainder
                | StandardBinaryOperator::BitOr
                | StandardBinaryOperator::BitXor
                | StandardBinaryOperator::BitAnd
                | StandardBinaryOperator::ShiftLeft
                | StandardBinaryOperator::ShiftRight => receiver,
                StandardBinaryOperator::Equal
                | StandardBinaryOperator::NotEqual
                | StandardBinaryOperator::LessThan
                | StandardBinaryOperator::LessThanOrEqual
                | StandardBinaryOperator::GreaterThan
                | StandardBinaryOperator::GreaterThanOrEqual => TypeRef::Core(CoreTypeId::Bool),
            };
            if item.signature.parameters.len() != 1 || item.signature.result != expected_result {
                errors.push(format!(
                    "operator implementation `{}` has an invalid binary signature",
                    item.qualified_name
                ));
            }
            if let Some(previous) = binary_operator_bindings.insert((item.owner, operator), item) {
                errors.push(format!(
                    "operator implementations `{}` and `{}` have the same owner and operator",
                    previous.qualified_name, item.qualified_name
                ));
            }
        }
        let mut unary_operator_bindings = HashMap::new();
        for item in ITEMS.iter().filter(|item| item.unary_operator.is_some()) {
            let operator = item
                .unary_operator
                .expect("filtered unary operator binding");
            let ItemKind::Method { receiver } = item.kind else {
                errors.push(format!(
                    "unary operator implementation `{}` is not a method",
                    item.qualified_name
                ));
                continue;
            };
            let expected_result = receiver;
            if !item.signature.parameters.is_empty() || item.signature.result != expected_result {
                errors.push(format!(
                    "operator implementation `{}` has an invalid unary signature",
                    item.qualified_name
                ));
            }
            if let Some(previous) = unary_operator_bindings.insert((item.owner, operator), item) {
                errors.push(format!(
                    "operator implementations `{}` and `{}` have the same owner and operator",
                    previous.qualified_name, item.qualified_name
                ));
            }
        }

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
            all_items_by_name,
            methods,
            methods_by_name,
            all_methods_by_name,
            binary_operators,
            unary_operators,
            children_by_owner: HashMap::new(),
            source_body_operations: OnceLock::new(),
        };
        graph.validate_references_and_index_ownership(&mut errors);
        errors.is_empty().then_some(graph).ok_or(errors)
    }

    pub(super) fn source_body_operation(&self, item: StdlibItemId) -> Option<OperationMetadata> {
        self.source_body_operations
            .get()
            .and_then(|operations| operations.get(&item).copied())
    }

    pub(super) fn initialize_source_body_operations_with(
        &self,
        initialize: impl FnOnce() -> HashMap<StdlibItemId, OperationMetadata>,
    ) {
        self.source_body_operations.get_or_init(initialize);
    }

    pub(super) fn source_body_operations_are_initialized(&self) -> bool {
        self.source_body_operations.get().is_some()
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
            if let Some(display) = ty.display {
                match self.items.get(&display).copied() {
                    Some(item)
                        if item.owner == StdlibOwner::Type(ty.id)
                            && matches!(
                                item.kind,
                                ItemKind::Method {
                                    receiver: TypeRef::Standard(receiver)
                                } if receiver == ty.id
                            )
                            && item.signature.parameters.is_empty()
                            && item.signature.result == TypeRef::Standard(StdlibTypeId::String)
                            && ty.capabilities.contains(&StdlibCapabilityId::Display) => {}
                    Some(item) => errors.push(format!(
                        "type `{}` has invalid display implementation `{}`",
                        ty.name, item.qualified_name
                    )),
                    None => errors.push(format!(
                        "type `{}` references missing display implementation `{:?}`",
                        ty.name, display
                    )),
                }
            }
            self.push_child(StdlibOwner::Root, StdlibSymbolId::Type(ty.id));
        }
        for field in FIELDS {
            if !self.owner_exists(field.owner)
                || !matches!(
                    field.owner,
                    StdlibOwner::Type(_) | StdlibOwner::TypeConstructor(_)
                )
            {
                errors.push(format!(
                    "field `{:?}` has missing owner `{:?}`",
                    field.id, field.owner
                ));
            }
            self.push_child(field.owner, StdlibSymbolId::Field(field.id));
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
            if item.visibility == ItemVisibility::Public {
                self.push_child(item.owner, StdlibSymbolId::Item(item.id));
            }
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[test]
    fn source_body_operation_initialization_runs_once_across_threads() {
        let graph = Arc::new(StandardLibraryGraph::build().expect("bundled graph is valid"));
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let graph = Arc::clone(&graph);
                let calls = &calls;
                scope.spawn(move || {
                    graph.initialize_source_body_operations_with(|| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        HashMap::new()
                    });
                });
            }
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(graph.source_body_operations_are_initialized());
    }
}
