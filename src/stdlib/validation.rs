//! Validation of normalized standard-library declarations.
//!
//! Validation sits above dependency-light schema and authored catalog data. It
//! receives explicit declaration views so neither lower layer reaches upward
//! into the other.

use std::collections::HashSet;

use crate::catalog::Documentation;

use super::{
    declarations::{
        CORE_TYPES, CoreTypeId, RuntimeRepresentation, StdlibCapability, StdlibField,
        StdlibNamespace, StdlibType, StdlibTypeKind, StdlibVariant,
    },
    ids::{StdlibCapabilityId, StdlibTypeConstructorId, StdlibTypeId},
    schema::TypeRef,
};

pub(super) fn validate(
    capabilities: &[StdlibCapability],
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
        let mut declared_capabilities = HashSet::new();
        for capability in ty.capabilities {
            if !declared_capabilities.insert(capability) {
                errors.push(format!(
                    "core type `{}` repeats capability `{:?}`",
                    ty.name, capability
                ));
            }
        }
        let declared_readable = declared_capabilities_satisfy(
            ty.capabilities,
            StdlibCapabilityId::MemoryReadable,
            capabilities,
        );
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
        let mut declared_capabilities = HashSet::new();
        for capability in ty.capabilities {
            if !declared_capabilities.insert(capability) {
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
        validate_field_type(field.ty, field.id, &core_type_ids, &type_ids, &mut errors);
    }

    for ty in types.iter().filter(|ty| {
        declared_capabilities_satisfy(
            ty.capabilities,
            StdlibCapabilityId::MemoryReadable,
            capabilities,
        )
    }) {
        let mut visiting = HashSet::new();
        if let Err(reason) =
            validate_standard_memory_layout(ty.id, &mut visiting, capabilities, types, fields)
        {
            errors.push(format!(
                "standard type `{}` declares MemoryReadable but {reason}",
                ty.name
            ));
        }
    }
    for ty in types.iter().filter(|ty| {
        declared_capabilities_satisfy(ty.capabilities, StdlibCapabilityId::Equatable, capabilities)
    }) {
        let mut visiting = HashSet::new();
        if let Err(reason) =
            validate_standard_equality(ty.id, &mut visiting, capabilities, types, fields)
        {
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
    capabilities: &[StdlibCapability],
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    let declaration = types
        .iter()
        .find(|declaration| declaration.id == ty)
        .expect("validated standard type references have declarations");
    if !declared_capabilities_satisfy(
        declaration.capabilities,
        StdlibCapabilityId::MemoryReadable,
        capabilities,
    ) {
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
                declared_fields.into_iter().try_for_each(|field| {
                    validate_memory_type(field.ty, visiting, capabilities, types, fields).map_err(
                        |reason| format!("field `{}` is not readable because {reason}", field.name),
                    )
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
    capabilities: &[StdlibCapability],
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    let declaration = types
        .iter()
        .find(|declaration| declaration.id == ty)
        .expect("validated standard type references have declarations");
    if !declared_capabilities_satisfy(
        declaration.capabilities,
        StdlibCapabilityId::Equatable,
        capabilities,
    ) {
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
            .filter(|core| {
                declared_capabilities_satisfy(
                    core.capabilities,
                    StdlibCapabilityId::Equatable,
                    capabilities,
                )
            })
            .map(|_| ())
            .ok_or_else(|| "its scalar storage is not Equatable".to_owned()),
        RuntimeRepresentation::GcStruct { .. } => fields
            .iter()
            .filter(|field| field.owner == ty)
            .try_for_each(|field| {
                validate_equality_type(field.ty, visiting, capabilities, types, fields).map_err(
                    |reason| format!("field `{}` is not Equatable because {reason}", field.name),
                )
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

fn validate_field_type(
    ty: TypeRef,
    field: super::StdlibFieldId,
    core_types: &HashSet<CoreTypeId>,
    standard_types: &HashSet<StdlibTypeId>,
    errors: &mut Vec<String>,
) {
    match ty {
        TypeRef::Core(referenced) if !core_types.contains(&referenced) => errors.push(format!(
            "field `{field:?}` references missing core type `{referenced:?}`"
        )),
        TypeRef::Standard(referenced) if !standard_types.contains(&referenced) => errors.push(
            format!("field `{field:?}` references missing type `{referenced:?}`"),
        ),
        TypeRef::Application { arguments, .. } => {
            for argument in arguments {
                validate_field_type(*argument, field, core_types, standard_types, errors);
            }
        }
        TypeRef::FixedArray { element, .. } => {
            validate_field_type(*element, field, core_types, standard_types, errors);
        }
        TypeRef::Parameter(parameter) => errors.push(format!(
            "field `{field:?}` references undeclared type parameter `{parameter}`"
        )),
        TypeRef::Core(_) | TypeRef::Standard(_) => {}
    }
}

fn validate_memory_type(
    ty: TypeRef,
    visiting: &mut HashSet<StdlibTypeId>,
    capabilities: &[StdlibCapability],
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    match ty {
        TypeRef::Core(core) => CORE_TYPES
            .iter()
            .find(|declaration| declaration.id == core)
            .and_then(|declaration| declaration.memory_layout)
            .map(|_| ())
            .ok_or_else(|| "it has no fixed memory layout".to_owned()),
        TypeRef::Standard(standard) => {
            validate_standard_memory_layout(standard, visiting, capabilities, types, fields)
        }
        TypeRef::FixedArray { element, length } if length != 0 => {
            validate_memory_type(*element, visiting, capabilities, types, fields)
        }
        TypeRef::FixedArray { .. } => {
            Err("zero-length arrays cannot be read from process memory".to_owned())
        }
        TypeRef::Application { .. } => {
            Err("constructed fields have no fixed process-memory layout".to_owned())
        }
        TypeRef::Parameter(_) => {
            Err("generic fields have no fixed process-memory layout".to_owned())
        }
    }
}

fn validate_equality_type(
    ty: TypeRef,
    visiting: &mut HashSet<StdlibTypeId>,
    capabilities: &[StdlibCapability],
    types: &[StdlibType],
    fields: &[StdlibField],
) -> Result<(), String> {
    match ty {
        TypeRef::Core(core) => CORE_TYPES
            .iter()
            .find(|declaration| declaration.id == core)
            .filter(|declaration| {
                declared_capabilities_satisfy(
                    declaration.capabilities,
                    StdlibCapabilityId::Equatable,
                    capabilities,
                )
            })
            .map(|_| ())
            .ok_or_else(|| "its core type is not Equatable".to_owned()),
        TypeRef::Standard(standard) => {
            validate_standard_equality(standard, visiting, capabilities, types, fields)
        }
        TypeRef::Application {
            constructor: StdlibTypeConstructorId::Option | StdlibTypeConstructorId::Result,
            arguments: [value],
        } => validate_equality_type(*value, visiting, capabilities, types, fields),
        TypeRef::Application { .. } => Err("its constructed type is not Equatable".to_owned()),
        TypeRef::FixedArray { .. } => Err("its fixed array type is not Equatable".to_owned()),
        TypeRef::Parameter(_) => Err("its generic type is not Equatable".to_owned()),
    }
}

fn declared_capabilities_satisfy(
    declared: &[StdlibCapabilityId],
    required: StdlibCapabilityId,
    capabilities: &[StdlibCapability],
) -> bool {
    declared
        .iter()
        .any(|provided| capability_implies(*provided, required, capabilities, &mut HashSet::new()))
}

fn capability_implies(
    provided: StdlibCapabilityId,
    required: StdlibCapabilityId,
    capabilities: &[StdlibCapability],
    visited: &mut HashSet<StdlibCapabilityId>,
) -> bool {
    provided == required
        || (visited.insert(provided)
            && capabilities
                .iter()
                .find(|capability| capability.id == provided)
                .is_some_and(|capability| {
                    capability
                        .super_capabilities
                        .iter()
                        .any(|super_capability| {
                            capability_implies(*super_capability, required, capabilities, visited)
                        })
                }))
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
