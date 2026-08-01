//! Recursive semantic capability analysis.
//!
//! Inference uses lightweight capability constraints while types are still
//! unknown. Once a program has semantic [`TypeId`] values, this module is the
//! authoritative query boundary for declared core/standard capabilities and
//! capabilities derived from source records, enums, and wrappers.

use crate::{
    ast::{EnumDecl, RecordDecl},
    equality::EqualityCapabilities,
    memory::MemoryLayouts,
    semantic::SemanticModel,
    stdlib::{CapabilityBehavior, StandardLibrary, StdlibCapabilityId},
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone)]
pub struct CapabilityAnalysis {
    standard_library: StandardLibrary,
    equality: EqualityCapabilities,
    memory: MemoryLayouts,
}

impl CapabilityAnalysis {
    pub fn build(records: &[RecordDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        Self::build_with_library(records, enums, semantics, StandardLibrary::new())
    }

    pub fn build_with_library(
        records: &[RecordDecl],
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        Self {
            standard_library: standard_library.clone(),
            equality: EqualityCapabilities::build_with_library(
                records,
                enums,
                semantics,
                standard_library.clone(),
            ),
            memory: MemoryLayouts::build_with_library(records, semantics, standard_library),
        }
    }

    pub fn require(
        &self,
        ty: TypeId,
        capability: StdlibCapabilityId,
        semantics: &SemanticModel,
    ) -> Result<(), String> {
        if matches!(
            semantics.types().kind(ty),
            TypeKind::GenericParameter { .. }
        ) && semantics
            .generic_parameter_constraints(ty)
            .contains(&capability)
        {
            return Ok(());
        }
        match self.standard_library.capability(capability).behavior {
            CapabilityBehavior::StructuralEquality => self.equality.require(ty, semantics),
            CapabilityBehavior::StructuralMemoryLayout => {
                self.memory.layout(ty, semantics).map(|_| ())
            }
            CapabilityBehavior::Declared => match semantics.types().kind(ty) {
                TypeKind::Builtin(core)
                    if self
                        .standard_library
                        .core_type_has_capability(*core, capability) =>
                {
                    Ok(())
                }
                TypeKind::Standard(standard)
                    if self
                        .standard_library
                        .type_has_capability(*standard, capability) =>
                {
                    Ok(())
                }
                kind => Err(format!(
                    "type `{kind:?}` does not provide capability `{capability:?}`"
                )),
            },
        }
    }

    pub fn has(
        &self,
        ty: TypeId,
        capability: StdlibCapabilityId,
        semantics: &SemanticModel,
    ) -> bool {
        self.require(ty, capability, semantics).is_ok()
    }

    pub fn equality(&self) -> &EqualityCapabilities {
        &self.equality
    }

    pub fn memory(&self) -> &MemoryLayouts {
        &self.memory
    }
}
