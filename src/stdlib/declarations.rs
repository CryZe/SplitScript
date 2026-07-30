//! Nominal standard-library declarations shared by semantics, code generation,
//! documentation, and editor tooling.
//!
//! This module deliberately contains no parser, inference, or WebAssembly
//! encoder types. It describes public symbols and backend-neutral runtime
//! shapes; consumers resolve those identities into their own stage-specific
//! representations.

use std::{collections::HashSet, fmt};

use crate::catalog::Documentation;

use super::catalog::{FIELDS, NAMESPACES, TYPES, VARIANTS};
use super::{
    StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId, StdlibTypeConstructorId,
    StdlibTypeId, StdlibVariantId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StdlibOwner {
    Root,
    Namespace(StdlibNamespaceId),
    Type(StdlibTypeId),
    Core(CoreTypeId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibSymbolId {
    Namespace(StdlibNamespaceId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
    Type(StdlibTypeId),
    Field(StdlibFieldId),
    Variant(StdlibVariantId),
    Item(StdlibItemId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreTypeId {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Address,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclaredTypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreType {
    pub id: CoreTypeId,
    pub name: &'static str,
    pub capabilities: &'static [StdlibCapabilityId],
    pub memory_layout: Option<ScalarMemoryLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarMemoryLayout {
    pub size: u32,
    pub alignment: u32,
}

impl fmt::Display for CoreTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            CORE_TYPES
                .iter()
                .find(|declaration| declaration.id == *self)
                .expect("every core type ID has a declaration")
                .name,
        )
    }
}

impl fmt::Display for DeclaredTypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(core) => core.fmt(formatter),
            Self::Standard(standard) => formatter.write_str(
                TYPES
                    .iter()
                    .find(|declaration| declaration.id == *standard)
                    .expect("every standard type reference has a declaration")
                    .name,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibTypeKind {
    Intrinsic,
    Struct,
    Enum,
}

/// Backend-neutral storage requirements for a standard-library type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRepresentation {
    Scalar {
        storage: CoreTypeId,
    },
    GcArray {
        element: CoreTypeId,
        mutable: bool,
        nullable: bool,
    },
    GcStruct {
        nullable: bool,
    },
    Enum {
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldVisibility {
    Public,
    RuntimePrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueUsage {
    pub record_field: bool,
    pub enum_payload: bool,
    pub state_field: bool,
    pub local_variable: bool,
    pub global_variable: bool,
}

pub(super) const ORDINARY_LOCAL_VALUE: ValueUsage = ValueUsage {
    record_field: true,
    enum_payload: true,
    state_field: true,
    local_variable: true,
    global_variable: false,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibNamespace {
    pub id: StdlibNamespaceId,
    pub name: &'static str,
    pub path: &'static [&'static str],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibCapability {
    pub id: StdlibCapabilityId,
    pub name: &'static str,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibTypeConstructor {
    pub id: StdlibTypeConstructorId,
    pub name: &'static str,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibType {
    pub id: StdlibTypeId,
    pub name: &'static str,
    pub kind: StdlibTypeKind,
    pub capabilities: &'static [StdlibCapabilityId],
    pub representation: RuntimeRepresentation,
    pub value_usage: ValueUsage,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibField {
    pub id: StdlibFieldId,
    pub owner: StdlibTypeId,
    pub name: &'static str,
    pub ty: DeclaredTypeRef,
    pub visibility: FieldVisibility,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibVariant {
    pub id: StdlibVariantId,
    pub owner: StdlibTypeId,
    pub name: &'static str,
    pub documentation: Documentation<StdlibSymbolId>,
}

pub(super) const EQUATABLE_INTERPOLATABLE: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::Interpolatable,
];
pub(super) const EQUATABLE: &[StdlibCapabilityId] = &[StdlibCapabilityId::Equatable];

const BOOL_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::MemoryReadable,
];
const SIGNED_INTEGER_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Numeric,
    StdlibCapabilityId::Integer,
    StdlibCapabilityId::Signed,
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::StringCast,
    StdlibCapabilityId::Interpolatable,
    StdlibCapabilityId::MemoryReadable,
];
const UNSIGNED_INTEGER_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Numeric,
    StdlibCapabilityId::Integer,
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::StringCast,
    StdlibCapabilityId::Interpolatable,
    StdlibCapabilityId::MemoryReadable,
];
const FLOAT_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Numeric,
    StdlibCapabilityId::Signed,
    StdlibCapabilityId::Float,
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::MemoryReadable,
];

pub(super) const CORE_TYPES: &[CoreType] = &[
    CoreType {
        id: CoreTypeId::Void,
        name: "void",
        capabilities: &[],
        memory_layout: None,
    },
    CoreType {
        id: CoreTypeId::Bool,
        name: "bool",
        capabilities: BOOL_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 1,
            alignment: 1,
        }),
    },
    CoreType {
        id: CoreTypeId::I8,
        name: "i8",
        capabilities: SIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 1,
            alignment: 1,
        }),
    },
    CoreType {
        id: CoreTypeId::U8,
        name: "u8",
        capabilities: UNSIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 1,
            alignment: 1,
        }),
    },
    CoreType {
        id: CoreTypeId::I16,
        name: "i16",
        capabilities: SIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 2,
            alignment: 2,
        }),
    },
    CoreType {
        id: CoreTypeId::U16,
        name: "u16",
        capabilities: UNSIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 2,
            alignment: 2,
        }),
    },
    CoreType {
        id: CoreTypeId::I32,
        name: "i32",
        capabilities: SIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 4,
            alignment: 4,
        }),
    },
    CoreType {
        id: CoreTypeId::U32,
        name: "u32",
        capabilities: UNSIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 4,
            alignment: 4,
        }),
    },
    CoreType {
        id: CoreTypeId::I64,
        name: "i64",
        capabilities: SIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 8,
            alignment: 8,
        }),
    },
    CoreType {
        id: CoreTypeId::U64,
        name: "u64",
        capabilities: UNSIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 8,
            alignment: 8,
        }),
    },
    CoreType {
        id: CoreTypeId::Address,
        name: "address",
        capabilities: UNSIGNED_INTEGER_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 8,
            alignment: 8,
        }),
    },
    CoreType {
        id: CoreTypeId::F32,
        name: "f32",
        capabilities: FLOAT_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 4,
            alignment: 4,
        }),
    },
    CoreType {
        id: CoreTypeId::F64,
        name: "f64",
        capabilities: FLOAT_CAPABILITIES,
        memory_layout: Some(ScalarMemoryLayout {
            size: 8,
            alignment: 8,
        }),
    },
];

