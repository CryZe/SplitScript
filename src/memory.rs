//! Type-directed process-memory layouts.
//!
//! This module is the single source of truth for the `MemoryReadable`
//! capability. Type checking, documentation/editor queries, and WebAssembly
//! lowering all consume the same deterministic layouts.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{StructDecl, StructFieldId, StructId},
    semantic::SemanticModel,
    stdlib::{RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibFieldId},
    types::{BuiltinType, TypeId, TypeKind},
};

/// Fixed arrays are expanded into statically typed GC construction code.
/// These limits bound both the host read and compiler/module growth.
pub const MAX_FIXED_ARRAY_ELEMENTS: u32 = 4_096;
pub const MAX_FIXED_ARRAY_BYTES: u32 = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryFieldId {
    Source(StructFieldId),
    Standard(StdlibFieldId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryFieldLayout {
    pub field: MemoryFieldId,
    pub ty: TypeId,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructMemoryLayout {
    pub ty: TypeId,
    pub size: u32,
    pub alignment: u32,
    pub fields: Vec<MemoryFieldLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedArrayMemoryLayout {
    pub ty: TypeId,
    pub element: TypeId,
    pub length: u32,
    pub stride: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTypeLayout<'a> {
    Scalar { size: u32, alignment: u32 },
    Struct(&'a StructMemoryLayout),
    FixedArray(&'a FixedArrayMemoryLayout),
}

impl MemoryTypeLayout<'_> {
    pub fn size(self) -> u32 {
        match self {
            Self::Scalar { size, .. } => size,
            Self::Struct(layout) => layout.size,
            Self::FixedArray(layout) => layout.size,
        }
    }

    pub fn alignment(self) -> u32 {
        match self {
            Self::Scalar { alignment, .. } => alignment,
            Self::Struct(layout) => layout.alignment,
            Self::FixedArray(layout) => layout.alignment,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryLayouts {
    standard_library: StandardLibrary,
    structs: HashMap<TypeId, Result<StructMemoryLayout, String>>,
    arrays: HashMap<TypeId, Result<FixedArrayMemoryLayout, String>>,
    source_structs: HashMap<StructId, TypeId>,
}

impl MemoryLayouts {
    pub fn build(structs: &[StructDecl], semantics: &SemanticModel) -> Self {
        Self::build_with_library(structs, semantics, StandardLibrary::new())
    }

    pub fn build_with_library(
        structs: &[StructDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut layouts = Self {
            standard_library,
            structs: HashMap::new(),
            arrays: HashMap::new(),
            source_structs: HashMap::new(),
        };
        for structure in structs {
            let ty = semantics.types().id_for_struct(structure.id);
            layouts.source_structs.insert(structure.id, ty);
            let mut visiting = HashSet::new();
            let _ = layouts.build_struct(ty, structs, semantics, &mut visiting);
        }
        let library = layouts.standard_library.clone();
        for standard in library.all_types().iter().filter(|standard| {
            library.type_has_capability(standard.id, StdlibCapabilityId::MemoryReadable)
                && matches!(
                    standard.representation,
                    RuntimeRepresentation::GcStruct { .. }
                )
        }) {
            let mut visiting = HashSet::new();
            let _ = layouts.build_struct(
                semantics.types().id_for_standard(standard.id),
                structs,
                semantics,
                &mut visiting,
            );
        }
        let fixed_arrays = semantics
            .types()
            .iter()
            .filter_map(|(ty, kind)| {
                matches!(
                    kind,
                    TypeKind::Array {
                        length: Some(_),
                        ..
                    }
                )
                .then_some(ty)
            })
            .collect::<Vec<_>>();
        for ty in fixed_arrays {
            let mut visiting = HashSet::new();
            let _ = layouts.build_array(ty, structs, semantics, &mut visiting);
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
            TypeKind::Struct(structure) => self
                .structs
                .get(&semantics.types().id_for_struct(*structure))
                .expect("every declared struct has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::Struct)
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
                        .structs
                        .get(&ty)
                        .expect("every readable standard struct has a memory-layout result")
                        .as_ref()
                        .map(MemoryTypeLayout::Struct)
                        .map_err(Clone::clone),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!("type `{}` is not MemoryReadable", declaration.name))
                    }
                }
            }
            TypeKind::Array {
                length: Some(_), ..
            } => self
                .arrays
                .get(&ty)
                .expect("every fixed array has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::FixedArray)
                .map_err(Clone::clone),
            TypeKind::Array { length: None, .. } => Err(
                "an unsized `[T]` array has no fixed process-memory layout; use `[T; N]`"
                    .to_owned(),
            ),
            kind => Err(format!("type `{kind:?}` is not MemoryReadable")),
        }
    }

    pub fn structure(&self, structure: StructId) -> Result<&StructMemoryLayout, &str> {
        let ty = self
            .source_structs
            .get(&structure)
            .expect("every declared struct has a semantic type");
        self.structs
            .get(ty)
            .expect("every declared struct has a memory-layout result")
            .as_ref()
            .map_err(String::as_str)
    }

    /// Largest fixed-layout value represented by this analysis. Backend
    /// scratch planning uses this conservative bound so every generated
    /// `process.read<T>` destination is sized before body emission.
    pub fn maximum_size(&self) -> u32 {
        self.structs
            .values()
            .filter_map(|layout| layout.as_ref().ok())
            .map(|layout| layout.size)
            .chain(
                self.arrays
                    .values()
                    .filter_map(|layout| layout.as_ref().ok())
                    .map(|layout| layout.size),
            )
            .max()
            .unwrap_or(0)
            .max(8)
    }

    fn build_array(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<FixedArrayMemoryLayout, String> {
        if let Some(layout) = self.arrays.get(&ty) {
            return layout.clone();
        }
        if !visiting.insert(ty) {
            return Err("recursive arrays do not have a finite process-memory layout".to_owned());
        }
        let result = (|| {
            let TypeKind::Array {
                element,
                length: Some(length),
                ..
            } = semantics.types().kind(ty)
            else {
                return Err("an unsized `[T]` array has no fixed process-memory layout".to_owned());
            };
            if *length == 0 {
                return Err(
                    "a zero-length array does not represent a process-memory read".to_owned(),
                );
            }
            if *length > MAX_FIXED_ARRAY_ELEMENTS {
                return Err(format!(
                    "fixed arrays are limited to {MAX_FIXED_ARRAY_ELEMENTS} elements"
                ));
            }
            let (element_size, alignment) =
                self.fixed_layout(*element, structs, semantics, visiting)?;
            let stride = align_up(element_size, alignment);
            let size = stride
                .checked_mul(*length)
                .ok_or_else(|| "fixed array byte size overflows `u32`".to_owned())?;
            if size > MAX_FIXED_ARRAY_BYTES {
                return Err(format!(
                    "fixed process arrays are limited to {MAX_FIXED_ARRAY_BYTES} bytes"
                ));
            }
            Ok(FixedArrayMemoryLayout {
                ty,
                element: *element,
                length: *length,
                stride,
                size,
                alignment,
            })
        })();
        visiting.remove(&ty);
        self.arrays.insert(ty, result.clone());
        result
    }

    fn build_struct(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<StructMemoryLayout, String> {
        if let Some(layout) = self.structs.get(&ty) {
            return layout.clone();
        }
        if !visiting.insert(ty) {
            return Err("recursive structs do not have a finite process-memory layout".to_owned());
        }

        let result = (|| {
            let (name, declared_fields) = match semantics.types().kind(ty) {
                TypeKind::Struct(structure) => {
                    let declaration = structs
                        .iter()
                        .find(|declaration| declaration.id == *structure)
                        .expect("struct IDs refer to declarations");
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
                                        .struct_field_type(field.id)
                                        .expect("checked struct fields have semantic types"),
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
                                    semantics
                                        .standard_field_type(field.id)
                                        .expect("checked standard fields have semantic types"),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                kind => return Err(format!("type `{kind:?}` is not a struct")),
            };
            if declared_fields.is_empty() {
                return Err(format!("struct `{name}` has no readable fields"));
            }

            let mut offset = 0;
            let mut alignment = 1;
            let mut fields = Vec::with_capacity(declared_fields.len());
            for (field, field_name, field_ty) in declared_fields {
                let (field_size, field_alignment) = self
                    .fixed_layout(field_ty, structs, semantics, visiting)
                    .map_err(|error| {
                        format!("struct `{name}.{field_name}` is not MemoryReadable: {error}")
                    })?;
                offset = align_up(offset, field_alignment);
                fields.push(MemoryFieldLayout {
                    field,
                    ty: field_ty,
                    offset,
                });
                offset = offset
                    .checked_add(field_size)
                    .ok_or_else(|| format!("struct `{name}` is too large"))?;
                alignment = alignment.max(field_alignment);
            }
            Ok(StructMemoryLayout {
                ty,
                size: align_up(offset, alignment),
                alignment,
                fields,
            })
        })();
        visiting.remove(&ty);
        self.structs.insert(ty, result.clone());
        result
    }

    fn fixed_layout(
        &mut self,
        ty: TypeId,
        structs: &[StructDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(u32, u32), String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => scalar_layout(&self.standard_library, *builtin)
                .ok_or_else(|| format!("`{builtin}` has no fixed process-memory layout")),
            TypeKind::Struct(_) => self
                .build_struct(ty, structs, semantics, visiting)
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
                        .build_struct(ty, structs, semantics, visiting)
                        .map(|layout| (layout.size, layout.alignment)),
                    RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                        Err(format!(
                            "`{}` has no fixed process-memory layout",
                            declaration.name
                        ))
                    }
                }
            }
            TypeKind::Array {
                length: Some(_), ..
            } => self
                .build_array(ty, structs, semantics, visiting)
                .map(|layout| (layout.size, layout.alignment)),
            TypeKind::Array { length: None, .. } => Err(
                "an unsized `[T]` array has no fixed process-memory layout; use `[T; N]`"
                    .to_owned(),
            ),
            kind => Err(format!("`{kind:?}` has no fixed process-memory layout")),
        }
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
