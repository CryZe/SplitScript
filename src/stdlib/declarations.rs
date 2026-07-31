//! Nominal standard-library declarations shared by semantics, code generation,
//! documentation, and editor tooling.
//!
//! This module deliberately contains no parser, inference, or WebAssembly
//! encoder types. It describes public symbols and backend-neutral runtime
//! shapes; consumers resolve those identities into their own stage-specific
//! representations.

use std::fmt;

use crate::catalog::Documentation;

use super::ids::{
    IntrinsicId, StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId,
    StdlibStateProviderId, StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
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
    StateProvider(StdlibStateProviderId),
    Namespace(StdlibNamespaceId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
    Type(StdlibTypeId),
    Field(StdlibFieldId),
    Variant(StdlibVariantId),
    Item(StdlibItemId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibStateProvider {
    pub id: StdlibStateProviderId,
    pub name: &'static str,
    pub value_name: &'static str,
    pub processes: &'static [&'static str],
    pub process_type: StdlibTypeId,
    pub attachment: IntrinsicId,
    pub direct_read: StdlibItemId,
    pub documentation: Documentation<StdlibSymbolId>,
}

macro_rules! with_core_types {
    ($consumer:ident) => {
        $consumer! {
            Void => { name: "void", capabilities: &[], memory_layout: None },
            Bool => { name: "bool", capabilities: BOOL_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            I8 => { name: "i8", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            U8 => { name: "u8", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            I16 => { name: "i16", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 2, alignment: 2 }) },
            U16 => { name: "u16", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 2, alignment: 2 }) },
            I32 => { name: "i32", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            U32 => { name: "u32", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            I64 => { name: "i64", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            U64 => { name: "u64", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            Address => { name: "address", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            F32 => { name: "f32", capabilities: FLOAT_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            F64 => { name: "f64", capabilities: FLOAT_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) }
        }
    };
}

macro_rules! define_core_type_ids {
    ($($id:ident => { $($declaration:tt)* }),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum CoreTypeId {
            $($id),*
        }

        impl CoreTypeId {
            pub const ALL: &'static [Self] = &[$(Self::$id),*];
        }
    };
}

with_core_types!(define_core_type_ids);
pub(crate) use with_core_types;

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
        formatter.write_str(self.name())
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
pub enum CapabilityBehavior {
    Declared,
    StructuralEquality,
    StructuralMemoryLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibCapability {
    pub id: StdlibCapabilityId,
    pub name: &'static str,
    pub behavior: CapabilityBehavior,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibTypeConstructor {
    pub id: StdlibTypeConstructorId,
    pub name: &'static str,
    pub parameters: &'static [&'static str],
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

macro_rules! define_core_type_table {
    ($($id:ident => {
        name: $name:literal,
        capabilities: $capabilities:expr,
        memory_layout: $memory_layout:expr
    }),* $(,)?) => {
        pub(super) const CORE_TYPES: &[CoreType] = &[
            $(CoreType {
                id: CoreTypeId::$id,
                name: $name,
                capabilities: $capabilities,
                memory_layout: $memory_layout,
            }),*
        ];
    };
}

with_core_types!(define_core_type_table);

impl CoreTypeId {
    pub fn name(self) -> &'static str {
        CORE_TYPES[self as usize].name
    }

    pub fn parse(name: &str) -> Option<Self> {
        if name == "Address" {
            return Some(Self::Address);
        }
        CORE_TYPES
            .iter()
            .find(|declaration| declaration.name == name)
            .map(|declaration| declaration.id)
    }

    pub fn is_integer(self) -> bool {
        CORE_TYPES[self as usize]
            .capabilities
            .contains(&StdlibCapabilityId::Integer)
    }

    pub fn is_numeric(self) -> bool {
        CORE_TYPES[self as usize]
            .capabilities
            .contains(&StdlibCapabilityId::Numeric)
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
