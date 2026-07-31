//! Interned semantic types exposed by checked programs and editor tooling.
//!
//! Source syntax uses [`crate::ast::TypeRef`], while the private inference
//! module owns temporary types and variables. This module is the stable,
//! inference-free boundary for later compiler stages and tooling.

use std::collections::HashMap;

use crate::{
    ast::{
        ArrayTypeDecl, ArrayTypeId, EnumDecl, EnumId, OptionTypeDecl, OptionTypeId, RecordDecl,
        RecordId, ResultTypeDecl, ResultTypeId,
    },
    inference::Type,
    stdlib::{CoreTypeId, StandardLibrary, StdlibTypeId},
};

/// An interned type in one checked program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Semantic name for a core, non-constructed SplitScript type.
///
/// This is an alias, not a second primitive-type universe. The standard
/// library's [`CoreTypeId`] is the identity used by syntax, semantics, and
/// catalog queries.
pub type BuiltinType = CoreTypeId;

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
        Self::new(&StandardLibrary::new())
    }
}

impl TypeStore {
    pub(crate) fn new(library: &StandardLibrary) -> Self {
        let mut store = Self {
            kinds: Vec::new(),
            interned: HashMap::new(),
        };
        for core in library.core_types() {
            store.intern(TypeKind::Builtin(core.id));
        }
        for standard in library.types() {
            store.intern(TypeKind::Standard(standard.id));
        }
        store
    }

    pub(crate) fn with_source_types(
        library: &StandardLibrary,
        records: &[RecordDecl],
        enums: &[EnumDecl],
    ) -> Self {
        let mut store = Self::new(library);
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

    pub fn id_for_core(&self, core: CoreTypeId) -> TypeId {
        self.id_for_builtin(core)
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
            crate::ast::TypeRef::Core(core) => self.id_for_core(core),
            crate::ast::TypeRef::Standard(standard) => self.id_for_standard(standard),
            crate::ast::TypeRef::Record(record) => self.id_for_record(record),
            crate::ast::TypeRef::Enum(enumeration) => self.id_for_enum(enumeration),
            crate::ast::TypeRef::Named(name) => {
                unreachable!("unresolved nominal type name {name} reached semantic interning")
            }
            crate::ast::TypeRef::Array(id) => {
                self.intern_inferred(Type::Array(id), arrays, options, results)
            }
            crate::ast::TypeRef::Option(id) => {
                self.intern_inferred(Type::Option(id), arrays, options, results)
            }
            crate::ast::TypeRef::Result(id) => {
                self.intern_inferred(Type::Result(id), arrays, options, results)
            }
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
