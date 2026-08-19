//! Temporary types and reusable whole-program type inference.
//!
//! Source syntax never contains these types. The checker records constraints
//! here and publishes only fully resolved [`crate::types::TypeId`] values.

use std::{collections::HashMap, fmt, ops::BitOr};

use crate::{
    ast::{
        ArrayTypeId, AsyncTypeId, ConstructedTypeIdAllocator, OptionTypeId, ResultTypeId,
        TypeApplicationId,
    },
    stdlib::{CapabilityBehavior, CoreTypeId, StandardLibrary, StdlibCapabilityId, StdlibTypeId},
    types::{BuiltinType, ResolvedTypeRef, TypeId, TypeKind, TypeStore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Type {
    Known(TypeId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
    Async(AsyncTypeId),
    Set(TypeApplicationId),
    Variable(u32),
}

impl Type {
    pub(crate) fn to_ref(self, types: &TypeStore) -> ResolvedTypeRef {
        match self {
            Self::Known(id) => match types.kind(id) {
                TypeKind::Error => ResolvedTypeRef::Error,
                TypeKind::Builtin(builtin) => ResolvedTypeRef::Core(*builtin),
                TypeKind::Standard(standard) => ResolvedTypeRef::Standard(*standard),
                TypeKind::StateSnapshot => ResolvedTypeRef::StateSnapshot,
                TypeKind::SettingsView => ResolvedTypeRef::SettingsView,
                TypeKind::Record(record) => ResolvedTypeRef::Record(*record),
                TypeKind::Enum(enumeration) => ResolvedTypeRef::Enum(*enumeration),
                TypeKind::GenericParameter { .. } => ResolvedTypeRef::GenericParameter(id),
                TypeKind::Array { layout, .. } => ResolvedTypeRef::Array(*layout),
                TypeKind::Option { layout, .. } => ResolvedTypeRef::Option(*layout),
                TypeKind::Result { layout, .. } => ResolvedTypeRef::Result(*layout),
                TypeKind::Async { layout, .. } => ResolvedTypeRef::Async(*layout),
                TypeKind::Set { layout, .. } => ResolvedTypeRef::Set(*layout),
            },
            Self::Array(id) => ResolvedTypeRef::Array(id),
            Self::Option(id) => ResolvedTypeRef::Option(id),
            Self::Result(id) => ResolvedTypeRef::Result(id),
            Self::Async(id) => ResolvedTypeRef::Async(id),
            Self::Set(id) => ResolvedTypeRef::Set(id),
            Self::Variable(variable) => {
                unreachable!("inference variable ?{variable} cannot become a source type reference")
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(id) => write!(formatter, "type#{}", id.index()),
            Self::Array(id) => write!(formatter, "[T]#{id}"),
            Self::Option(id) => write!(formatter, "T?#{id}"),
            Self::Result(id) => write!(formatter, "T!#{id}"),
            Self::Async(id) => write!(formatter, "Async#{id}"),
            Self::Set(id) => write!(formatter, "Set#{id}"),
            Self::Variable(id) => write!(formatter, "?{id}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Requirements(Vec<StdlibCapabilityId>);

impl Requirements {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn capability(capability: StdlibCapabilityId) -> Self {
        Self(vec![capability])
    }

    pub(crate) fn capabilities(capabilities: impl IntoIterator<Item = StdlibCapabilityId>) -> Self {
        let mut requirements = Self::none();
        for capability in capabilities {
            if !requirements.0.contains(&capability) {
                requirements.0.push(capability);
            }
        }
        requirements
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn as_slice(&self) -> &[StdlibCapabilityId] {
        &self.0
    }

    fn contains(&self, capability: StdlibCapabilityId) -> bool {
        self.0.contains(&capability)
    }
}

impl BitOr for Requirements {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self::Output {
        for capability in rhs.0 {
            if !self.0.contains(&capability) {
                self.0.push(capability);
            }
        }
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum InferenceError {
    Message(String),
    TypeMismatch {
        left: Type,
        right: Type,
    },
    UnsupportedOperation {
        ty: Type,
        requirements: Requirements,
    },
    UnsatisfiedConstraints {
        ty: Type,
        requirements: Requirements,
    },
    IntegerLiteralOutOfRange(Type),
}

#[derive(Debug, Clone)]
struct Variable {
    parent: u32,
    binding: Option<Type>,
    requirements: Requirements,
    largest_literal: Option<u64>,
    literal_default: Option<LiteralDefault>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralDefault {
    Integer,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralKind {
    Integer,
    Float,
}

impl LiteralDefault {
    fn merge(left: Option<Self>, right: Option<Self>) -> Option<Self> {
        match (left, right) {
            (Some(Self::Float), _) | (_, Some(Self::Float)) => Some(Self::Float),
            (Some(Self::Integer), _) | (_, Some(Self::Integer)) => Some(Self::Integer),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrayLayout {
    pub(crate) id: ArrayTypeId,
    pub(crate) element: Type,
    pub(crate) length: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OptionLayout {
    pub(crate) id: OptionTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResultLayout {
    pub(crate) id: ResultTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AsyncLayout {
    pub(crate) id: AsyncTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SetLayout {
    pub(crate) id: TypeApplicationId,
    pub(crate) element: Type,
    pub(crate) backing: Option<ArrayTypeId>,
}

#[derive(Default)]
pub(crate) struct ConstructedLayouts {
    pub(crate) arrays: Vec<ArrayLayout>,
    pub(crate) options: Vec<OptionLayout>,
    pub(crate) results: Vec<ResultLayout>,
    pub(crate) asyncs: Vec<AsyncLayout>,
    pub(crate) sets: Vec<SetLayout>,
}

pub(crate) struct InferenceContext {
    standard_library: StandardLibrary,
    types: TypeStore,
    variables: Vec<Variable>,
    arrays: Vec<ArrayLayout>,
    options: Vec<OptionLayout>,
    results: Vec<ResultLayout>,
    asyncs: Vec<AsyncLayout>,
    sets: Vec<SetLayout>,
    canonical_arrays: HashMap<ArrayTypeId, ArrayTypeId>,
    canonical_options: HashMap<OptionTypeId, OptionTypeId>,
    canonical_results: HashMap<ResultTypeId, ResultTypeId>,
    canonical_asyncs: HashMap<AsyncTypeId, AsyncTypeId>,
    canonical_sets: HashMap<TypeApplicationId, TypeApplicationId>,
    constructed_types: HashMap<Type, TypeId>,
    constructed_type_ids: ConstructedTypeIdAllocator,
}

impl InferenceContext {
    pub(crate) fn new(
        standard_library: StandardLibrary,
        types: TypeStore,
        first_constructed_type_index: u32,
        layouts: ConstructedLayouts,
    ) -> Self {
        let ConstructedLayouts {
            mut arrays,
            options,
            results,
            asyncs,
            mut sets,
        } = layouts;
        let next_constructed_type_index = arrays
            .iter()
            .map(|layout| layout.id.index() as u32 + 1)
            .chain(options.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(results.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(asyncs.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(sets.iter().map(|layout| layout.id.index() as u32 + 1))
            .fold(first_constructed_type_index, u32::max);
        let mut constructed_type_ids =
            ConstructedTypeIdAllocator::starting_at(next_constructed_type_index);
        for set in &mut sets {
            let backing = arrays
                .iter()
                .find(|array| array.element == set.element && array.length.is_none())
                .map(|array| array.id)
                .unwrap_or_else(|| {
                    let id = constructed_type_ids.array();
                    arrays.push(ArrayLayout {
                        id,
                        element: set.element,
                        length: None,
                    });
                    id
                });
            set.backing = Some(backing);
        }
        Self {
            standard_library,
            types,
            variables: Vec::new(),
            arrays,
            options,
            results,
            asyncs,
            sets,
            canonical_arrays: HashMap::new(),
            canonical_options: HashMap::new(),
            canonical_results: HashMap::new(),
            canonical_asyncs: HashMap::new(),
            canonical_sets: HashMap::new(),
            constructed_types: HashMap::new(),
            constructed_type_ids,
        }
    }

    pub(crate) fn known_standard(&self, standard: StdlibTypeId) -> Type {
        Type::Known(self.types.id_for_standard(standard))
    }

    pub(crate) fn known_core(&self, core: CoreTypeId) -> Type {
        Type::Known(self.types.id_for_core(core))
    }

    pub(crate) fn known_builtin(&self, builtin: BuiltinType) -> Type {
        Type::Known(self.types.id_for_builtin(builtin))
    }

    pub(crate) fn type_store(&self) -> &TypeStore {
        &self.types
    }

    /// Produces the semantic poison type used after an already-diagnosed
    /// expression failure. It preserves declaration identity for later editor
    /// queries without making the failed program eligible for code generation.
    pub(crate) fn error_type(&mut self) -> Type {
        Type::Known(self.types.id_for_error())
    }

    pub(crate) fn is_error_type(&mut self, ty: Type) -> bool {
        matches!(
            self.shallow(ty),
            Type::Known(id) if matches!(self.types.kind(id), TypeKind::Error)
        )
    }

    pub(crate) fn is_never_type(&mut self, ty: Type) -> bool {
        matches!(
            self.shallow(ty),
            Type::Known(id)
                if matches!(
                    self.types.kind(id),
                    TypeKind::Builtin(CoreTypeId::Never)
                )
        )
    }

    pub(crate) fn standard_type(&self, ty: Type) -> Option<StdlibTypeId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.types.kind(id) {
            TypeKind::Standard(standard) => Some(*standard),
            TypeKind::SettingsView => Some(StdlibTypeId::SettingsView),
            _ => None,
        }
    }

    pub(crate) fn type_may_have_capability(
        &self,
        ty: Type,
        capability: StdlibCapabilityId,
    ) -> bool {
        type_may_have_capability(&self.standard_library, &self.types, ty, capability)
    }

    pub(crate) fn is_integer(&self, ty: Type) -> bool {
        self.type_may_have_capability(ty, StdlibCapabilityId::Integer)
    }

    pub(crate) fn is_numeric(&self, ty: Type) -> bool {
        self.type_may_have_capability(ty, StdlibCapabilityId::Numeric)
    }

    pub(crate) fn fits_unsigned_literal(&self, value: u64, ty: Type) -> bool {
        fits_unsigned_literal(&self.types, value, ty)
    }

    pub(crate) fn known_type_name(&self, id: TypeId) -> String {
        match self.types.kind(id) {
            TypeKind::Error => "<unknown>".into(),
            TypeKind::Builtin(builtin) => builtin.to_string(),
            TypeKind::Standard(standard) => self.standard_library.type_decl(*standard).name.into(),
            TypeKind::StateSnapshot => "StateSnapshot".into(),
            TypeKind::SettingsView => "SettingsView".into(),
            TypeKind::Record(record) => format!("record#{record}"),
            TypeKind::Enum(enumeration) => format!("enum#{enumeration}"),
            TypeKind::GenericParameter { index, .. } => {
                crate::types::generic_parameter_name(*index)
            }
            TypeKind::Array {
                element, length, ..
            } => match length {
                Some(length) => format!("[{}; {length}]", self.known_type_name(*element)),
                None => format!("[{}]", self.known_type_name(*element)),
            },
            TypeKind::Set { element, .. } => {
                format!("Set<{}>", self.known_type_name(*element))
            }
            TypeKind::Option { value, .. } => format!("{}?", self.known_type_name(*value)),
            TypeKind::Result { value, .. } => format!("{}!", self.known_type_name(*value)),
            TypeKind::Async { value, .. } => format!("async {}", self.known_type_name(*value)),
        }
    }

    pub(crate) fn fresh(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
    ) -> Type {
        let literal_default = largest_literal.map(|_| LiteralDefault::Integer);
        self.fresh_with_literal_default(requirements, largest_literal, literal_default)
    }

    pub(crate) fn fresh_float_literal(&mut self) -> Type {
        self.fresh_with_literal_default(
            Requirements::capability(StdlibCapabilityId::Float),
            None,
            Some(LiteralDefault::Float),
        )
    }

    fn fresh_with_literal_default(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
        literal_default: Option<LiteralDefault>,
    ) -> Type {
        let requirements = Requirements(
            self.standard_library
                .minimal_capabilities(requirements.as_slice()),
        );
        let id = self.variables.len() as u32;
        self.variables.push(Variable {
            parent: id,
            binding: None,
            requirements,
            largest_literal,
            literal_default,
        });
        Type::Variable(id)
    }

    pub(crate) fn shallow(&mut self, ty: Type) -> Type {
        let Type::Variable(id) = ty else {
            return ty;
        };
        let root = self.root(id);
        if let Some(binding) = self.variables[root as usize].binding {
            self.shallow(binding)
        } else {
            Type::Variable(root)
        }
    }

    /// Instantiates one type from a generalized function signature.
    ///
    /// Every generalized root receives one fresh variable per call. Reusing
    /// `substitutions` across all parameters and the result preserves equality
    /// relationships such as `(T, T) -> T`, including when `T` is nested in a
    /// constructed GC type.
    pub(crate) fn instantiate_type(
        &mut self,
        ty: Type,
        generalized: &[u32],
        substitutions: &mut HashMap<u32, Type>,
    ) -> Type {
        match self.shallow(ty) {
            Type::Variable(variable) if generalized.contains(&variable) => {
                if let Some(substitution) = substitutions.get(&variable) {
                    return *substitution;
                }
                let template = self.variables[variable as usize].clone();
                let substitution = self.fresh_with_literal_default(
                    template.requirements,
                    template.largest_literal,
                    template.literal_default,
                );
                substitutions.insert(variable, substitution);
                substitution
            }
            variable @ Type::Variable(_) => variable,
            Type::Array(array) => {
                let element = self.array_element(array);
                let instantiated = self.instantiate_type(element, generalized, substitutions);
                if instantiated == element {
                    Type::Array(array)
                } else {
                    Type::Array(self.array_type_with_length(instantiated, self.array_length(array)))
                }
            }
            Type::Set(set) => {
                let element = self.set_element(set);
                let instantiated = self.instantiate_type(element, generalized, substitutions);
                if instantiated == element {
                    Type::Set(set)
                } else {
                    Type::Set(self.set_type(instantiated))
                }
            }
            Type::Option(option) => {
                let value = self.option_value(option);
                let instantiated = self.instantiate_type(value, generalized, substitutions);
                if instantiated == value {
                    Type::Option(option)
                } else {
                    Type::Option(self.option_type(instantiated))
                }
            }
            Type::Result(result) => {
                let value = self.result_value(result);
                let instantiated = self.instantiate_type(value, generalized, substitutions);
                if instantiated == value {
                    Type::Result(result)
                } else {
                    Type::Result(self.result_type(instantiated))
                }
            }
            Type::Async(future) => {
                let value = self.async_value(future);
                let instantiated = self.instantiate_type(value, generalized, substitutions);
                if instantiated == value {
                    Type::Async(future)
                } else {
                    Type::Async(self.async_type(instantiated))
                }
            }
            known @ Type::Known(_) => known,
        }
    }

    pub(crate) fn unbound_variables_in(
        &mut self,
        types: impl IntoIterator<Item = Type>,
    ) -> Vec<u32> {
        let mut variables = Vec::new();
        for ty in types {
            self.collect_unbound_variables(ty, &mut variables);
        }
        variables
    }

    fn collect_unbound_variables(&mut self, ty: Type, output: &mut Vec<u32>) {
        match self.shallow(ty) {
            Type::Variable(variable) => {
                if !output.contains(&variable) {
                    output.push(variable);
                }
            }
            Type::Array(array) => self.collect_unbound_variables(self.array_element(array), output),
            Type::Set(set) => self.collect_unbound_variables(self.set_element(set), output),
            Type::Option(option) => {
                self.collect_unbound_variables(self.option_value(option), output)
            }
            Type::Result(result) => {
                self.collect_unbound_variables(self.result_value(result), output)
            }
            Type::Async(future) => self.collect_unbound_variables(self.async_value(future), output),
            Type::Known(_) => {}
        }
    }

    pub(crate) fn bind_generic_parameter(&mut self, variable: u32, parameter: TypeId) {
        let root = self.root(variable);
        debug_assert_eq!(root, variable, "generalized variables are canonical roots");
        debug_assert!(
            self.variables[root as usize].binding.is_none(),
            "generalized variables remain unbound until semantic publication"
        );
        self.variables[root as usize].binding = Some(Type::Known(parameter));
    }

    pub(crate) fn variable_requirements(&mut self, variable: u32) -> Requirements {
        let root = self.root(variable);
        self.variables[root as usize].requirements.clone()
    }

    pub(crate) fn literal_kind(&mut self, ty: Type) -> Option<LiteralKind> {
        let Type::Variable(variable) = self.shallow(ty) else {
            return None;
        };
        let root = self.root(variable);
        match self.variables[root as usize].literal_default {
            Some(LiteralDefault::Integer) => Some(LiteralKind::Integer),
            Some(LiteralDefault::Float) => Some(LiteralKind::Float),
            None => None,
        }
    }

    pub(crate) fn intern_generic_parameter(
        &mut self,
        owner: crate::ast::FunctionId,
        index: u32,
    ) -> TypeId {
        self.types.intern_generic_parameter(owner, index)
    }

    pub(crate) fn is_unbound_without_default(&mut self, ty: Type) -> bool {
        let Type::Variable(variable) = self.shallow(ty) else {
            return false;
        };
        self.default_builtin(variable).is_none()
    }

    pub(crate) fn unify(&mut self, left: Type, right: Type) -> Result<Type, InferenceError> {
        let left = self.shallow(left);
        let right = self.shallow(right);
        if self.is_error_type(left) || self.is_error_type(right) {
            return Ok(self.error_type());
        }
        match (left, right) {
            (Type::Variable(left), Type::Variable(right)) => self.unify_variables(left, right),
            (Type::Variable(variable), ty) | (ty, Type::Variable(variable)) => {
                self.bind(variable, ty)
            }
            (Type::Array(left), Type::Array(right)) => {
                if matches!(
                    (self.array_length(left), self.array_length(right)),
                    (Some(left), Some(right)) if left != right
                ) {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Array(left),
                        right: Type::Array(right),
                    });
                }
                let left_element = self.array_element(left);
                let right_element = self.array_element(right);
                self.unify(left_element, right_element)?;
                Ok(Type::Array(left))
            }
            (Type::Set(left), Type::Set(right)) => {
                let left_element = self.set_element(left);
                let right_element = self.set_element(right);
                self.unify(left_element, right_element)?;
                Ok(Type::Set(left))
            }
            (Type::Option(left), Type::Option(right)) => {
                let left_value = self.option_value(left);
                let right_value = self.option_value(right);
                self.unify(left_value, right_value)?;
                Ok(Type::Option(left))
            }
            (Type::Result(left), Type::Result(right)) => {
                let left_value = self.result_value(left);
                let right_value = self.result_value(right);
                self.unify(left_value, right_value)?;
                Ok(Type::Result(left))
            }
            (Type::Async(left), Type::Async(right)) => {
                let left_value = self.async_value(left);
                let right_value = self.async_value(right);
                self.unify(left_value, right_value)?;
                Ok(Type::Async(left))
            }
            (left, right) if left == right => Ok(left),
            (left, right) => Err(InferenceError::TypeMismatch { left, right }),
        }
    }

    pub(crate) fn require(
        &mut self,
        ty: Type,
        requirements: Requirements,
    ) -> Result<(), InferenceError> {
        let ty = self.shallow(ty);
        if self.is_error_type(ty) {
            return Ok(());
        }
        match ty {
            Type::Variable(variable) => {
                let variable = self.root(variable);
                let combined =
                    self.variables[variable as usize].requirements.clone() | requirements;
                let combined = Requirements(
                    self.standard_library
                        .minimal_capabilities(combined.as_slice()),
                );
                if !requirements_are_possible(&self.standard_library, &self.types, &combined) {
                    return Err(error("incompatible type constraints"));
                }
                self.variables[variable as usize].requirements = combined;
                Ok(())
            }
            concrete
                if type_meets_requirements(
                    &self.standard_library,
                    &self.types,
                    concrete,
                    &requirements,
                ) =>
            {
                Ok(())
            }
            concrete => Err(InferenceError::UnsupportedOperation {
                ty: concrete,
                requirements,
            }),
        }
    }

    pub(crate) fn default_unbound(&mut self) -> Vec<InferenceError> {
        let mut errors = Vec::new();
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root != id || self.variables[root as usize].binding.is_some() {
                continue;
            }
            let Some(default) = self.default_builtin(root) else {
                errors.push(error(format!(
                    "cannot infer type variable `?{root}` without more context"
                )));
                continue;
            };
            let default = self.known_builtin(default);
            match self.validate_binding(root, default) {
                Ok(()) => self.variables[root as usize].binding = Some(default),
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    fn default_builtin(&self, variable: u32) -> Option<BuiltinType> {
        let variable = &self.variables[variable as usize];
        // A numeric default is an ergonomic source-language choice. It must
        // never silently become a process-memory layout choice: memory reads
        // need an explicit concrete representation from an annotation or an
        // otherwise fixed type.
        if variable
            .requirements
            .contains(StdlibCapabilityId::MemoryReadable)
        {
            return None;
        }
        if variable.requirements.contains(StdlibCapabilityId::Float) {
            return Some(BuiltinType::F64);
        }
        if variable.requirements.contains(StdlibCapabilityId::Integer) {
            return Some(BuiltinType::I32);
        }
        match variable.literal_default {
            Some(LiteralDefault::Integer) => Some(BuiltinType::I32),
            Some(LiteralDefault::Float) => Some(BuiltinType::F64),
            None => None,
        }
    }

    /// Gives every still-unbound variable a recovery-only representation so
    /// editor queries can retain independent semantic facts after diagnostics.
    /// Strict checking never observes these fallback bindings.
    pub(crate) fn recover_unbound(&mut self) {
        let error = self.types.id_for_error();
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root == id && self.variables[root as usize].binding.is_none() {
                self.variables[root as usize].binding = Some(Type::Known(error));
            }
        }
    }

    pub(crate) fn recover_unbound_type(&mut self, ty: Type) {
        let variables = self.unbound_variables_in([ty]);
        if variables.is_empty() {
            return;
        }
        let error = Type::Known(self.types.id_for_error());
        for variable in variables {
            let root = self.root(variable);
            if self.variables[root as usize].binding.is_none() {
                self.variables[root as usize].binding = Some(error);
            }
        }
    }

    pub(crate) fn resolve(&mut self, ty: Type) -> Type {
        let ty = match self.shallow(ty) {
            Type::Variable(variable) => {
                panic!("unresolved type variable ?{variable} after inference")
            }
            Type::Array(array) => {
                Type::Array(self.canonical_arrays.get(&array).copied().unwrap_or(array))
            }
            Type::Option(option) => Type::Option(
                self.canonical_options
                    .get(&option)
                    .copied()
                    .unwrap_or(option),
            ),
            Type::Result(result) => Type::Result(
                self.canonical_results
                    .get(&result)
                    .copied()
                    .unwrap_or(result),
            ),
            Type::Async(future) => Type::Async(
                self.canonical_asyncs
                    .get(&future)
                    .copied()
                    .unwrap_or(future),
            ),
            Type::Set(set) => Type::Set(self.canonical_sets.get(&set).copied().unwrap_or(set)),
            ty => ty,
        };
        self.constructed_types
            .get(&ty)
            .copied()
            .map_or(ty, Type::Known)
    }

    pub(crate) fn intern_resolved_constructed_types(&mut self) {
        let arrays = self
            .arrays
            .iter()
            .map(|layout| Type::Array(layout.id))
            .collect::<Vec<_>>();
        let options = self
            .options
            .iter()
            .map(|layout| Type::Option(layout.id))
            .collect::<Vec<_>>();
        let results = self
            .results
            .iter()
            .map(|layout| Type::Result(layout.id))
            .collect::<Vec<_>>();
        let asyncs = self
            .asyncs
            .iter()
            .map(|layout| Type::Async(layout.id))
            .collect::<Vec<_>>();
        let sets = self
            .sets
            .iter()
            .map(|layout| Type::Set(layout.id))
            .collect::<Vec<_>>();
        for ty in arrays
            .into_iter()
            .chain(options)
            .chain(results)
            .chain(asyncs)
            .chain(sets)
        {
            self.intern_resolved_type(ty);
        }
    }

    pub(crate) fn array_type(&mut self, element: Type) -> ArrayTypeId {
        self.array_type_with_length(element, None)
    }

    pub(crate) fn array_type_with_length(
        &mut self,
        element: Type,
        length: Option<u32>,
    ) -> ArrayTypeId {
        if let Some(array) = self
            .arrays
            .iter()
            .find(|array| array.element == element && array.length == length)
        {
            return array.id;
        }
        let id = self.constructed_type_ids.array();
        self.arrays.push(ArrayLayout {
            id,
            element,
            length,
        });
        id
    }

    pub(crate) fn array_element(&self, id: ArrayTypeId) -> Type {
        self.arrays
            .iter()
            .find(|array| array.id == id)
            .expect("checked array type has a layout")
            .element
    }

    pub(crate) fn array_length(&self, id: ArrayTypeId) -> Option<u32> {
        self.arrays
            .iter()
            .find(|array| array.id == id)
            .expect("checked array type has a layout")
            .length
    }

    pub(crate) fn option_value(&self, id: OptionTypeId) -> Type {
        self.options
            .iter()
            .find(|option| option.id == id)
            .expect("checked option type has a layout")
            .value
    }

    pub(crate) fn result_value(&self, id: ResultTypeId) -> Type {
        self.results
            .iter()
            .find(|result| result.id == id)
            .expect("checked result type has a layout")
            .value
    }

    pub(crate) fn async_value(&self, id: AsyncTypeId) -> Type {
        self.asyncs
            .iter()
            .find(|future| future.id == id)
            .expect("checked async type has a declaration")
            .value
    }

    pub(crate) fn option_type(&mut self, value: Type) -> OptionTypeId {
        if let Some(option) = self.options.iter().find(|option| option.value == value) {
            return option.id;
        }
        let id = self.constructed_type_ids.option();
        self.options.push(OptionLayout { id, value });
        id
    }

    pub(crate) fn result_type(&mut self, value: Type) -> ResultTypeId {
        if let Some(result) = self.results.iter().find(|result| result.value == value) {
            return result.id;
        }
        let id = self.constructed_type_ids.result();
        self.results.push(ResultLayout { id, value });
        id
    }

    pub(crate) fn async_type(&mut self, value: Type) -> AsyncTypeId {
        if let Some(future) = self.asyncs.iter().find(|future| future.value == value) {
            return future.id;
        }
        let id = self.constructed_type_ids.async_value();
        self.asyncs.push(AsyncLayout { id, value });
        id
    }

    pub(crate) fn set_type(&mut self, element: Type) -> TypeApplicationId {
        if let Some(set) = self.sets.iter().find(|set| set.element == element) {
            return set.id;
        }
        let backing = self.array_type(element);
        let id = self.constructed_type_ids.application();
        self.sets.push(SetLayout {
            id,
            element,
            backing: Some(backing),
        });
        id
    }

    pub(crate) fn set_element(&self, id: TypeApplicationId) -> Type {
        self.sets
            .iter()
            .find(|set| set.id == id)
            .expect("checked set type has a layout")
            .element
    }

    pub(crate) fn set_backing(&self, id: TypeApplicationId) -> ArrayTypeId {
        self.sets
            .iter()
            .find(|set| set.id == id)
            .expect("checked set type has a layout")
            .backing
            .expect("set backing arrays are assigned during inference initialization")
    }

    pub(crate) fn finalize_arrays(&mut self) {
        let unresolved = self
            .arrays
            .iter()
            .map(|array| array.element)
            .collect::<Vec<_>>();
        let resolved = unresolved
            .into_iter()
            .map(|element| self.resolve(element))
            .collect::<Vec<_>>();
        for (array, element) in self.arrays.iter_mut().zip(resolved) {
            array.element = element;
        }

        // Array layouts can be allocated while their element is still an
        // inference variable. Distinct provisional layouts may consequently
        // resolve to the same `[T]` or `[T; N]` type. Collapse those layouts
        // before semantic publication so Wasm GC never receives two nominal
        // identities for one source type.
        loop {
            let previous = self.canonical_arrays.clone();
            self.canonical_arrays.clear();
            let mut representatives = Vec::<(Type, Option<u32>, ArrayTypeId)>::new();
            for array in &self.arrays {
                let element = canonical_constructed_type(
                    array.element,
                    &previous,
                    &self.canonical_options,
                    &self.canonical_results,
                    &self.canonical_asyncs,
                    &self.canonical_sets,
                );
                let canonical = representatives
                    .iter()
                    .find_map(|(candidate, length, id)| {
                        (*candidate == element && *length == array.length).then_some(*id)
                    })
                    .unwrap_or_else(|| {
                        representatives.push((element, array.length, array.id));
                        array.id
                    });
                self.canonical_arrays.insert(array.id, canonical);
            }
            if self.canonical_arrays == previous {
                break;
            }
        }

        let canonical_arrays = self.canonical_arrays.clone();
        for array in &mut self.arrays {
            array.element = canonical_constructed_type(
                array.element,
                &canonical_arrays,
                &self.canonical_options,
                &self.canonical_results,
                &self.canonical_asyncs,
                &self.canonical_sets,
            );
        }
    }

    pub(crate) fn finalize_wrappers(&mut self) {
        let option_values = self
            .options
            .iter()
            .map(|option| option.value)
            .collect::<Vec<_>>();
        let option_values = option_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (option, value) in self.options.iter_mut().zip(option_values) {
            option.value = value;
        }

        let result_values = self
            .results
            .iter()
            .map(|result| result.value)
            .collect::<Vec<_>>();
        let result_values = result_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (result, value) in self.results.iter_mut().zip(result_values) {
            result.value = value;
        }

        let async_values = self
            .asyncs
            .iter()
            .map(|future| future.value)
            .collect::<Vec<_>>();
        let async_values = async_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (future, value) in self.asyncs.iter_mut().zip(async_values) {
            future.value = value;
        }

        // Constructors can allocate a provisional wrapper before later uses
        // constrain its value type. Once all inference variables resolve,
        // collapse layouts with identical value types to the first stable ID.
        // This keeps WebAssembly GC's nominal type identities aligned with the
        // structural unification performed above.
        loop {
            let previous_options = self.canonical_options.clone();
            let previous_results = self.canonical_results.clone();
            let previous_asyncs = self.canonical_asyncs.clone();
            self.canonical_options.clear();
            self.canonical_results.clear();
            self.canonical_asyncs.clear();

            let mut option_representatives = Vec::<(Type, OptionTypeId)>::new();
            for option in &self.options {
                let value = canonical_constructed_type(
                    option.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &self.canonical_sets,
                );
                let canonical = option_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        option_representatives.push((value, option.id));
                        option.id
                    });
                self.canonical_options.insert(option.id, canonical);
            }

            let mut result_representatives = Vec::<(Type, ResultTypeId)>::new();
            for result in &self.results {
                let value = canonical_constructed_type(
                    result.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &self.canonical_sets,
                );
                let canonical = result_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        result_representatives.push((value, result.id));
                        result.id
                    });
                self.canonical_results.insert(result.id, canonical);
            }

            let mut async_representatives = Vec::<(Type, AsyncTypeId)>::new();
            for future in &self.asyncs {
                let value = canonical_constructed_type(
                    future.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &self.canonical_sets,
                );
                let canonical = async_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        async_representatives.push((value, future.id));
                        future.id
                    });
                self.canonical_asyncs.insert(future.id, canonical);
            }

            if self.canonical_options == previous_options
                && self.canonical_results == previous_results
                && self.canonical_asyncs == previous_asyncs
            {
                break;
            }
        }

        let canonical_options = self.canonical_options.clone();
        let canonical_results = self.canonical_results.clone();
        let canonical_asyncs = self.canonical_asyncs.clone();
        for option in &mut self.options {
            option.value = canonical_constructed_type(
                option.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &self.canonical_sets,
            );
        }
        for result in &mut self.results {
            result.value = canonical_constructed_type(
                result.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &self.canonical_sets,
            );
        }
        for future in &mut self.asyncs {
            future.value = canonical_constructed_type(
                future.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &self.canonical_sets,
            );
        }
    }

    pub(crate) fn finalize_sets(&mut self) {
        let unresolved = self.sets.iter().map(|set| set.element).collect::<Vec<_>>();
        let resolved = unresolved
            .into_iter()
            .map(|element| self.resolve(element))
            .collect::<Vec<_>>();
        for (set, element) in self.sets.iter_mut().zip(resolved) {
            set.element = element;
            let backing = set
                .backing
                .expect("set backing arrays are assigned during inference initialization");
            set.backing = Some(
                self.canonical_arrays
                    .get(&backing)
                    .copied()
                    .unwrap_or(backing),
            );
        }

        let mut representatives = Vec::<(Type, TypeApplicationId)>::new();
        self.canonical_sets.clear();
        for set in &self.sets {
            let canonical = representatives
                .iter()
                .find_map(|(candidate, id)| (*candidate == set.element).then_some(*id))
                .unwrap_or_else(|| {
                    representatives.push((set.element, set.id));
                    set.id
                });
            self.canonical_sets.insert(set.id, canonical);
        }
    }

    pub(crate) fn arrays(&self) -> &[ArrayLayout] {
        &self.arrays
    }

    pub(crate) fn options(&self) -> &[OptionLayout] {
        &self.options
    }

    pub(crate) fn results(&self) -> &[ResultLayout] {
        &self.results
    }

    pub(crate) fn asyncs(&self) -> &[AsyncLayout] {
        &self.asyncs
    }

    pub(crate) fn sets(&self) -> &[SetLayout] {
        &self.sets
    }

    fn intern_resolved_type(&mut self, ty: Type) -> TypeId {
        let ty = self.resolve(ty);
        if let Type::Known(id) = ty {
            return id;
        }
        if let Some(id) = self.constructed_types.get(&ty) {
            return *id;
        }
        let kind = match ty {
            Type::Array(layout) => {
                let element = self.array_element(layout);
                let element = self.intern_resolved_type(element);
                TypeKind::Array {
                    layout,
                    element,
                    length: self.array_length(layout),
                }
            }
            Type::Option(layout) => {
                let value = self.option_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Option { layout, value }
            }
            Type::Result(layout) => {
                let value = self.result_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Result { layout, value }
            }
            Type::Async(layout) => {
                let value = self.async_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Async { layout, value }
            }
            Type::Set(layout) => {
                let element = self.set_element(layout);
                let element = self.intern_resolved_type(element);
                TypeKind::Set {
                    layout,
                    element,
                    backing: self.set_backing(layout),
                }
            }
            Type::Variable(variable) => {
                unreachable!("unresolved type variable ?{variable} reached semantic interning")
            }
            Type::Known(_) => unreachable!("known types return before constructed interning"),
        };
        let id = self.types.intern(kind);
        self.constructed_types.insert(ty, id);
        id
    }

    fn root(&mut self, id: u32) -> u32 {
        let parent = self.variables[id as usize].parent;
        if parent == id {
            id
        } else {
            let root = self.root(parent);
            self.variables[id as usize].parent = root;
            root
        }
    }

    fn unify_variables(&mut self, left: u32, right: u32) -> Result<Type, InferenceError> {
        let left = self.root(left);
        let right = self.root(right);
        if left == right {
            return Ok(Type::Variable(left));
        }

        let right_variable = self.variables[right as usize].clone();
        let left_variable = self.variables[left as usize].clone();
        self.variables[right as usize].parent = left;
        let requirements = left_variable.requirements.clone() | right_variable.requirements.clone();
        self.variables[left as usize].requirements = Requirements(
            self.standard_library
                .minimal_capabilities(requirements.as_slice()),
        );
        self.variables[left as usize].largest_literal = left_variable
            .largest_literal
            .max(right_variable.largest_literal);
        self.variables[left as usize].literal_default = LiteralDefault::merge(
            left_variable.literal_default,
            right_variable.literal_default,
        );
        self.variables[left as usize].binding = None;

        let binding = match (left_variable.binding, right_variable.binding) {
            (Some(left), Some(right)) => Some(self.unify(left, right)?),
            (Some(binding), None) | (None, Some(binding)) => Some(binding),
            (None, None) => None,
        };
        if let Some(binding) = binding {
            self.validate_binding(left, binding)?;
            self.variables[left as usize].binding = Some(binding);
            Ok(self.shallow(Type::Variable(left)))
        } else if requirements_are_possible(
            &self.standard_library,
            &self.types,
            &self.variables[left as usize].requirements,
        ) {
            Ok(Type::Variable(left))
        } else {
            Err(error("incompatible type constraints"))
        }
    }

    fn bind(&mut self, variable: u32, ty: Type) -> Result<Type, InferenceError> {
        let variable = self.root(variable);
        let ty = self.shallow(ty);
        if ty == Type::Variable(variable) {
            return Ok(ty);
        }
        if let Some(binding) = self.variables[variable as usize].binding {
            return self.unify(binding, ty);
        }
        if self.occurs_in(variable, ty, &mut Vec::new()) {
            return Err(InferenceError::Message(
                "polymorphic recursion is not supported because it would require an infinite recursive type"
                    .to_owned(),
            ));
        }
        self.validate_binding(variable, ty)?;
        self.variables[variable as usize].binding = Some(ty);
        Ok(ty)
    }

    fn occurs_in(&mut self, variable: u32, ty: Type, visited: &mut Vec<Type>) -> bool {
        let ty = self.shallow(ty);
        if !visited.contains(&ty) {
            visited.push(ty);
        } else {
            return false;
        }
        match ty {
            Type::Variable(candidate) => self.root(candidate) == variable,
            Type::Array(array) => self.occurs_in(variable, self.array_element(array), visited),
            Type::Set(set) => self.occurs_in(variable, self.set_element(set), visited),
            Type::Option(option) => self.occurs_in(variable, self.option_value(option), visited),
            Type::Result(result) => self.occurs_in(variable, self.result_value(result), visited),
            Type::Async(future) => self.occurs_in(variable, self.async_value(future), visited),
            Type::Known(_) => false,
        }
    }

    fn validate_binding(&self, variable: u32, ty: Type) -> Result<(), InferenceError> {
        let inference = &self.variables[variable as usize];
        if !type_meets_requirements(
            &self.standard_library,
            &self.types,
            ty,
            &inference.requirements,
        ) {
            return Err(InferenceError::UnsatisfiedConstraints {
                ty,
                requirements: inference.requirements.clone(),
            });
        }
        if let Some(literal) = inference.largest_literal
            && !fits_unsigned_literal(&self.types, literal, ty)
        {
            return Err(InferenceError::IntegerLiteralOutOfRange(ty));
        }
        Ok(())
    }
}

fn fits_unsigned_literal(types: &TypeStore, value: u64, ty: Type) -> bool {
    let Type::Known(id) = ty else {
        return false;
    };
    match types.kind(id) {
        TypeKind::Error => false,
        TypeKind::Builtin(BuiltinType::Bool) => value <= 1,
        TypeKind::Builtin(BuiltinType::U8) => u8::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I8) => value <= i8::MAX as u64,
        TypeKind::Builtin(BuiltinType::U16) => u16::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I16) => value <= i16::MAX as u64,
        TypeKind::Builtin(BuiltinType::U32) => u32::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I32) => value <= i32::MAX as u64,
        TypeKind::Builtin(BuiltinType::U64 | BuiltinType::Address) => true,
        TypeKind::Builtin(BuiltinType::I64) => value <= i64::MAX as u64,
        TypeKind::Builtin(BuiltinType::F32) => integer_is_exact_at_precision(value, 24),
        TypeKind::Builtin(BuiltinType::F64) => integer_is_exact_at_precision(value, 53),
        _ => false,
    }
}

fn integer_is_exact_at_precision(value: u64, precision: u32) -> bool {
    let significant_bits = u64::BITS - value.leading_zeros();
    significant_bits <= precision || value.trailing_zeros() >= significant_bits - precision
}

fn type_meets_requirements(
    standard_library: &StandardLibrary,
    types: &TypeStore,
    ty: Type,
    requirements: &Requirements,
) -> bool {
    if matches!(ty, Type::Variable(_)) {
        return true;
    }
    if matches!(
        ty,
        Type::Known(id) if matches!(types.kind(id), TypeKind::Error)
    ) {
        return true;
    }
    requirements
        .0
        .iter()
        .all(|capability| type_may_have_capability(standard_library, types, ty, *capability))
}

/// Conservative pre-semantic admissibility check. Derived capabilities are
/// proven recursively by `CapabilityAnalysis` once inference produces a
/// semantic TypeId.
pub(crate) fn type_may_have_capability(
    library: &StandardLibrary,
    types: &TypeStore,
    ty: Type,
    capability: StdlibCapabilityId,
) -> bool {
    let behavior = library.capability(capability).behavior;
    match ty {
        Type::Known(id) => match types.kind(id) {
            TypeKind::Error => false,
            TypeKind::Builtin(builtin) => library.core_type_has_capability(*builtin, capability),
            TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
            TypeKind::StateSnapshot | TypeKind::SettingsView => false,
            TypeKind::Record(_) => matches!(
                behavior,
                CapabilityBehavior::StructuralEquality | CapabilityBehavior::StructuralMemoryLayout
            ),
            TypeKind::Enum(_) | TypeKind::Option { .. } | TypeKind::Result { .. } => {
                behavior == CapabilityBehavior::StructuralEquality
            }
            TypeKind::Async { .. } => false,
            TypeKind::Array { length, .. } => {
                behavior == CapabilityBehavior::StructuralMemoryLayout && length.is_some()
            }
            TypeKind::Set { .. } => false,
            TypeKind::GenericParameter { .. } => false,
        },
        Type::Option(_) | Type::Result(_) => behavior == CapabilityBehavior::StructuralEquality,
        Type::Async(_) => false,
        Type::Array(_) => behavior == CapabilityBehavior::StructuralMemoryLayout,
        Type::Set(_) => false,
        Type::Variable(_) => false,
    }
}

fn canonical_constructed_type(
    ty: Type,
    arrays: &HashMap<ArrayTypeId, ArrayTypeId>,
    options: &HashMap<OptionTypeId, OptionTypeId>,
    results: &HashMap<ResultTypeId, ResultTypeId>,
    asyncs: &HashMap<AsyncTypeId, AsyncTypeId>,
    sets: &HashMap<TypeApplicationId, TypeApplicationId>,
) -> Type {
    match ty {
        Type::Array(array) => Type::Array(arrays.get(&array).copied().unwrap_or(array)),
        Type::Option(option) => Type::Option(options.get(&option).copied().unwrap_or(option)),
        Type::Result(result) => Type::Result(results.get(&result).copied().unwrap_or(result)),
        Type::Async(future) => Type::Async(asyncs.get(&future).copied().unwrap_or(future)),
        Type::Set(set) => Type::Set(sets.get(&set).copied().unwrap_or(set)),
        ty => ty,
    }
}

fn requirements_are_possible(
    library: &StandardLibrary,
    types: &TypeStore,
    requirements: &Requirements,
) -> bool {
    library
        .core_types()
        .iter()
        .map(|ty| Type::Known(types.id_for_core(ty.id)))
        .chain(
            library
                .types()
                .iter()
                .map(|ty| Type::Known(types.id_for_standard(ty.id))),
        )
        .any(|ty| type_meets_requirements(library, types, ty, requirements))
}

fn error(message: impl Into<String>) -> InferenceError {
    InferenceError::Message(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unifies_bidirectionally_and_checks_literal_bounds() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let value = inference.fresh(
            Requirements::capability(StdlibCapabilityId::Integer),
            Some(256),
        );
        let alias = inference.fresh(Requirements::none(), None);
        inference.unify(alias, value).unwrap();
        assert!(
            inference
                .unify(alias, inference.known_builtin(BuiltinType::U8))
                .is_err()
        );
        let u16_type = inference.known_builtin(BuiltinType::U16);
        inference.unify(alias, u16_type).unwrap();
        assert_eq!(inference.resolve(value), u16_type);
    }

    #[test]
    fn broad_capability_constraints_do_not_choose_a_numeric_representation() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let value = inference.fresh(Requirements::capability(StdlibCapabilityId::Signed), None);
        inference
            .require(value, Requirements::capability(StdlibCapabilityId::Numeric))
            .unwrap();
        assert_eq!(inference.default_unbound().len(), 1);
        inference.recover_unbound();
        let Type::Known(recovered) = inference.resolve(value) else {
            panic!("recovery must publish a semantic type")
        };
        assert_eq!(inference.type_store().kind(recovered), &TypeKind::Error);
    }

    #[test]
    fn integer_and_float_capabilities_choose_the_language_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(Requirements::capability(StdlibCapabilityId::Integer), None);
        let float = inference.fresh(Requirements::capability(StdlibCapabilityId::Float), None);
        assert!(inference.default_unbound().is_empty());
        assert_eq!(
            inference.resolve(integer),
            inference.known_builtin(BuiltinType::I32)
        );
        assert_eq!(
            inference.resolve(float),
            inference.known_builtin(BuiltinType::F64)
        );
    }

    #[test]
    fn memory_readable_suppresses_literal_and_capability_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(
            Requirements::capabilities([
                StdlibCapabilityId::MemoryReadable,
                StdlibCapabilityId::Integer,
            ]),
            Some(1),
        );
        let float = inference.fresh(
            Requirements::capabilities([
                StdlibCapabilityId::MemoryReadable,
                StdlibCapabilityId::Float,
            ]),
            None,
        );
        let errors = inference.default_unbound();
        assert_eq!(errors.len(), 2);
        assert!(inference.is_unbound_without_default(integer));
        assert!(inference.is_unbound_without_default(float));
    }

    #[test]
    fn unsuffixed_literals_use_the_language_numeric_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(
            Requirements::capability(StdlibCapabilityId::Numeric),
            Some(7),
        );
        let float = inference.fresh_float_literal();
        assert!(inference.default_unbound().is_empty());
        let i32_type = inference.known_builtin(BuiltinType::I32);
        let f64_type = inference.known_builtin(BuiltinType::F64);
        assert_eq!(inference.resolve(integer), i32_type);
        assert_eq!(inference.resolve(float), f64_type);
    }

    #[test]
    fn capability_requirements_are_not_limited_to_well_known_bit_positions() {
        let loaded = StdlibCapabilityId::from_u32(10_000);
        let requirements = Requirements::capabilities([
            StdlibCapabilityId::Numeric,
            loaded,
            StdlibCapabilityId::Numeric,
        ]);

        assert!(requirements.contains(StdlibCapabilityId::Numeric));
        assert!(requirements.contains(loaded));
        assert_eq!(requirements.0.len(), 2);
    }

    #[test]
    fn scheme_instantiation_preserves_constraints_sharing_and_nested_shapes() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let template = inference.fresh(Requirements::capability(StdlibCapabilityId::Numeric), None);
        let Type::Variable(root) = template else {
            unreachable!()
        };
        let array = inference.array_type(template);
        let option = inference.option_type(Type::Array(array));
        let result = inference.result_type(Type::Option(option));

        let mut first_substitutions = HashMap::new();
        let first_value = inference.instantiate_type(template, &[root], &mut first_substitutions);
        let first_nested =
            inference.instantiate_type(Type::Result(result), &[root], &mut first_substitutions);
        let Type::Result(first_result) = first_nested else {
            unreachable!()
        };
        let Type::Option(first_option) = inference.result_value(first_result) else {
            unreachable!()
        };
        let Type::Array(first_array) = inference.option_value(first_option) else {
            unreachable!()
        };
        assert_eq!(inference.array_element(first_array), first_value);

        let mut second_substitutions = HashMap::new();
        let second_value = inference.instantiate_type(template, &[root], &mut second_substitutions);
        assert_ne!(first_value, second_value);

        let i64_type = inference.known_builtin(BuiltinType::I64);
        inference.unify(first_value, i64_type).unwrap();
        let string_type = inference.known_standard(StdlibTypeId::String);
        assert!(inference.unify(second_value, string_type).is_err());
    }

    #[test]
    fn interns_resolved_constructed_types_before_semantic_publication() {
        let mut ids = ConstructedTypeIdAllocator::starting_at(0);
        let array = ids.array();
        let option = ids.option();
        let result = ids.result();
        let types = TypeStore::default();
        let u32_type = Type::Known(types.id_for_builtin(BuiltinType::U32));
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            types,
            3,
            ConstructedLayouts {
                arrays: vec![ArrayLayout {
                    id: array,
                    element: u32_type,
                    length: None,
                }],
                options: vec![OptionLayout {
                    id: option,
                    value: Type::Array(array),
                }],
                results: vec![ResultLayout {
                    id: result,
                    value: Type::Option(option),
                }],
                ..ConstructedLayouts::default()
            },
        );

        inference.finalize_arrays();
        inference.finalize_wrappers();
        inference.intern_resolved_constructed_types();

        let Type::Known(array_type) = inference.resolve(Type::Array(array)) else {
            panic!("resolved array did not become a semantic TypeId")
        };
        let Type::Known(option_type) = inference.resolve(Type::Option(option)) else {
            panic!("resolved option did not become a semantic TypeId")
        };
        let Type::Known(result_type) = inference.resolve(Type::Result(result)) else {
            panic!("resolved result did not become a semantic TypeId")
        };

        assert_eq!(
            inference.type_store().kind(array_type),
            &TypeKind::Array {
                layout: array,
                element: match u32_type {
                    Type::Known(id) => id,
                    _ => unreachable!(),
                },
                length: None,
            }
        );
        assert_eq!(
            inference.type_store().kind(option_type),
            &TypeKind::Option {
                layout: option,
                value: array_type,
            }
        );
        assert_eq!(
            inference.type_store().kind(result_type),
            &TypeKind::Result {
                layout: result,
                value: option_type,
            }
        );
    }
}
