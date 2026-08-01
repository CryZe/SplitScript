//! Validation of normalized standard-library declarations.
//!
//! Validation sits above dependency-light schema and authored catalog data. It
//! receives explicit declaration views so neither lower layer reaches upward
//! into the other.

use std::collections::HashSet;

use crate::catalog::Documentation;

use super::{
    declarations::{
        CORE_TYPES, CoreTypeId, DeclaredTypeRef, RuntimeRepresentation, StdlibField,
        StdlibNamespace, StdlibType, StdlibTypeKind, StdlibVariant,
    },
    ids::{StdlibCapabilityId, StdlibTypeId},
};

pub(super) fn validate(
    namespaces: &[StdlibNamespace],
    types: &[StdlibType],
    fields: &[StdlibField],
    variants: &[StdlibVariant],
) -> Vec<String> {
    let mut errors = Vec::new();
    if !CORE_TYPES
        .iter()
        .map(|ty| ty.id)
        .eq(CoreTypeId::ALL.iter().copied())
    {
        errors.push(
            "the standard-library core table must cover primitive syntax types in canonical order"
                .to_owned(),
        );
    }
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
    for namespace in namespaces {
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
        if namespace.path.len() > 1 && !namespaces.iter().any(|candidate| candidate.path == parent)
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
    for ty in types {
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
        let has_fields = fields.iter().any(|field| field.owner == ty.id);
        let has_variants = variants.iter().any(|variant| variant.owner == ty.id);
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
    for field in fields {
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

    for ty in types.iter().filter(|ty| {
        ty.capabilities
            .contains(&StdlibCapabilityId::MemoryReadable)
    }) {
        let mut visiting = HashSet::new();
        if let Err(reason) = validate_standard_memory_layout(ty.id, &mut visiting, types, fields) {
            errors.push(format!(
                "standard type `{}` declares MemoryReadable but {reason}",
                ty.name
            ));
        }
    }
    for ty in types
        .iter()
        .filter(|ty| ty.capabilities.contains(&StdlibCapabilityId::Equatable))
    {
        let mut visiting = HashSet::new();
        if let Err(reason) = validate_standard_equality(ty.id, &mut visiting, types, fields) {
            errors.push(format!(
                "standard type `{}` declares Equatable but {reason}",
                ty.name
            ));
        }
    }

    let mut variant_ids = HashSet::new();
    let mut variant_names = HashSet::new();
    for variant in variants {
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
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    let declaration = types
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
            let declared_fields = fields
                .iter()
                .filter(|field| field.owner == ty)
                .collect::<Vec<_>>();
            if declared_fields.is_empty() {
                Err("it has no readable fields".to_owned())
            } else {
                declared_fields
                    .into_iter()
                    .try_for_each(|field| match field.ty {
                        DeclaredTypeRef::Core(core) => CORE_TYPES
                            .iter()
                            .find(|declaration| declaration.id == core)
                            .and_then(|declaration| declaration.memory_layout)
                            .map(|_| ())
                            .ok_or_else(|| {
                                format!("field `{}` has no fixed memory layout", field.name)
                            }),
                        DeclaredTypeRef::Standard(standard) => {
                            validate_standard_memory_layout(standard, visiting, types, fields)
                                .map_err(|reason| {
                                    format!(
                                        "field `{}` is not readable because {reason}",
                                        field.name
                                    )
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
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    let declaration = types
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
        RuntimeRepresentation::GcStruct { .. } => fields
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
                DeclaredTypeRef::Standard(standard) => validate_standard_equality(
                    standard, visiting, types, fields,
                )
                .map_err(|reason| {
                    format!("field `{}` is not Equatable because {reason}", field.name)
                }),
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
