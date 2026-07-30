//! Interned semantic types exposed by checked programs and editor tooling.
//!
//! Source syntax uses [`crate::ast::TypeRef`], while the private inference
//! module owns temporary types and variables. This module is the stable,
//! inference-free boundary for later compiler stages and tooling.

use std::{collections::HashMap, fmt};

use crate::{
    ast::{
        ArrayTypeDecl, ArrayTypeId, EnumId, OptionTypeDecl, OptionTypeId, RecordId, ResultTypeDecl,
        ResultTypeId,
    },
    inference::Type,
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
    String,
    Signature,
    Duration,
    Module,
    UnityModule,
    UnityImage,
    UnityClass,
    UnityField,
}

impl BuiltinType {
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::Address
        )
    }

    pub fn is_numeric(self) -> bool {
        self.is_integer() || matches!(self, Self::F32 | Self::F64)
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
            Self::String => Type::String,
            Self::Signature => Type::Signature,
            Self::Duration => Type::Duration,
            Self::Module => Type::Module,
            Self::UnityModule => Type::UnityModule,
            Self::UnityImage => Type::UnityImage,
            Self::UnityClass => Type::UnityClass,
            Self::UnityField => Type::UnityField,
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
            Self::String => "String",
            Self::Signature => "Signature",
            Self::Duration => "Duration",
            Self::Module => "Module",
            Self::UnityModule => "UnityModule",
            Self::UnityImage => "UnityImage",
            Self::UnityClass => "UnityClass",
            Self::UnityField => "UnityField",
        })
    }
}

/// The fully resolved shape of an interned semantic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Builtin(BuiltinType),
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
        for builtin in [
            BuiltinType::Void,
            BuiltinType::Bool,
            BuiltinType::I8,
            BuiltinType::U8,
            BuiltinType::I16,
            BuiltinType::U16,
            BuiltinType::I32,
            BuiltinType::U32,
            BuiltinType::I64,
            BuiltinType::U64,
            BuiltinType::Address,
            BuiltinType::F32,
            BuiltinType::F64,
            BuiltinType::String,
            BuiltinType::Signature,
            BuiltinType::Duration,
            BuiltinType::Module,
            BuiltinType::UnityModule,
            BuiltinType::UnityImage,
            BuiltinType::UnityClass,
            BuiltinType::UnityField,
        ] {
            store.intern(TypeKind::Builtin(builtin));
        }
        store
    }
}

impl TypeStore {
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
            Type::String => TypeKind::Builtin(BuiltinType::String),
            Type::Signature => TypeKind::Builtin(BuiltinType::Signature),
            Type::Duration => TypeKind::Builtin(BuiltinType::Duration),
            Type::Module => TypeKind::Builtin(BuiltinType::Module),
            Type::UnityModule => TypeKind::Builtin(BuiltinType::UnityModule),
            Type::UnityImage => TypeKind::Builtin(BuiltinType::UnityImage),
            Type::UnityClass => TypeKind::Builtin(BuiltinType::UnityClass),
            Type::UnityField => TypeKind::Builtin(BuiltinType::UnityField),
            Type::Record(id) => TypeKind::Record(id),
            Type::Enum(id) => TypeKind::Enum(id),
            Type::Array(id) => {
                let element = arrays
                    .iter()
                    .find(|array| array.id == id)
                    .unwrap_or_else(|| panic!("missing checked array type {id}"))
                    .element;
                let element = self.intern_inferred(element.into(), arrays, options, results);
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
                let value = self.intern_inferred(value.into(), arrays, options, results);
                TypeKind::Option { layout: id, value }
            }
            Type::Result(id) => {
                let value = results
                    .iter()
                    .find(|result| result.id == id)
                    .unwrap_or_else(|| panic!("missing checked result type {id}"))
                    .value;
                let value = self.intern_inferred(value.into(), arrays, options, results);
                TypeKind::Result { layout: id, value }
            }
            Type::Variable(_) => {
                unreachable!("unresolved `{ty}` reached the semantic type store")
            }
        };
        self.intern(kind)
    }

    fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(id) = self.interned.get(&kind) {
            return *id;
        }
        let id = TypeId(self.kinds.len() as u32);
        self.kinds.push(kind.clone());
        self.interned.insert(kind, id);
        id
    }
}
