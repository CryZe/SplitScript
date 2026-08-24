//! Recursive semantic capability analysis.
//!
//! Inference uses lightweight capability constraints while types are still
//! unknown. Once a program has semantic [`TypeId`] values, this module is the
//! authoritative query boundary for declared core/standard capabilities and
//! capabilities derived from source records, enums, and wrappers.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{EnumDecl, FunctionDecl, FunctionId, RecordDecl},
    equality::EqualityCapabilities,
    memory::MemoryLayouts,
    semantic::SemanticModel,
    stdlib::{
        CapabilityBehavior, Implementation, ItemKind, StandardLibrary, StdlibCapabilityId,
        StdlibItem, StdlibItemId, StdlibOwner, StdlibTypeConstructorId, TypeRef,
    },
    structural::StructuralTypes,
    types::{TypeId, TypeKind},
};

#[derive(Debug, Clone)]
pub struct CapabilityAnalysis {
    standard_library: StandardLibrary,
    equality: EqualityCapabilities,
    memory: MemoryLayouts,
    source_methods: HashMap<TypeId, HashMap<String, FunctionId>>,
    structural: StructuralTypes,
    structural_requirements: HashMap<StdlibCapabilityId, Vec<StdlibItemId>>,
}

/// Concrete implementation selected for a structural capability requirement.
///
/// Generic bodies retain the requirement until monomorphization. Once their
/// receiver is concrete, dispatch resolves either to a user-authored method or
/// to the standard-library member owned by that concrete type shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityMethodImplementation {
    Source(FunctionId),
    Standard(StdlibItemId),
}

