//! Recursive semantic capability analysis.
//!
//! Inference uses lightweight capability constraints while types are still
//! unknown. Once a program has semantic [`TypeId`] values, this module is the
//! authoritative query boundary for declared core/standard capabilities and
//! capabilities derived from source records, enums, and wrappers.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, FunctionDecl, FunctionId, RecordDecl},
    equality::EqualityCapabilities,
    memory::MemoryLayouts,
    semantic::SemanticModel,
    stdlib::{
        CapabilityBehavior, Implementation, ItemKind, StandardLibrary, StdlibCapabilityId,
        StdlibItem, StdlibItemId, StdlibOwner, StdlibTypeConstructorId, TypeRef,
    },
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone)]
pub struct CapabilityAnalysis {
    standard_library: StandardLibrary,
    equality: EqualityCapabilities,
    memory: MemoryLayouts,
    source_methods: HashMap<TypeId, HashMap<String, FunctionId>>,
    type_names: HashMap<TypeId, String>,
    structural_requirements: HashMap<StdlibCapabilityId, Vec<StdlibItemId>>,
}

impl CapabilityAnalysis {
    pub fn build(
        records: &[RecordDecl],
        enums: &[EnumDecl],
        functions: &[FunctionDecl],
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut source_methods = HashMap::<TypeId, HashMap<String, FunctionId>>::new();
        for function in functions
            .iter()
            .filter(|function| function.method_of.is_some())
        {
            let Some(receiver) = semantics
                .function_parameter_types(function.id)
                .first()
                .copied()
            else {
                continue;
            };
            if matches!(
                semantics.types().kind(receiver),
                TypeKind::Record(_) | TypeKind::Enum(_)
            ) {
                source_methods
                    .entry(receiver)
                    .or_default()
                    .insert(function.name.clone(), function.id);
            }
        }
        let type_names = semantics
            .types()
            .iter()
            .filter_map(|(ty, kind)| match kind {
                TypeKind::Record(id) => records
                    .iter()
                    .find(|record| record.id == *id)
                    .map(|record| (ty, record.name.clone())),
                TypeKind::Enum(id) => enums
                    .iter()
                    .find(|enumeration| enumeration.id == *id)
                    .map(|enumeration| (ty, enumeration.name.clone())),
                _ => None,
            })
            .collect();
        let structural_requirements = standard_library
            .capabilities()
            .iter()
            .filter(|capability| capability.behavior == CapabilityBehavior::StructuralMethods)
            .map(|capability| {
                let requirements = standard_library
                    .children_of(StdlibOwner::Capability(capability.id))
                    .filter_map(|symbol| match symbol {
                        crate::stdlib::StdlibSymbolId::Item(item)
                            if standard_library.item(item).implementation
                                == Implementation::CapabilityRequirement =>
                        {
                            Some(item)
                        }
                        _ => None,
                    })
                    .collect();
                (capability.id, requirements)
            })
            .collect();
        Self {
            standard_library: standard_library.clone(),
            equality: EqualityCapabilities::build_with_library(
                records,
                enums,
                semantics,
                standard_library.clone(),
            ),
            memory: MemoryLayouts::build_with_library(records, semantics, standard_library),
            source_methods,
            type_names,
            structural_requirements,
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
            .iter()
            .any(|provided| {
                self.standard_library
                    .capability_implies(*provided, capability)
            })
        {
            return Ok(());
        }
        let declaration = self.standard_library.capability(capability);
        match declaration.behavior {
            CapabilityBehavior::StructuralEquality => self.equality.require(ty, semantics),
            CapabilityBehavior::StructuralMemoryLayout => {
                self.memory.layout(ty, semantics).map(|_| ())
            }
            CapabilityBehavior::StructuralMethods => {
                let declared = match semantics.types().kind(ty) {
                    TypeKind::Builtin(core)
                        if self
                            .standard_library
                            .core_type_has_capability(*core, capability) =>
                    {
                        true
                    }
                    TypeKind::Standard(standard)
                        if self
                            .standard_library
                            .type_has_capability(*standard, capability) =>
                    {
                        true
                    }
                    _ => false,
                };
                if !declared {
                    let requirements = self.structural_method_requirements(capability);
                    if !matches!(
                        semantics.types().kind(ty),
                        TypeKind::Record(_) | TypeKind::Enum(_)
                    ) {
                        return Err(format!(
                            "type `{}` cannot structurally implement capability `{}`",
                            self.type_name(ty, semantics),
                            declaration.name,
                        ));
                    }
                    for requirement in requirements {
                        let requirement = self.standard_library.item(*requirement);
                        let Some(function) = self
                            .source_methods
                            .get(&ty)
                            .and_then(|methods| methods.get(requirement.name))
                            .copied()
                        else {
                            return Err(format!(
                                "type `{}` is missing `{}` required by capability `{}`",
                                self.type_name(ty, semantics),
                                self.standard_library.render_signature(requirement.id),
                                declaration.name,
                            ));
                        };
                        if !self.method_matches(ty, function, requirement, semantics) {
                            return Err(format!(
                                "method `{}.{}` does not match `{}` required by capability `{}`",
                                self.type_name(ty, semantics),
                                requirement.name,
                                self.standard_library.render_signature(requirement.id),
                                declaration.name,
                            ));
                        }
                    }
                }
                Ok(())
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
        }?;
        for super_capability in declaration.super_capabilities {
            self.require(ty, *super_capability, semantics)?;
        }
        Ok(())
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

    pub fn method_implementation(
        &self,
        ty: TypeId,
        capability: StdlibCapabilityId,
        requirement: StdlibItemId,
        semantics: &SemanticModel,
    ) -> Option<FunctionId> {
        let requirement = self.standard_library.item(requirement);
        (self.standard_library.capability(capability).behavior
            == CapabilityBehavior::StructuralMethods
            && requirement.owner == StdlibOwner::Capability(capability)
            && requirement.implementation == Implementation::CapabilityRequirement)
            .then(|| {
                self.source_methods
                    .get(&ty)
                    .and_then(|methods| methods.get(requirement.name))
                    .copied()
            })
            .flatten()
            .filter(|function| self.method_matches(ty, *function, requirement, semantics))
    }

    pub fn structural_method_requirements(
        &self,
        capability: StdlibCapabilityId,
    ) -> &[StdlibItemId] {
        self.structural_requirements
            .get(&capability)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn method_candidate(&self, ty: TypeId, requirement: StdlibItemId) -> Option<FunctionId> {
        let requirement = self.standard_library.item(requirement);
        self.source_methods
            .get(&ty)
            .and_then(|methods| methods.get(requirement.name))
            .copied()
    }

    fn method_matches(
        &self,
        receiver: TypeId,
        function: FunctionId,
        requirement: &StdlibItem,
        semantics: &SemanticModel,
    ) -> bool {
        let ItemKind::Method {
            receiver: required_receiver,
        } = requirement.kind
        else {
            return false;
        };
        let parameters = semantics.function_parameter_types(function);
        let Some((actual_receiver, parameters)) = parameters.split_first() else {
            return false;
        };
        if !self.type_ref_matches(required_receiver, *actual_receiver, receiver, semantics)
            || parameters.len() != requirement.signature.parameters.len()
            || !parameters
                .iter()
                .zip(requirement.signature.parameters)
                .all(|(actual, required)| {
                    self.type_ref_matches(required.ty, *actual, receiver, semantics)
                })
        {
            return false;
        }
        let Some(actual_result) = semantics.function_result(function) else {
            return false;
        };
        if requirement.signature.result_is_async {
            let TypeKind::Async { value, .. } = semantics.types().kind(actual_result) else {
                return false;
            };
            self.type_ref_matches(requirement.signature.result, *value, receiver, semantics)
        } else {
            self.type_ref_matches(
                requirement.signature.result,
                actual_result,
                receiver,
                semantics,
            )
        }
    }

    fn type_ref_matches(
        &self,
        required: TypeRef,
        actual: TypeId,
        receiver: TypeId,
        semantics: &SemanticModel,
    ) -> bool {
        match required {
            TypeRef::Core(required) => {
                matches!(semantics.types().kind(actual), TypeKind::Builtin(found) if *found == required)
            }
            TypeRef::Standard(required) => {
                matches!(semantics.types().kind(actual), TypeKind::Standard(found) if *found == required)
            }
            TypeRef::Parameter(_) => actual == receiver,
            TypeRef::Application {
                constructor,
                arguments: [element],
            } => {
                let child = match (constructor, semantics.types().kind(actual)) {
                    (StdlibTypeConstructorId::Array, TypeKind::Array { element, .. }) => *element,
                    (StdlibTypeConstructorId::Option, TypeKind::Option { value, .. })
                    | (StdlibTypeConstructorId::Result, TypeKind::Result { value, .. }) => *value,
                    (StdlibTypeConstructorId::Set, TypeKind::Set { element, .. }) => *element,
                    (
                        StdlibTypeConstructorId::ExclusiveRange,
                        TypeKind::Range {
                            kind: crate::ast::RangeKind::Exclusive,
                            bound,
                            ..
                        },
                    )
                    | (
                        StdlibTypeConstructorId::InclusiveRange,
                        TypeKind::Range {
                            kind: crate::ast::RangeKind::Inclusive,
                            bound,
                            ..
                        },
                    ) => *bound,
                    _ => return false,
                };
                self.type_ref_matches(*element, child, receiver, semantics)
            }
            TypeRef::FixedArray { element, length } => {
                let TypeKind::Array {
                    element: actual,
                    length: Some(actual_length),
                    ..
                } = semantics.types().kind(actual)
                else {
                    return false;
                };
                *actual_length == length
                    && self.type_ref_matches(*element, *actual, receiver, semantics)
            }
            TypeRef::Application { .. } => false,
        }
    }

    fn type_name(&self, ty: TypeId, semantics: &SemanticModel) -> String {
        self.type_names
            .get(&ty)
            .cloned()
            .unwrap_or_else(|| format!("{:?}", semantics.types().kind(ty)))
    }
}