pub(super) fn validate() -> Vec<String> {
    let mut errors = Vec::new();
    let mut core_type_ids = HashSet::new();
    let mut core_type_names = HashSet::new();
    for ty in CORE_TYPES {
        if !core_type_ids.insert(ty.id) {
            errors.push(format!("duplicate core type ID `{:?}`", ty.id));
        }
        if !core_type_names.insert(ty.name) {
            errors.push(format!("duplicate core type name `{}`", ty.name));
        }
        let mut capabilities = HashSet::new();
        for capability in ty.capabilities {
            if !capabilities.insert(capability) {
                errors.push(format!(
                    "core type `{}` repeats capability `{:?}`",
                    ty.name, capability
                ));
            }
        }
        let declared_readable = ty
            .capabilities
            .contains(&StdlibCapabilityId::MemoryReadable);
        if declared_readable != ty.memory_layout.is_some() {
            errors.push(format!(
                "core type `{}` must declare MemoryReadable and a memory layout together",
                ty.name
            ));
        }
        if let Some(layout) = ty.memory_layout
            && (layout.size == 0
                || !layout.alignment.is_power_of_two()
                || layout.size % layout.alignment != 0)
        {
            errors.push(format!(
                "core type `{}` has invalid process-memory size/alignment",
                ty.name
            ));
        }
    }
    let mut namespace_ids = HashSet::new();
    let mut namespace_names = HashSet::new();
    let mut namespace_paths = HashSet::new();
    for namespace in NAMESPACES {
        if !namespace_ids.insert(namespace.id) {
            errors.push(format!("duplicate namespace ID `{:?}`", namespace.id));
        }
        let parent = &namespace.path[..namespace.path.len().saturating_sub(1)];
        if !namespace_names.insert((parent, namespace.name)) {
            errors.push(format!(
                "duplicate namespace member `{}` below `{}`",
                namespace.name,
                parent.join(".")
            ));
        }
        if !namespace_paths.insert(namespace.path) {
            errors.push(format!(
                "duplicate namespace path `{}`",
                namespace.path.join(".")
            ));
        }
        if namespace.path.last().copied() != Some(namespace.name) {
            errors.push(format!(
                "namespace `{:?}` has path/name disagreement",
                namespace.id
            ));
        }
        if namespace.path.len() > 1 && !NAMESPACES.iter().any(|candidate| candidate.path == parent)
        {
            errors.push(format!(
                "namespace `{}` has missing parent `{}`",
                namespace.path.join("."),
                parent.join(".")
            ));
        }
        validate_documentation(
            &mut errors,
            "namespace",
            namespace.name,
            &namespace.documentation,
        );
    }

    let mut type_ids = HashSet::new();
    let mut type_names = HashSet::new();
    for ty in TYPES {
        if !type_ids.insert(ty.id) {
            errors.push(format!("duplicate standard type ID `{:?}`", ty.id));
        }
        if !type_names.insert(ty.name) {
            errors.push(format!("duplicate standard type name `{}`", ty.name));
        }
        let mut capabilities = HashSet::new();
        for capability in ty.capabilities {
            if !capabilities.insert(capability) {
                errors.push(format!(
                    "standard type `{}` repeats capability `{:?}`",
                    ty.name, capability
                ));
            }
        }
        validate_documentation(&mut errors, "type", ty.name, &ty.documentation);
        let has_fields = FIELDS.iter().any(|field| field.owner == ty.id);
        let has_variants = VARIANTS.iter().any(|variant| variant.owner == ty.id);
        match ty.kind {
            StdlibTypeKind::Enum if !has_variants => {
                errors.push(format!("enum `{}` has no variants", ty.name));
            }
            StdlibTypeKind::Enum if has_fields => {
                errors.push(format!("enum `{}` declares struct fields", ty.name));
            }
            StdlibTypeKind::Struct if !has_fields => {
                errors.push(format!("struct `{}` has no fields", ty.name));
            }
            StdlibTypeKind::Intrinsic | StdlibTypeKind::Struct | StdlibTypeKind::Enum => {}
        }
        let representation_core = match ty.representation {
            RuntimeRepresentation::Scalar { storage } => Some(storage),
            RuntimeRepresentation::GcArray { element, .. } => Some(element),
            RuntimeRepresentation::GcStruct { .. } | RuntimeRepresentation::Enum { .. } => None,
        };
        if let Some(core) = representation_core
            && !core_type_ids.contains(&core)
        {
            errors.push(format!(
                "standard type `{}` has a representation using missing core type `{:?}`",
                ty.name, core
            ));
        }
    }

    let mut field_ids = HashSet::new();
    let mut field_names = HashSet::new();
    for field in FIELDS {
        if !field_ids.insert(field.id) {
            errors.push(format!("duplicate standard field ID `{:?}`", field.id));
        }
        if !field_names.insert((field.owner, field.name)) {
            errors.push(format!(
                "duplicate field name `{:?}.{}`",
                field.owner, field.name
            ));
        }
        validate_documentation(&mut errors, "field", field.name, &field.documentation);
        if !type_ids.contains(&field.owner) {
            errors.push(format!(
                "field `{:?}` has missing owner `{:?}`",
                field.id, field.owner
            ));
        }
        match field.ty {
            DeclaredTypeRef::Core(referenced) if !core_type_ids.contains(&referenced) => {
                errors.push(format!(
                    "field `{:?}` references missing core type `{:?}`",
                    field.id, referenced
                ));
            }
            DeclaredTypeRef::Standard(referenced) if !type_ids.contains(&referenced) => {
                errors.push(format!(
                    "field `{:?}` references missing type `{:?}`",
                    field.id, referenced
                ));
            }
            DeclaredTypeRef::Core(_) | DeclaredTypeRef::Standard(_) => {}
        }
    }

    for ty in TYPES.iter().filter(|ty| {
        ty.capabilities
            .contains(&StdlibCapabilityId::MemoryReadable)
    }) {
        let mut visiting = HashSet::new();
        if let Err(reason) = validate_standard_memory_layout(ty.id, &mut visiting) {
            errors.push(format!(
                "standard type `{}` declares MemoryReadable but {reason}",
                ty.name
            ));
        }
    }
    for ty in TYPES
        .iter()
        .filter(|ty| ty.capabilities.contains(&StdlibCapabilityId::Equatable))
    {
        let mut visiting = HashSet::new();
        if let Err(reason) = validate_standard_equality(ty.id, &mut visiting) {
            errors.push(format!(
                "standard type `{}` declares Equatable but {reason}",
                ty.name
            ));
        }
    }

    let mut variant_ids = HashSet::new();
    let mut variant_names = HashSet::new();
    for variant in VARIANTS {
        if !variant_ids.insert(variant.id) {
            errors.push(format!("duplicate standard variant ID `{:?}`", variant.id));
        }
        if !variant_names.insert((variant.owner, variant.name)) {
            errors.push(format!(
                "duplicate variant name `{:?}.{}`",
                variant.owner, variant.name
            ));
        }
        validate_documentation(&mut errors, "variant", variant.name, &variant.documentation);
        if !type_ids.contains(&variant.owner) {
            errors.push(format!(
                "variant `{:?}` has missing owner `{:?}`",
                variant.id, variant.owner
            ));
        }
    }

    errors
}

