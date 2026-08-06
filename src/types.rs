//! Interned semantic types exposed by checked programs and editor tooling.
//!
//! Source syntax uses [`crate::ast::TypeRef`], while the private inference
//! module owns temporary types and variables. This module is the stable,
//! inference-free boundary for later compiler stages and tooling.

use std::collections::HashMap;
use std::fmt;

use crate::{
    ast::{
        ArrayTypeId, AsyncTypeId, EnumDecl, EnumId, FunctionId, OptionTypeId, RecordDecl, RecordId,
        ResultTypeId, TypeApplicationId,
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

/// Fully resolved nominal identity for an enum type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnumTypeId {
    Source(EnumId),
    Standard(StdlibTypeId),
}

impl fmt::Display for EnumTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(id) => id.fmt(formatter),
            Self::Standard(id) => write!(formatter, "{id:?}"),
        }
    }
}

/// A resolved, inference-free type reference used by semantic layout tables.
/// Source syntax has a deliberately smaller [`crate::ast::TypeRef`] that never
/// contains compiler or standard-library identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedTypeRef {
    /// Recovery-only marker for a type that could not be inferred. Checked
    /// programs never contain this reference.
    Error,
    Core(CoreTypeId),
    Standard(StdlibTypeId),
    StateSnapshot,
    SettingsView,
    Record(RecordId),
    Enum(EnumId),
    GenericParameter(TypeId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
    Async(AsyncTypeId),
    Set(TypeApplicationId),
}

