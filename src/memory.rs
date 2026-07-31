//! Type-directed process-memory layouts.
//!
//! This module is the single source of truth for the `MemoryReadable`
//! capability. Type checking, documentation/editor queries, and WebAssembly
//! lowering all consume the same deterministic layouts.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{RecordDecl, RecordFieldId, RecordId},
    semantic::SemanticModel,
    stdlib::{
        DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibFieldId,
    },
    types::{BuiltinType, TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryFieldId {
    Source(RecordFieldId),
    Standard(StdlibFieldId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryFieldLayout {
    pub field: MemoryFieldId,
    pub ty: TypeId,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordMemoryLayout {
    pub ty: TypeId,
    pub size: u32,
    pub alignment: u32,
    pub fields: Vec<MemoryFieldLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTypeLayout<'a> {
    Scalar { size: u32, alignment: u32 },
    Record(&'a RecordMemoryLayout),
}

impl MemoryTypeLayout<'_> {
    pub fn size(self) -> u32 {
        match self {
            Self::Scalar { size, .. } => size,
            Self::Record(layout) => layout.size,
        }
    }

    pub fn alignment(self) -> u32 {
        match self {
            Self::Scalar { alignment, .. } => alignment,
            Self::Record(layout) => layout.alignment,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryLayouts {
    standard_library: StandardLibrary,
    records: HashMap<TypeId, Result<RecordMemoryLayout, String>>,
    source_records: HashMap<RecordId, TypeId>,
}

impl MemoryLayouts {
    pub fn build(records: &[RecordDecl], semantics: &SemanticModel) -> Self {
        Self::build_with_library(records, semantics, StandardLibrary::new())
    }

    pub fn build_with_library(
        records: &[RecordDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut layouts = Self {
            standard_library,
            records: HashMap::new(),
            source_records: HashMap::new(),
        };
        for record in records {
            let ty = semantics.types().id_for_record(record.id);
            layouts.source_records.insert(record.id, ty);
            let mut visiting = HashSet::new();
            let _ = layouts.build_record(ty, records, semantics, &mut visiting);
        }
        let library = layouts.standard_library.clone();
        for standard in library.types().iter().filter(|standard| {
            library.type_has_capability(standard.id, StdlibCapabilityId::MemoryReadable)
                && matches!(
                    standard.representation,
                    RuntimeRepresentation::GcStruct { .. }
                )
        }) {
            let mut visiting = HashSet::new();
            let _ = layouts.build_record(
                semantics.types().id_for_standard(standard.id),
                records,
                semantics,
                &mut visiting,
            );
        }
        layouts
    }

    pub fn layout<'a>(
        &'a self,
        ty: TypeId,
        semantics: &SemanticModel,
    ) -> Result<MemoryTypeLayout<'a>, String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => scalar_layout(&self.standard_library, *builtin)
                .map(|(size, alignment)| MemoryTypeLayout::Scalar { size, alignment })
                .ok_or_else(|| format!("type `{builtin}` is not MemoryReadable")),
            TypeKind::Record(record) => self
                .records
                .get(&semantics.types().id_for_record(*record))
                .expect("every declared record has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::Record)
                .map_err(Clone::clone),
            TypeKind::Standard(standard) => {
                let library = &self.standard_library;
                let declaration = library.type_decl(*standard);
                if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable) {
                    return Err(format!("type `{}` is not MemoryReadable", declaration.name));
                }
                match declaration.representation {
                    RuntimeRepresentation::Scalar { storage } => library
                        .core_type(storage)
                        .memory_layout
                        .map(|layout| MemoryTypeLayout::Scalar {
                            size: layout.size,
                            alignment: layout.alignment,
                        })
                        .ok_or_else(|| {
                            format!("type `{}` is not MemoryReadable", declaration.name)
                        }),
                    RuntimeRepresentation::GcStruct { .. } => self
                        .records
                        .get(&ty)
                        .expect("every readable standard record has a memory-layout result")
                        .as_ref()
                        .map(MemoryTypeLayout::Record)
                        .map_err(Clone::clone),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!("type `{}` is not MemoryReadable", declaration.name))
                    }
                }
            }
            kind => Err(format!("type `{kind:?}` is not MemoryReadable")),
        }
    }

    pub fn record(&self, record: RecordId) -> Result<&RecordMemoryLayout, &str> {
        let ty = self
            .source_records
            .get(&record)
            .expect("every declared record has a semantic type");
        self.records
            .get(ty)
            .expect("every declared record has a memory-layout result")
            .as_ref()
            .map_err(String::as_str)
    }

    /// Largest fixed-layout value represented by this analysis. Backend
    /// scratch planning uses this conservative bound so every generated
    /// `process.read<T>` destination is sized before body emission.
    pub fn maximum_size(&self) -> u32 {
        self.records
            .values()
            .filter_map(|layout| layout.as_ref().ok())
            .map(|layout| layout.size)
            .max()
            .unwrap_or(0)
            .max(8)
    }

    fn build_record(
        &mut self,
        ty: TypeId,
        records: &[RecordDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<RecordMemoryLayout, String> {
        if let Some(layout) = self.records.get(&ty) {
            return layout.clone();
        }
        if !visiting.insert(ty) {
            return Err("recursive records do not have a finite process-memory layout".to_owned());
        }

        let result = (|| {
            let (name, declared_fields) = match semantics.types().kind(ty) {
                TypeKind::Record(record) => {
                    let declaration = records
                        .iter()
                        .find(|declaration| declaration.id == *record)
                        .expect("record IDs refer to declarations");
                    (
                        declaration.name.clone(),
                        declaration
                            .fields
                            .iter()
                            .map(|field| {
                                (
                                    MemoryFieldId::Source(field.id),
                                    field.name.clone(),
                                    semantics
                                        .record_field_type(field.id)
                                        .expect("checked record fields have semantic types"),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                TypeKind::Standard(standard) => {
                    let library = &self.standard_library;
                    let declaration = library.type_decl(*standard);
                    if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable)
                        || !matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                    {
                        return Err(format!("type `{}` is not MemoryReadable", declaration.name));
                    }
                    (
                        declaration.name.to_owned(),
                        library
                            .fields_of(*standard)
                            .map(|field| {
                                (
                                    MemoryFieldId::Standard(field.id),
                                    field.name.to_owned(),
                                    declared_type_id(field.ty, semantics),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                kind => return Err(format!("type `{kind:?}` is not a record")),
            };
            if declared_fields.is_empty() {
                return Err(format!("record `{name}` has no readable fields"));
            }

            let mut offset = 0;
            let mut alignment = 1;
            let mut fields = Vec::with_capacity(declared_fields.len());
            for (field, field_name, field_ty) in declared_fields {
                let (field_size, field_alignment) = self
                    .fixed_layout(field_ty, records, semantics, visiting)
                    .map_err(|error| {
                        format!("record `{name}.{field_name}` is not MemoryReadable: {error}")
                    })?;
                offset = align_up(offset, field_alignment);
                fields.push(MemoryFieldLayout {
                    field,
                    ty: field_ty,
                    offset,
                });
                offset = offset
                    .checked_add(field_size)
                    .ok_or_else(|| format!("record `{name}` is too large"))?;
                alignment = alignment.max(field_alignment);
            }
            Ok(RecordMemoryLayout {
                ty,
                size: align_up(offset, alignment),
                alignment,
                fields,
            })
        })();
        visiting.remove(&ty);
        self.records.insert(ty, result.clone());
        result
    }

    fn fixed_layout(
        &mut self,
        ty: TypeId,
        records: &[RecordDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(u32, u32), String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => scalar_layout(&self.standard_library, *builtin)
                .ok_or_else(|| format!("`{builtin}` has no fixed process-memory layout")),
            TypeKind::Record(_) => self
                .build_record(ty, records, semantics, visiting)
                .map(|layout| (layout.size, layout.alignment)),
            TypeKind::Standard(standard) => {
                let library = &self.standard_library;
                let declaration = library.type_decl(*standard);
                if !library.type_has_capability(*standard, StdlibCapabilityId::MemoryReadable) {
                    return Err(format!(
                        "`{}` has no fixed process-memory layout",
                        declaration.name
                    ));
                }
                match declaration.representation {
                    RuntimeRepresentation::Scalar { storage } => library
                        .core_type(storage)
                        .memory_layout
                        .map(|layout| (layout.size, layout.alignment))
                        .ok_or_else(|| {
                            format!("`{}` has no fixed process-memory layout", declaration.name)
                        }),
                    RuntimeRepresentation::GcStruct { .. } => self
                        .build_record(ty, records, semantics, visiting)
                        .map(|layout| (layout.size, layout.alignment)),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!(
                            "`{}` has no fixed process-memory layout",
                            declaration.name
                        ))
                    }
                }
            }
            kind => Err(format!("`{kind:?}` has no fixed process-memory layout")),
        }
    }
}

fn declared_type_id(ty: DeclaredTypeRef, semantics: &SemanticModel) -> TypeId {
    match ty {
        DeclaredTypeRef::Core(core) => semantics.types().id_for_core(core),
        DeclaredTypeRef::Standard(standard) => semantics.types().id_for_standard(standard),
    }
}

fn scalar_layout(library: &StandardLibrary, ty: BuiltinType) -> Option<(u32, u32)> {
    library
        .core_type(ty)
        .memory_layout
        .map(|layout| (layout.size, layout.alignment))
}

fn align_up(value: u32, alignment: u32) -> u32 {
    debug_assert!(alignment.is_power_of_two());
    value.saturating_add(alignment - 1) & !(alignment - 1)
}