fn validate_standard_memory_layout(
    ty: StdlibTypeId,
    visiting: &mut HashSet<StdlibTypeId>,
) -> Result<(), String> {
    let declaration = TYPES
        .iter()
        .find(|declaration| declaration.id == ty)
        .expect("validated standard type references have declarations");
    if !declaration
        .capabilities
        .contains(&StdlibCapabilityId::MemoryReadable)
    {
        return Err(format!(
            "referenced type `{}` is not MemoryReadable",
            declaration.name
        ));
    }
    if !visiting.insert(ty) {
        return Err("its process-memory representation is recursive".to_owned());
    }
    let result = match declaration.representation {
        RuntimeRepresentation::Scalar { storage } => CORE_TYPES
            .iter()
            .find(|core| core.id == storage)
            .and_then(|core| core.memory_layout)
            .map(|_| ())
            .ok_or_else(|| "its scalar storage has no fixed memory layout".to_owned()),
        RuntimeRepresentation::GcStruct { .. } => {
            let fields = FIELDS
                .iter()
                .filter(|field| field.owner == ty)
                .collect::<Vec<_>>();
            if fields.is_empty() {
                Err("it has no readable fields".to_owned())
            } else {
                fields.into_iter().try_for_each(|field| match field.ty {
                    DeclaredTypeRef::Core(core) => CORE_TYPES
                        .iter()
                        .find(|declaration| declaration.id == core)
                        .and_then(|declaration| declaration.memory_layout)
                        .map(|_| ())
                        .ok_or_else(|| {
                            format!("field `{}` has no fixed memory layout", field.name)
                        }),
                    DeclaredTypeRef::Standard(standard) => {
                        validate_standard_memory_layout(standard, visiting).map_err(|reason| {
                            format!("field `{}` is not readable because {reason}", field.name)
                        })
                    }
                })
            }
        }
        RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
            Err("its runtime representation has no fixed process-memory layout".to_owned())
        }
    };
    visiting.remove(&ty);
    result
}

