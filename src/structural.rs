//! Shared semantic shape of user-defined structural values.
//!
//! Capability analysis and lazy backend derivations consume this one graph so
//! equality, display, and future structural capabilities cannot disagree about
//! which record fields or enum payloads belong to a type.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, EnumId, EnumVariantId, RecordDecl, RecordFieldId, RecordId},
    semantic::SemanticModel,
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StructuralTypeId {
    Record(RecordId),
    Enum(EnumId),
}

#[derive(Debug, Clone)]
pub(crate) struct StructuralMember {
    pub name: String,
    pub source: StructuralMemberId,
    /// Records always have a member type. Payload-less enum variants do not.
    pub ty: Option<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StructuralMemberId {
    RecordField(RecordFieldId),
    EnumVariant(EnumVariantId),
}

#[derive(Debug, Clone)]
pub(crate) struct StructuralType {
    pub id: StructuralTypeId,
    pub name: String,
    pub members: Vec<StructuralMember>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StructuralTypes {
    by_type: HashMap<TypeId, StructuralType>,
    semantic_types: HashMap<StructuralTypeId, TypeId>,
    records: Vec<TypeId>,
    enums: Vec<TypeId>,
}

impl StructuralTypes {
    pub fn build(records: &[RecordDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        let semantic_types = semantics
            .types()
            .iter()
            .filter_map(|(ty, kind)| match kind {
                TypeKind::Record(record) => Some((StructuralTypeId::Record(*record), ty)),
                TypeKind::Enum(enumeration) => Some((StructuralTypeId::Enum(*enumeration), ty)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut by_type = HashMap::new();
        let mut record_types = Vec::with_capacity(records.len());
        for record in records {
            let id = StructuralTypeId::Record(record.id);
            let ty = semantic_types[&id];
            by_type.insert(
                ty,
                StructuralType {
                    id,
                    name: record.name.clone(),
                    members: record
                        .fields
                        .iter()
                        .map(|field| StructuralMember {
                            name: field.name.clone(),
                            source: StructuralMemberId::RecordField(field.id),
                            ty: Some(
                                semantics
                                    .record_field_type(field.id)
                                    .expect("checked record fields have semantic types"),
                            ),
                        })
                        .collect(),
                },
            );
            record_types.push(ty);
        }
        let mut enum_types = Vec::with_capacity(enums.len());
        for enumeration in enums {
            let id = StructuralTypeId::Enum(enumeration.id);
            let ty = semantic_types[&id];
            by_type.insert(
                ty,
                StructuralType {
                    id,
                    name: enumeration.name.clone(),
                    members: enumeration
                        .variants
                        .iter()
                        .map(|variant| StructuralMember {
                            name: variant.name.clone(),
                            source: StructuralMemberId::EnumVariant(variant.id),
                            ty: semantics.enum_variant_payload(variant.id),
                        })
                        .collect(),
                },
            );
            enum_types.push(ty);
        }
        Self {
            by_type,
            semantic_types,
            records: record_types,
            enums: enum_types,
        }
    }

    pub fn get(&self, ty: TypeId) -> Option<&StructuralType> {
        self.by_type.get(&ty)
    }

    pub fn semantic_type(&self, id: StructuralTypeId) -> TypeId {
        self.semantic_types[&id]
    }

    pub fn iter(&self) -> impl Iterator<Item = (StructuralTypeId, TypeId)> + '_ {
        self.records
            .iter()
            .chain(&self.enums)
            .map(|ty| (self.by_type[ty].id, *ty))
    }

    pub fn records(&self) -> impl Iterator<Item = (TypeId, &StructuralType)> {
        self.records.iter().map(|ty| (*ty, &self.by_type[ty]))
    }

    pub fn enums(&self) -> impl Iterator<Item = (TypeId, &StructuralType)> {
        self.enums.iter().map(|ty| (*ty, &self.by_type[ty]))
    }
}
