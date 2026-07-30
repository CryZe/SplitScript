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
    stdlib::{StandardLibrary, StdlibCapabilityId},
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone)]
pub struct CapabilityAnalysis {
    equality: EqualityCapabilities,
    memory: MemoryLayouts,
}

impl CapabilityAnalysis {
    pub fn build(records: &[RecordDecl], enums: &[EnumDecl], semantics: &SemanticModel) -> Self {
        Self {
            equality: EqualityCapabilities::build(records, enums, semantics),
            memory: MemoryLayouts::build(records, semantics),
        }
    }

    pub fn require(
        &self,
        ty: TypeId,
        capability: StdlibCapabilityId,
        semantics: &SemanticModel,
    ) -> Result<(), String> {
        match capability {
            StdlibCapabilityId::Equatable => self.equality.require(ty, semantics),
            StdlibCapabilityId::MemoryReadable => self.memory.layout(ty, semantics).map(|_| ()),
            capability => match semantics.types().kind(ty) {
                TypeKind::Builtin(core)
                    if StandardLibrary::new().core_type_has_capability(core.core(), capability) =>
                {
                    Ok(())
                }
                TypeKind::Standard(standard)
                    if StandardLibrary::new().type_has_capability(*standard, capability) =>
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
