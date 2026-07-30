//! Interned semantic types exposed by checked programs and editor tooling.
//!
//! Source syntax uses [`crate::ast::TypeRef`], while the private inference
//! module owns temporary types and variables. This module is the stable,
//! inference-free boundary for later compiler stages and tooling.

use std::{collections::HashMap, fmt};

use crate::{
    ast::{
        ArrayTypeDecl, ArrayTypeId, EnumDecl, EnumId, OptionTypeDecl, OptionTypeId, RecordDecl,
        RecordId, ResultTypeDecl, ResultTypeId,
    },
    inference::Type,
    stdlib::{CoreTypeId, StandardLibrary, StdlibCapabilityId, StdlibTypeId},
};

/// An interned type in one checked program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A built-in, non-constructed SplitScript type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuiltinType {
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

impl BuiltinType {
    pub const fn from_core(core: CoreTypeId) -> Self {
        match core {
            CoreTypeId::Void => Self::Void,
            CoreTypeId::Bool => Self::Bool,
            CoreTypeId::I8 => Self::I8,
            CoreTypeId::U8 => Self::U8,
            CoreTypeId::I16 => Self::I16,
            CoreTypeId::U16 => Self::U16,
            CoreTypeId::I32 => Self::I32,
            CoreTypeId::U32 => Self::U32,
            CoreTypeId::I64 => Self::I64,
            CoreTypeId::U64 => Self::U64,
            CoreTypeId::Address => Self::Address,
            CoreTypeId::F32 => Self::F32,
            CoreTypeId::F64 => Self::F64,
        }
    }

    pub const fn core(self) -> CoreTypeId {
        match self {
            Self::Void => CoreTypeId::Void,
            Self::Bool => CoreTypeId::Bool,
            Self::I8 => CoreTypeId::I8,
            Self::U8 => CoreTypeId::U8,
            Self::I16 => CoreTypeId::I16,
            Self::U16 => CoreTypeId::U16,
            Self::I32 => CoreTypeId::I32,
            Self::U32 => CoreTypeId::U32,
            Self::I64 => CoreTypeId::I64,
            Self::U64 => CoreTypeId::U64,
            Self::Address => CoreTypeId::Address,
            Self::F32 => CoreTypeId::F32,
            Self::F64 => CoreTypeId::F64,
        }
    }

    pub fn is_integer(self) -> bool {
        StandardLibrary::new().core_type_has_capability(self.core(), StdlibCapabilityId::Integer)
    }

    pub fn is_numeric(self) -> bool {
        StandardLibrary::new().core_type_has_capability(self.core(), StdlibCapabilityId::Numeric)
    }

    pub(crate) const fn legacy(self) -> Type {
        match self {
            Self::Void => Type::Void,
            Self::Bool => Type::Bool,
            Self::I8 => Type::I8,
            Self::U8 => Type::U8,
            Self::I16 => Type::I16,
            Self::U16 => Type::U16,
            Self::I32 => Type::I32,
            Self::U32 => Type::U32,
            Self::I64 => Type::I64,
            Self::U64 => Type::U64,
            Self::Address => Type::Address,
            Self::F32 => Type::F32,
            Self::F64 => Type::F64,
        }
    }
}

impl fmt::Display for BuiltinType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Address => "address",
            Self::F32 => "f32",
            Self::F64 => "f64",
        })
    }
}

/// The fully resolved shape of an interned semantic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Builtin(BuiltinType),
    Standard(StdlibTypeId),
    Record(RecordId),
    Enum(EnumId),
    Array {
        layout: ArrayTypeId,
        element: TypeId,
    },
    Option {
        layout: OptionTypeId,
        value: TypeId,
    },
    Result {
        layout: ResultTypeId,
        value: TypeId,
    },
}

/// Per-program storage for canonical semantic types.
#[derive(Debug, Clone)]
pub struct TypeStore {
    kinds: Vec<TypeKind>,
    interned: HashMap<TypeKind, TypeId>,
}

impl Default for TypeStore {
    fn default() -> Self {
        let mut store = Self {
            kinds: Vec::new(),
            interned: HashMap::new(),
        };
        let library = StandardLibrary::new();
        for core in library.core_types() {
            store.intern(TypeKind::Builtin(BuiltinType::from_core(core.id)));
        }
        for standard in library.types() {
            store.intern(TypeKind::Standard(standard.id));
        }
        store
    }
}