pub(crate) fn generic_parameter_name(index: u32) -> String {
    match index {
        0..=25 => char::from_u32('T' as u32 + index)
            .expect("ASCII generic parameter names are valid")
            .to_string(),
        _ => format!("T{}", index + 1),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedArrayType {
    pub id: ArrayTypeId,
    pub element: ResolvedTypeRef,
    pub length: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedOptionType {
    pub id: OptionTypeId,
    pub value: ResolvedTypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedResultType {
    pub id: ResultTypeId,
    pub value: ResolvedTypeRef,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedAsyncType {
    pub id: AsyncTypeId,
    pub value: ResolvedTypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedSetType {
    pub id: TypeApplicationId,
    pub element: ResolvedTypeRef,
    /// General GC array used as the growable backing storage.
    pub backing: ArrayTypeId,
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
    /// Recovery-only marker for failed inference. It has no source spelling,
    /// capabilities, memory layout, or code-generation representation.
    Error,
    Builtin(BuiltinType),
    Standard(StdlibTypeId),
    /// The generated structural value described by the program's `state`
    /// declaration. It is intentionally not a source-spellable nominal type.
    StateSnapshot,
    /// An immutable view of either the current or previous setting values.
    /// The backend represents this as a selector, so passing one does not
    /// allocate a GC object.
    SettingsView,
    Record(RecordId),
    Enum(EnumId),
    GenericParameter {
        owner: FunctionId,
        index: u32,
    },
    Array {
        layout: ArrayTypeId,
        element: TypeId,
        length: Option<u32>,
    },
    Option {
        layout: OptionTypeId,
        value: TypeId,
    },
    Result {
        layout: ResultTypeId,
        value: TypeId,
    },
    Async {
        layout: AsyncTypeId,
        value: TypeId,
    },
    Set {
        layout: TypeApplicationId,
        element: TypeId,
        backing: ArrayTypeId,
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
            // SettingsView's callable surface and documentation are authored
            // in the standard library, but its fields are generated from this
            // program's settings declaration. Keep one semantic identity for
            // that program-shaped view instead of interning a second nominal
            // type with the same source spelling.
            if standard.id != StdlibTypeId::SettingsView {
                store.intern(TypeKind::Standard(standard.id));
            }
        }
        store.intern(TypeKind::StateSnapshot);
        store.intern(TypeKind::SettingsView);
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

    pub(crate) fn id_for_error(&mut self) -> TypeId {
        self.intern(TypeKind::Error)
    }

    pub(crate) fn existing_error(&self) -> Option<TypeId> {
        self.interned.get(&TypeKind::Error).copied()
    }

    pub fn contains_error(&self, id: TypeId) -> bool {
        match self.kind(id) {
            TypeKind::Error => true,
            TypeKind::Array { element, .. } => self.contains_error(*element),
            TypeKind::Set { element, .. } => self.contains_error(*element),
            TypeKind::Option { value, .. }
            | TypeKind::Result { value, .. }
            | TypeKind::Async { value, .. } => self.contains_error(*value),
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::Record(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. } => false,
        }
    }

    pub fn id_for_core(&self, core: CoreTypeId) -> TypeId {
        self.id_for_builtin(core)
    }

    pub fn id_for_standard(&self, standard: StdlibTypeId) -> TypeId {
        if standard == StdlibTypeId::SettingsView {
            return self.id_for_settings_view();
        }
        self.interned[&TypeKind::Standard(standard)]
    }

    pub fn id_for_state_snapshot(&self) -> TypeId {
        self.interned[&TypeKind::StateSnapshot]
    }

    pub fn id_for_settings_view(&self) -> TypeId {
        self.interned[&TypeKind::SettingsView]
    }

    pub fn id_for_record(&self, record: RecordId) -> TypeId {
        self.interned[&TypeKind::Record(record)]
    }

    pub fn id_for_enum(&self, enumeration: EnumId) -> TypeId {
        self.interned[&TypeKind::Enum(enumeration)]
    }

    pub(crate) fn intern_generic_parameter(&mut self, owner: FunctionId, index: u32) -> TypeId {
        self.intern(TypeKind::GenericParameter { owner, index })
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
        arrays: &[ResolvedArrayType],
        options: &[ResolvedOptionType],
        results: &[ResolvedResultType],
        asyncs: &[ResolvedAsyncType],
        sets: &[ResolvedSetType],
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
                let array = arrays
                    .iter()
                    .find(|array| array.id == id)
                    .unwrap_or_else(|| panic!("missing checked array type {id}"));
                let element =
                    self.intern_type_ref(array.element, arrays, options, results, asyncs, sets);
                TypeKind::Array {
                    layout: id,
                    element,
                    length: array.length,
                }
            }
            Type::Option(id) => {
                let value = options
                    .iter()
                    .find(|option| option.id == id)
                    .unwrap_or_else(|| panic!("missing checked option type {id}"))
                    .value;
                let value = self.intern_type_ref(value, arrays, options, results, asyncs, sets);
                TypeKind::Option { layout: id, value }
            }
            Type::Result(id) => {
                let value = results
                    .iter()
                    .find(|result| result.id == id)
                    .unwrap_or_else(|| panic!("missing checked result type {id}"))
                    .value;
                let value = self.intern_type_ref(value, arrays, options, results, asyncs, sets);
                TypeKind::Result { layout: id, value }
            }
            Type::Async(id) => {
                let value = asyncs
                    .iter()
                    .find(|future| future.id == id)
                    .unwrap_or_else(|| panic!("missing checked async type {id}"))
                    .value;
                let value = self.intern_type_ref(value, arrays, options, results, asyncs, sets);
                TypeKind::Async { layout: id, value }
            }
            Type::Set(id) => {
                let set = sets
                    .iter()
                    .find(|set| set.id == id)
                    .unwrap_or_else(|| panic!("missing checked set type {id}"));
                let element =
                    self.intern_type_ref(set.element, arrays, options, results, asyncs, sets);
                TypeKind::Set {
                    layout: id,
                    element,
                    backing: set.backing,
                }
            }
            Type::Variable(_) => {
                unreachable!("unresolved `{ty}` reached the semantic type store")
            }
        };
        self.intern(kind)
    }

    fn intern_type_ref(
        &mut self,
        ty: ResolvedTypeRef,
        arrays: &[ResolvedArrayType],
        options: &[ResolvedOptionType],
        results: &[ResolvedResultType],
        asyncs: &[ResolvedAsyncType],
        sets: &[ResolvedSetType],
    ) -> TypeId {
        match ty {
            ResolvedTypeRef::Error => self.id_for_error(),
            ResolvedTypeRef::Core(core) => self.id_for_core(core),
            ResolvedTypeRef::Standard(standard) => self.id_for_standard(standard),
            ResolvedTypeRef::StateSnapshot => self.id_for_state_snapshot(),
            ResolvedTypeRef::SettingsView => self.id_for_settings_view(),
            ResolvedTypeRef::Record(record) => self.id_for_record(record),
            ResolvedTypeRef::Enum(enumeration) => self.id_for_enum(enumeration),
            ResolvedTypeRef::GenericParameter(parameter) => parameter,
            ResolvedTypeRef::Array(id) => {
                self.intern_inferred(Type::Array(id), arrays, options, results, asyncs, sets)
            }
            ResolvedTypeRef::Option(id) => {
                self.intern_inferred(Type::Option(id), arrays, options, results, asyncs, sets)
            }
            ResolvedTypeRef::Result(id) => {
                self.intern_inferred(Type::Result(id), arrays, options, results, asyncs, sets)
            }
            ResolvedTypeRef::Async(id) => {
                self.intern_inferred(Type::Async(id), arrays, options, results, asyncs, sets)
            }
            ResolvedTypeRef::Set(id) => {
                self.intern_inferred(Type::Set(id), arrays, options, results, asyncs, sets)
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