fn validate_standard_equality(
    ty: StdlibTypeId,
    visiting: &mut HashSet<StdlibTypeId>,
) -> Result<(), String> {
    let declaration = TYPES
        .iter()
        .find(|declaration| declaration.id == ty)
        .expect("validated standard type references have declarations");
    if !declaration
        .capabilities
        .contains(&StdlibCapabilityId::Equatable)
    {
        return Err(format!(
            "referenced type `{}` is not Equatable",
            declaration.name
        ));
    }
    if !visiting.insert(ty) {
        return Err("its equality representation is recursive".to_owned());
    }
    let result = match declaration.representation {
        RuntimeRepresentation::Scalar { storage } => CORE_TYPES
            .iter()
            .find(|core| core.id == storage)
            .filter(|core| core.capabilities.contains(&StdlibCapabilityId::Equatable))
            .map(|_| ())
            .ok_or_else(|| "its scalar storage is not Equatable".to_owned()),
        RuntimeRepresentation::GcStruct { .. } => FIELDS
            .iter()
            .filter(|field| field.owner == ty)
            .try_for_each(|field| match field.ty {
                DeclaredTypeRef::Core(core) => CORE_TYPES
                    .iter()
                    .find(|declaration| declaration.id == core)
                    .filter(|declaration| {
                        declaration
                            .capabilities
                            .contains(&StdlibCapabilityId::Equatable)
                    })
                    .map(|_| ())
                    .ok_or_else(|| format!("field `{}` is not Equatable", field.name)),
                DeclaredTypeRef::Standard(standard) => {
                    validate_standard_equality(standard, visiting).map_err(|reason| {
                        format!("field `{}` is not Equatable because {reason}", field.name)
                    })
                }
            }),
        RuntimeRepresentation::Enum { .. } => Ok(()),
        RuntimeRepresentation::GcArray { .. } if declaration.kind == StdlibTypeKind::Intrinsic => {
            // Intrinsic aggregate equality has a deliberately scoped backend
            // implementation, as String does today.
            Ok(())
        }
        RuntimeRepresentation::GcArray { .. } => {
            Err("its GC-array equality has no intrinsic implementation".to_owned())
        }
    };
    visiting.remove(&ty);
    result
}

