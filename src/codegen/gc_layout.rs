//! Deterministic assignment of semantic aggregate types to Wasm GC indices.

use std::collections::HashMap;

use wasm_encoder::{HeapType, RefType, StorageType, ValType};

use crate::{
    ast::{ArrayTypeDecl, EnumDecl, EnumTypeId, OptionTypeDecl, Program, ResultTypeDecl},
    semantic::ResolvedEnumVariantId,
    stdlib::{
        DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibFieldId, StdlibTypeId,
    },
};

use super::{STATE_TYPE, Type, reachability};

pub(super) struct GcLayout {
    pub standard_library: StandardLibrary,
    standard: HashMap<StdlibTypeId, u32>,
    standard_fields: HashMap<StdlibFieldId, u32>,
    async_frame: u32,
    dynamic: HashMap<Type, u32>,
    ordered: Vec<Type>,
    pub type_count: u32,
}

impl GcLayout {
    pub(super) fn plan(
        standard_library: StandardLibrary,
        program: &Program,
        enums: &[EnumDecl],
        arrays: &[ArrayTypeDecl],
        options: &[OptionTypeDecl],
        results: &[ResultTypeDecl],
        reachability: &reachability::Reachability,
    ) -> Self {
        let standard = standard_library
            .types()
            .iter()
            .filter(|declaration| {
                matches!(
                    declaration.representation,
                    RuntimeRepresentation::GcArray { .. }
                        | RuntimeRepresentation::GcStruct { .. }
                        | RuntimeRepresentation::Enum { .. }
                )
            })
            .enumerate()
            .map(|(position, declaration)| (declaration.id, STATE_TYPE + 1 + position as u32))
            .collect::<HashMap<_, _>>();
        let standard_fields = standard_library
            .types()
            .iter()
            .filter(|declaration| {
                matches!(
                    declaration.representation,
                    RuntimeRepresentation::GcStruct { .. }
                )
            })
            .flat_map(|declaration| {
                standard_library
                    .fields_of(declaration.id)
                    .enumerate()
                    .map(|(position, field)| (field.id, position as u32))
            })
            .collect::<HashMap<_, _>>();
        let async_frame = STATE_TYPE + 1 + standard.len() as u32;
        let mut next = async_frame + 1;
        let mut dynamic = HashMap::new();
        let mut ordered = Vec::new();

        for record in program
            .records
            .iter()
            .filter(|record| reachability.contains_record_type(record.id))
        {
            dynamic.insert(Type::Record(record.id), next);
            ordered.push(Type::Record(record.id));
            next += 1;
        }
        for enumeration in enums
            .iter()
            .filter(|enumeration| reachability.contains_enum_type(enumeration.id))
        {
            dynamic.insert(Type::Enum(enumeration.id), next);
            ordered.push(Type::Enum(enumeration.id));
            next += 1;
        }
        let mut constructed = arrays
            .iter()
            .filter(|array| reachability.contains_array_type(array.id))
            .map(|array| (array.id.index(), Type::Array(array.id)))
            .chain(
                options
                    .iter()
                    .filter(|option| reachability.contains_option_type(option.id))
                    .map(|option| (option.id.index(), Type::Option(option.id))),
            )
            .chain(
                results
                    .iter()
                    .filter(|result| reachability.contains_result_type(result.id))
                    .map(|result| (result.id.index(), Type::Result(result.id))),
            )
            .collect::<Vec<_>>();
        constructed.sort_by_key(|(id, _)| *id);
        for (_, ty) in constructed {
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        Self {
            standard_library,
            standard,
            standard_fields,
            async_frame,
            dynamic,
            ordered,
            type_count: next,
        }
    }

    pub(super) fn dynamic_types(&self) -> impl ExactSizeIterator<Item = Type> + '_ {
        self.ordered.iter().copied()
    }

    pub(super) fn standard_index(&self, ty: StdlibTypeId) -> u32 {
        *self
            .standard
            .get(&ty)
            .unwrap_or_else(|| panic!("standard type `{ty:?}` has no static GC layout"))
    }

    pub(super) fn standard_field_index(&self, field: StdlibFieldId) -> u32 {
        self.standard_fields
            .get(&field)
            .copied()
            .expect("every standard field belongs to its owner's declared slots")
    }

    pub(super) fn enum_variant_index(
        &self,
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        enums: &[EnumDecl],
    ) -> usize {
        match (enumeration, variant) {
            (EnumTypeId::Source(enumeration), ResolvedEnumVariantId::Source(variant)) => enums
                .iter()
                .find(|declaration| declaration.id == enumeration)
                .and_then(|declaration| {
                    declaration
                        .variants
                        .iter()
                        .position(|declared| declared.id == variant)
                })
                .expect("checked source enum variants belong to their declaration"),
            (EnumTypeId::Standard(enumeration), ResolvedEnumVariantId::Standard(variant)) => self
                .standard_library
                .variants_of(enumeration)
                .position(|declared| declared.id == variant)
                .expect("checked standard enum variants belong to their declaration"),
            _ => unreachable!("checked enum and variant identities have the same owner"),
        }
    }

    pub(super) fn async_frame_index(&self) -> u32 {
        self.async_frame
    }

    pub(super) fn index(&self, ty: Type) -> u32 {
        match ty {
            Type::Standard(standard) => self.standard_index(standard),
            Type::Record(_)
            | Type::Enum(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_) => *self
                .dynamic
                .get(&ty)
                .unwrap_or_else(|| panic!("dynamic GC type `{ty:?}` was not marked reachable")),
            _ => unreachable!("scalar types have no GC heap index"),
        }
    }

    pub(super) fn val_type(&self, ty: Type) -> ValType {
        match ty {
            Type::Void => unreachable!(),
            Type::Bool | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
                ValType::I32
            }
            Type::I64 | Type::U64 | Type::Address => ValType::I64,
            Type::F32 => ValType::F32,
            Type::F64 => ValType::F64,
            Type::Standard(standard) => {
                match self.standard_library.type_decl(standard).representation {
                    RuntimeRepresentation::Scalar { storage } => {
                        self.val_type(Type::from_declared(DeclaredTypeRef::Core(storage)))
                    }
                    RuntimeRepresentation::GcArray { nullable, .. }
                    | RuntimeRepresentation::GcStruct { nullable, .. }
                    | RuntimeRepresentation::Enum { nullable } => ValType::Ref(RefType {
                        nullable,
                        heap_type: HeapType::Concrete(self.index(ty)),
                    }),
                }
            }
            Type::Record(_)
            | Type::Enum(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_) => ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(self.index(ty)),
            }),
        }
    }

    pub(super) fn storage_type(&self, ty: Type) -> StorageType {
        match ty {
            Type::Bool | Type::I8 | Type::U8 => StorageType::I8,
            Type::I16 | Type::U16 => StorageType::I16,
            _ => StorageType::Val(self.val_type(ty)),
        }
    }
}