impl TypeStore {
    pub(crate) fn with_source_types(records: &[RecordDecl], enums: &[EnumDecl]) -> Self {
        let mut store = Self::default();
        for record in records {
            store.intern(TypeKind::Record(record.id));
        }
        for enumeration in enums {
            store.intern(TypeKind::Enum(enumeration.id));
        }
        store
    }

    pub fn kind(&self, id: TypeId) -> &TypeKind {
        &self.kinds[id.index()]
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeKind> {
        self.kinds.get(id.index())
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn id_for_builtin(&self, builtin: BuiltinType) -> TypeId {
        self.interned[&TypeKind::Builtin(builtin)]
    }

    pub fn id_for_standard(&self, standard: StdlibTypeId) -> TypeId {
        self.interned[&TypeKind::Standard(standard)]
    }

    pub fn id_for_record(&self, record: RecordId) -> TypeId {
        self.interned[&TypeKind::Record(record)]
    }

    pub fn id_for_enum(&self, enumeration: EnumId) -> TypeId {
        self.interned[&TypeKind::Enum(enumeration)]
    }

    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &TypeKind)> {
        self.kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| (TypeId(index as u32), kind))
    }

    pub(crate) fn intern_inferred(
        &mut self,
        ty: Type,
        arrays: &[ArrayTypeDecl],
        options: &[OptionTypeDecl],
        results: &[ResultTypeDecl],
    ) -> TypeId {
        if let Type::Known(id) = ty {
            debug_assert!(
                self.get(id).is_some(),
                "inference returned an unknown TypeId"
            );
            return id;
        }
        let kind = match ty {
            Type::Void => TypeKind::Builtin(BuiltinType::Void),
            Type::Bool => TypeKind::Builtin(BuiltinType::Bool),
            Type::I8 => TypeKind::Builtin(BuiltinType::I8),
            Type::U8 => TypeKind::Builtin(BuiltinType::U8),
            Type::I16 => TypeKind::Builtin(BuiltinType::I16),
            Type::U16 => TypeKind::Builtin(BuiltinType::U16),
            Type::I32 => TypeKind::Builtin(BuiltinType::I32),
            Type::U32 => TypeKind::Builtin(BuiltinType::U32),
            Type::I64 => TypeKind::Builtin(BuiltinType::I64),
            Type::U64 => TypeKind::Builtin(BuiltinType::U64),
            Type::Address => TypeKind::Builtin(BuiltinType::Address),
            Type::F32 => TypeKind::Builtin(BuiltinType::F32),
            Type::F64 => TypeKind::Builtin(BuiltinType::F64),
            Type::Known(_) => unreachable!("known types return before semantic interning"),
            Type::Array(id) => {
                let element = arrays
                    .iter()
                    .find(|array| array.id == id)
                    .unwrap_or_else(|| panic!("missing checked array type {id}"))
                    .element;
                let element = self.intern_type_ref(element, arrays, options, results);
                TypeKind::Array {
                    layout: id,
                    element,
                }
            }
            Type::Option(id) => {
                let value = options
                    .iter()
                    .find(|option| option.id == id)
                    .unwrap_or_else(|| panic!("missing checked option type {id}"))
                    .value;
                let value = self.intern_type_ref(value, arrays, options, results);
                TypeKind::Option { layout: id, value }
            }
            Type::Result(id) => {
                let value = results
                    .iter()
                    .find(|result| result.id == id)
                    .unwrap_or_else(|| panic!("missing checked result type {id}"))
                    .value;
                let value = self.intern_type_ref(value, arrays, options, results);
                TypeKind::Result { layout: id, value }
            }
            Type::Variable(_) => {
                unreachable!("unresolved `{ty}` reached the semantic type store")
            }
        };
        self.intern(kind)
    }

    fn intern_type_ref(
        &mut self,
        ty: crate::ast::TypeRef,
        arrays: &[ArrayTypeDecl],
        options: &[OptionTypeDecl],
        results: &[ResultTypeDecl],
    ) -> TypeId {
        match ty {
            crate::ast::TypeRef::Standard(standard) => self.id_for_standard(standard),
            crate::ast::TypeRef::Record(record) => self.id_for_record(record),
            crate::ast::TypeRef::Enum(enumeration) => self.id_for_enum(enumeration),
            crate::ast::TypeRef::Named(name) => {
                unreachable!("unresolved nominal type name {name} reached semantic interning")
            }
            ty => self.intern_inferred(ty.into(), arrays, options, results),
        }
    }

    pub(crate) fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(id) = self.interned.get(&kind) {
            return *id;
        }
        let id = TypeId(self.kinds.len() as u32);
        self.kinds.push(kind.clone());
        self.interned.insert(kind, id);
        id
    }
}