fn validate_documentation<Id>(
    errors: &mut Vec<String>,
    kind: &str,
    name: &str,
    documentation: &Documentation<Id>,
) {
    if documentation.summary.trim().is_empty() {
        errors.push(format!("{kind} `{name}` has no documentation summary"));
    }
    if documentation.details.trim().is_empty() {
        errors.push(format!("{kind} `{name}` has no documentation details"));
    }
}

#[cfg(test)]
mod tests {
    use wasmparser::{Validator, WasmFeatures};

    use crate::{
        completion::CompletionKind,
        database::{CompilerDatabase, DefinitionTarget},
        memory::{MemoryFieldId, MemoryTypeLayout},
        stdlib::{StandardLibrary, StdlibSymbolId},
    };

    use super::{StdlibFieldId, StdlibTypeId};

    #[test]
    fn ordinary_catalog_record_flows_through_compiler_and_tooling_generically() {
        let source = r#"
            state "probe.exe" {}

            whileAttached {
                let probe: CatalogRecordProbe = process.read(0x100) else return
                if probe == probe {
                    print(probe.value as String)
                }
            }
        "#;
        let library = StandardLibrary::new();
        let record = library
            .type_by_name("CatalogRecordProbe")
            .expect("the test record should resolve by its sole catalog name");
        assert_eq!(record.id, StdlibTypeId::CatalogRecordProbe);
        let field = library
            .public_field(record.id, "value")
            .expect("the test record's declared field should be discoverable");
        assert_eq!(field.id, StdlibFieldId::CatalogRecordProbeValue);
        assert_eq!(library.validate(), Vec::<String>::new());

        let mut database = CompilerDatabase::new(source);
        let checked = database
            .check()
            .expect("catalog names and fields should type-check without special cases");
        let ty = checked
            .semantics()
            .types()
            .id_for_standard(StdlibTypeId::CatalogRecordProbe);
        let MemoryTypeLayout::Record(memory) = checked
            .memory_layouts()
            .layout(ty, checked.semantics())
            .expect("the declared capability should produce a generic memory layout")
        else {
            panic!("the catalog fixture should have a record memory layout")
        };
        assert_eq!((memory.size, memory.alignment), (4, 4));
        assert_eq!(
            memory.fields[0].field,
            MemoryFieldId::Standard(StdlibFieldId::CatalogRecordProbeValue)
        );

        let type_offset = source.find("CatalogRecordProbe").unwrap();
        assert_eq!(
            database.definition_at(type_offset).unwrap(),
            Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Type(StdlibTypeId::CatalogRecordProbe)
            ))
        );
        let type_hover = database
            .hover(type_offset)
            .unwrap()
            .expect("catalog type documentation should power hover");
        assert!(
            type_hover
                .markdown
                .contains("Exercises the generic standard-library record pipeline")
        );
        let type_completion = database
            .completions(type_offset + "CatalogRecord".len())
            .unwrap();
        assert!(type_completion.items.iter().any(|item| {
            item.label == "CatalogRecordProbe"
                && item.kind == CompletionKind::Struct
                && item.documentation.as_deref().is_some_and(|documentation| {
                    documentation.contains("generic standard-library record pipeline")
                })
        }));

        let field_offset = source.find("probe.value").unwrap() + "probe.".len();
        assert_eq!(
            database.definition_at(field_offset).unwrap(),
            Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Field(StdlibFieldId::CatalogRecordProbeValue)
            ))
        );
        let hover = database
            .hover(field_offset)
            .unwrap()
            .expect("catalog field documentation should power hover");
        assert!(hover.markdown.contains("CatalogRecordProbe.value: u32"));
        assert!(hover.markdown.contains("Returns the probe value"));

        let completion_offset = field_offset + "va".len();
        let completion = database.completions(completion_offset).unwrap();
        let value = completion
            .items
            .iter()
            .find(|item| item.label == "value")
            .expect("catalog fields should power receiver completion");
        assert_eq!(value.kind, CompletionKind::Property);
        assert!(
            value
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("Returns the probe value"))
        );

        let wasm = crate::compile(source)
            .expect("generic process-memory and field lowering should compile the catalog record");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("the catalog representation should produce a valid Wasm GC record");
    }
}