/// Source declarations and aggregate layouts reached by formatting a value.
///
/// A custom formatter contributes an ordinary source-function edge. A
/// compiler-derived formatter instead reads the complete aggregate shape, so
/// consumers such as unused-declaration analysis need to observe its fields
/// even though no source function exists for that work.
#[derive(Debug, Clone, Default)]
pub(crate) struct CapabilityDependencies {
    pub(crate) source_functions: Vec<FunctionId>,
    pub(crate) derived_aggregates: Vec<TypeId>,
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
        let structural = StructuralTypes::build(records, enums, semantics);
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
            equality: EqualityCapabilities::build_with_structural(
                structural.clone(),
                semantics,
                standard_library.clone(),
            ),
            memory: MemoryLayouts::build_with_library(records, semantics, standard_library),
            source_methods,
            structural,
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
                    TypeKind::Application { constructor, .. }
                        if self
                            .standard_library
                            .type_constructor_has_capability(*constructor, capability) =>
                    {
                        true
                    }
                    TypeKind::Array { .. }
                        if self.standard_library.type_constructor_has_capability(
                            StdlibTypeConstructorId::Array,
                            capability,
                        ) =>
                    {
                        true
                    }
                    TypeKind::Set { .. }
                        if self.standard_library.type_constructor_has_capability(
                            StdlibTypeConstructorId::Set,
                            capability,
                        ) =>
                    {
                        true
                    }
                    TypeKind::Range { kind, .. }
                        if self.standard_library.type_constructor_has_capability(
                            match kind {
                                crate::ast::RangeKind::Exclusive => {
                                    StdlibTypeConstructorId::ExclusiveRange
                                }
                                crate::ast::RangeKind::Inclusive => {
                                    StdlibTypeConstructorId::InclusiveRange
                                }
                            },
                            capability,
                        ) =>
                    {
                        true
                    }
                    _ => false,
                };
                let requirements = self.structural_method_requirements(capability);
                let has_candidate = requirements
                    .iter()
                    .any(|requirement| self.method_candidate(ty, *requirement).is_some());
                let derives_debug = capability == StdlibCapabilityId::Debug
                    && !has_candidate
                    && self.debug_derivation_is_enabled(ty, semantics);
                if derives_debug {
                    self.require_derived_debug(ty, semantics, &mut HashSet::new())?;
                } else if capability == StdlibCapabilityId::Display && !has_candidate {
                    // `Debug` is a sub-capability of `Display`: ordinary values
                    // use their structural representation whenever no exact
                    // user-facing formatter was authored.
                    self.require(ty, StdlibCapabilityId::Debug, semantics)?;
                } else if !declared || has_candidate {
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
                TypeKind::Application { constructor, .. }
                    if self
                        .standard_library
                        .type_constructor_has_capability(*constructor, capability) =>
                {
                    Ok(())
                }
                TypeKind::Array { .. }
                    if self.standard_library.type_constructor_has_capability(
                        StdlibTypeConstructorId::Array,
                        capability,
                    ) =>
                {
                    Ok(())
                }
                TypeKind::Set { .. }
                    if self.standard_library.type_constructor_has_capability(
                        StdlibTypeConstructorId::Set,
                        capability,
                    ) =>
                {
                    Ok(())
                }
                TypeKind::Range { kind, .. }
                    if self.standard_library.type_constructor_has_capability(
                        match kind {
                            crate::ast::RangeKind::Exclusive => {
                                StdlibTypeConstructorId::ExclusiveRange
                            }
                            crate::ast::RangeKind::Inclusive => {
                                StdlibTypeConstructorId::InclusiveRange
                            }
                        },
                        capability,
                    ) =>
                {
                    Ok(())
                }
                kind => Err(format!(
                    "type `{kind:?}` does not provide capability `{capability:?}`"
                )),
            },
        }?;
        for super_capability in declaration.super_capabilities {
            // `Debug`'s declared `Display` super-capability is implemented by
            // the same fallback that asked for `Debug` in the first place.
            // The implication is therefore already proven here; recursing
            // through `Display` would merely rediscover this exact obligation.
            if capability == StdlibCapabilityId::Debug
                && *super_capability == StdlibCapabilityId::Display
            {
                continue;
            }
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

    pub(crate) fn resolve_method_requirement(
        &self,
        ty: TypeId,
        requirement: StdlibItemId,
        semantics: &SemanticModel,
    ) -> Option<CapabilityMethodImplementation> {
        let requirement = self.standard_library.item(requirement);
        let StdlibOwner::Capability(capability) = requirement.owner else {
            return None;
        };
        if requirement.implementation != Implementation::CapabilityRequirement
            || self.require(ty, capability, semantics).is_err()
        {
            return None;
        }
        if let Some(function) =
            self.method_implementation(ty, capability, requirement.id, semantics)
        {
            return Some(CapabilityMethodImplementation::Source(function));
        }

        let owner = match semantics.types().kind(ty) {
            TypeKind::Builtin(core) => StdlibOwner::Core(*core),
            TypeKind::Standard(standard) => StdlibOwner::Type(*standard),
            TypeKind::SettingsView => StdlibOwner::Type(crate::stdlib::StdlibTypeId::SettingsView),
            TypeKind::Array { .. } => StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Array),
            TypeKind::Option { .. } => {
                StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Option)
            }
            TypeKind::Result { .. } => {
                StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Result)
            }
            TypeKind::Set { .. } => StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Set),
            TypeKind::Range { kind, .. } => StdlibOwner::TypeConstructor(match kind {
                crate::ast::RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                crate::ast::RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
            }),
            TypeKind::Application { constructor, .. } => StdlibOwner::TypeConstructor(*constructor),
            TypeKind::Error
            | TypeKind::StateSnapshot
            | TypeKind::Record(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. }
            | TypeKind::Async { .. }
            | TypeKind::Callable { .. } => return None,
        };
        self.standard_library
            .children_of(owner)
            .filter_map(|symbol| match symbol {
                crate::stdlib::StdlibSymbolId::Item(item) => Some(item),
                _ => None,
            })
            .find(|item| {
                let item = self.standard_library.item(*item);
                item.name == requirement.name
                    && item.implementation != Implementation::CapabilityRequirement
            })
            .map(CapabilityMethodImplementation::Standard)
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

    /// Direct fields or payloads in one source-defined aggregate.
    pub fn structural_dependency_types(&self, ty: TypeId) -> impl Iterator<Item = TypeId> + '_ {
        self.structural
            .get(ty)
            .into_iter()
            .flat_map(|aggregate| aggregate.members.iter().filter_map(|member| member.ty))
    }

    /// Canonical source-aggregate shape shared by semantic derivation and the
    /// backend implementations it decides to materialize.
    pub(crate) fn structural_types(&self) -> &StructuralTypes {
        &self.structural
    }

    pub fn has_derived_display(&self, ty: TypeId, semantics: &SemanticModel) -> bool {
        self.method_candidate(ty, StdlibItemId::DisplayToString)
            .is_none()
            && self
                .require(ty, StdlibCapabilityId::Debug, semantics)
                .is_ok()
    }

    pub fn has_derived_debug(&self, ty: TypeId, semantics: &SemanticModel) -> bool {
        self.method_candidate(ty, StdlibItemId::DebugDebugString)
            .is_none()
            && self.debug_derivation_is_enabled(ty, semantics)
            && self
                .require_derived_debug(ty, semantics, &mut HashSet::new())
                .is_ok()
    }

    /// Values recursively formatted by a compiler-derived `Debug` body.
    pub fn debug_dependency_types(&self, ty: TypeId, semantics: &SemanticModel) -> Vec<TypeId> {
        if let Some(aggregate) = self.structural.get(ty) {
            return aggregate
                .members
                .iter()
                .filter_map(|member| member.ty)
                .collect();
        }
        match semantics.types().kind(ty) {
            TypeKind::Array { element, .. } | TypeKind::Set { element, .. } => vec![*element],
            TypeKind::Option { value, .. } | TypeKind::Result { value, .. } => vec![*value],
            TypeKind::Range { bound, .. } => vec![*bound],
            TypeKind::Application {
                constructor,
                arguments,
                ..
            } if self
                .standard_library
                .type_constructor_has_capability(*constructor, StdlibCapabilityId::Debug) =>
            {
                arguments.clone()
            }
            _ => Vec::new(),
        }
    }

    /// Declarations and layouts used by displaying this value, including
    /// custom overrides nested inside a compiler-derived aggregate formatter.
    pub(crate) fn method_dependencies(
        &self,
        root: TypeId,
        requirement: StdlibItemId,
        semantics: &SemanticModel,
    ) -> CapabilityDependencies {
        #[derive(Clone, Copy)]
        enum Mode {
            Display,
            Debug,
        }
        let initial_mode = match requirement {
            StdlibItemId::DisplayToString => Mode::Display,
            StdlibItemId::DebugDebugString => Mode::Debug,
            _ => {
                let source_functions =
                    match self.resolve_method_requirement(root, requirement, semantics) {
                        Some(CapabilityMethodImplementation::Source(function)) => vec![function],
                        Some(CapabilityMethodImplementation::Standard(_)) | None => Vec::new(),
                    };
                return CapabilityDependencies {
                    source_functions,
                    derived_aggregates: Vec::new(),
                };
            }
        };
        let mut pending = vec![(root, initial_mode)];
        let mut visited = HashSet::new();
        let mut dependencies = CapabilityDependencies::default();
        while let Some((ty, mode)) = pending.pop() {
            if !visited.insert((ty, matches!(mode, Mode::Debug))) {
                continue;
            }
            match mode {
                Mode::Display => {
                    if let Some(function) = self.method_implementation(
                        ty,
                        StdlibCapabilityId::Display,
                        StdlibItemId::DisplayToString,
                        semantics,
                    ) {
                        dependencies.source_functions.push(function);
                    } else if self.has_derived_display(ty, semantics) {
                        pending.push((ty, Mode::Debug));
                    }
                }
                Mode::Debug => {
                    if let Some(function) = self.method_implementation(
                        ty,
                        StdlibCapabilityId::Debug,
                        StdlibItemId::DebugDebugString,
                        semantics,
                    ) {
                        dependencies.source_functions.push(function);
                    } else if self.has_derived_debug(ty, semantics) {
                        dependencies.derived_aggregates.push(ty);
                        pending.extend(
                            self.debug_dependency_types(ty, semantics)
                                .into_iter()
                                .map(|dependency| (dependency, Mode::Debug)),
                        );
                    }
                }
            }
        }
        dependencies
            .source_functions
            .sort_by_key(|function| function.index());
        dependencies.source_functions.dedup();
        dependencies.derived_aggregates.sort_by_key(|ty| ty.index());
        dependencies.derived_aggregates.dedup();
        dependencies
    }

    pub(crate) fn display_dependencies(
        &self,
        root: TypeId,
        semantics: &SemanticModel,
    ) -> CapabilityDependencies {
        self.method_dependencies(root, StdlibItemId::DisplayToString, semantics)
    }

    /// Source functions invoked by displaying this value, including custom
    /// overrides nested inside a compiler-derived aggregate formatter.
    pub fn display_method_implementations(
        &self,
        root: TypeId,
        semantics: &SemanticModel,
    ) -> Vec<FunctionId> {
        self.display_dependencies(root, semantics).source_functions
    }

    fn debug_derivation_is_enabled(&self, ty: TypeId, semantics: &SemanticModel) -> bool {
        self.structural.get(ty).is_some()
            || match semantics.types().kind(ty) {
                TypeKind::Array { .. } => self.standard_library.type_constructor_has_capability(
                    StdlibTypeConstructorId::Array,
                    StdlibCapabilityId::Debug,
                ),
                TypeKind::Set { .. } => self.standard_library.type_constructor_has_capability(
                    StdlibTypeConstructorId::Set,
                    StdlibCapabilityId::Debug,
                ),
                TypeKind::Option { .. } => self.standard_library.type_constructor_has_capability(
                    StdlibTypeConstructorId::Option,
                    StdlibCapabilityId::Debug,
                ),
                TypeKind::Result { .. } => self.standard_library.type_constructor_has_capability(
                    StdlibTypeConstructorId::Result,
                    StdlibCapabilityId::Debug,
                ),
                TypeKind::Range { kind, .. } => {
                    self.standard_library.type_constructor_has_capability(
                        match kind {
                            crate::ast::RangeKind::Exclusive => {
                                StdlibTypeConstructorId::ExclusiveRange
                            }
                            crate::ast::RangeKind::Inclusive => {
                                StdlibTypeConstructorId::InclusiveRange
                            }
                        },
                        StdlibCapabilityId::Debug,
                    )
                }
                TypeKind::Application { constructor, .. } => self
                    .standard_library
                    .type_constructor_has_capability(*constructor, StdlibCapabilityId::Debug),
                _ => false,
            }
    }

    fn require_derived_debug(
        &self,
        ty: TypeId,
        semantics: &SemanticModel,
        visiting: &mut HashSet<TypeId>,
    ) -> Result<(), String> {
        if !visiting.insert(ty) {
            // Source aggregates are immutable, so recursively named types
            // cannot construct a runtime cycle without a mutable container.
            // Treat the semantic contract coinductively here.
            return Ok(());
        }
        for dependency in self.debug_dependency_types(ty, semantics) {
            let result = if self
                .method_candidate(dependency, StdlibItemId::DebugDebugString)
                .is_some()
            {
                self.require(dependency, StdlibCapabilityId::Debug, semantics)
            } else if self.debug_derivation_is_enabled(dependency, semantics) {
                self.require_derived_debug(dependency, semantics, visiting)
            } else {
                self.require(dependency, StdlibCapabilityId::Debug, semantics)
            };
            if let Err(reason) = result {
                visiting.remove(&ty);
                return Err(format!(
                    "type `{}` cannot derive `Debug` because one of its contained values is not debug-printable: {reason}",
                    self.type_name(ty, semantics),
                ));
            }
        }
        visiting.remove(&ty);
        Ok(())
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
            TypeRef::Associated(_) => false,
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
                        required,
                        TypeKind::Application {
                            constructor,
                            arguments,
                            ..
                        },
                    ) if required == *constructor && arguments.len() == 1 => arguments[0],
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
            TypeRef::Callable { parameters, result } => {
                let TypeKind::Callable {
                    parameters: actual_parameters,
                    result: actual_result,
                    ..
                } = semantics.types().kind(actual)
                else {
                    return false;
                };
                parameters.len() == actual_parameters.len()
                    && parameters
                        .iter()
                        .zip(actual_parameters)
                        .all(|(required, actual)| {
                            self.type_ref_matches(*required, *actual, receiver, semantics)
                        })
                    && self.type_ref_matches(*result, *actual_result, receiver, semantics)
            }
            TypeRef::Application { .. } => false,
        }
    }

    fn type_name(&self, ty: TypeId, semantics: &SemanticModel) -> String {
        self.structural
            .get(ty)
            .map(|aggregate| aggregate.name.clone())
            .unwrap_or_else(|| format!("{:?}", semantics.types().kind(ty)))
    }
}
