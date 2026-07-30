//! Temporary types and reusable whole-program type inference.
//!
//! Source syntax never contains these types. The checker records constraints
//! here and publishes only fully resolved [`crate::types::TypeId`] values.

use std::{collections::HashMap, fmt, ops::BitOr};

use crate::{
    ast::{ArrayTypeId, EnumId, OptionTypeId, RecordId, ResultTypeId, TypeRef},
    stdlib::{CoreTypeId, StandardLibrary, StdlibCapabilityId, StdlibTypeId},
    types::{TypeId, TypeKind, TypeStore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Type {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Address,
    F32,
    F64,
    Known(TypeId),
    Record(RecordId),
    Enum(EnumId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
    Variable(u32),
}

impl Type {
    pub(crate) fn from_core(core: CoreTypeId) -> Self {
        match core {
            CoreTypeId::Void => Self::Void,
            CoreTypeId::Bool => Self::Bool,
            CoreTypeId::I8 => Self::I8,
            CoreTypeId::U8 => Self::U8,
            CoreTypeId::I16 => Self::I16,
            CoreTypeId::U16 => Self::U16,
            CoreTypeId::I32 => Self::I32,
            CoreTypeId::U32 => Self::U32,
            CoreTypeId::I64 => Self::I64,
            CoreTypeId::U64 => Self::U64,
            CoreTypeId::Address => Self::Address,
            CoreTypeId::F32 => Self::F32,
            CoreTypeId::F64 => Self::F64,
        }
    }

    pub(crate) fn core(self) -> Option<CoreTypeId> {
        Some(match self {
            Self::Void => CoreTypeId::Void,
            Self::Bool => CoreTypeId::Bool,
            Self::I8 => CoreTypeId::I8,
            Self::U8 => CoreTypeId::U8,
            Self::I16 => CoreTypeId::I16,
            Self::U16 => CoreTypeId::U16,
            Self::I32 => CoreTypeId::I32,
            Self::U32 => CoreTypeId::U32,
            Self::I64 => CoreTypeId::I64,
            Self::U64 => CoreTypeId::U64,
            Self::Address => CoreTypeId::Address,
            Self::F32 => CoreTypeId::F32,
            Self::F64 => CoreTypeId::F64,
            _ => return None,
        })
    }

    pub(crate) fn is_integer(self) -> bool {
        self.core().is_some_and(|core| {
            StandardLibrary::new().core_type_has_capability(core, StdlibCapabilityId::Integer)
        })
    }

    pub(crate) fn is_numeric(self) -> bool {
        self.core().is_some_and(|core| {
            StandardLibrary::new().core_type_has_capability(core, StdlibCapabilityId::Numeric)
        })
    }

    pub(crate) fn to_ref(self, types: &TypeStore) -> TypeRef {
        match self {
            Self::Void => TypeRef::Void,
            Self::Bool => TypeRef::Bool,
            Self::I8 => TypeRef::I8,
            Self::U8 => TypeRef::U8,
            Self::I16 => TypeRef::I16,
            Self::U16 => TypeRef::U16,
            Self::I32 => TypeRef::I32,
            Self::U32 => TypeRef::U32,
            Self::I64 => TypeRef::I64,
            Self::U64 => TypeRef::U64,
            Self::Address => TypeRef::Address,
            Self::F32 => TypeRef::F32,
            Self::F64 => TypeRef::F64,
            Self::Known(id) => match types.kind(id) {
                TypeKind::Standard(standard) => TypeRef::Standard(*standard),
                TypeKind::Record(record) => TypeRef::Record(*record),
                TypeKind::Enum(enumeration) => TypeRef::Enum(*enumeration),
                kind => unreachable!("unsupported known inference type `{kind:?}`"),
            },
            Self::Record(id) => TypeRef::Record(id),
            Self::Enum(id) => TypeRef::Enum(id),
            Self::Array(id) => TypeRef::Array(id),
            Self::Option(id) => TypeRef::Option(id),
            Self::Result(id) => TypeRef::Result(id),
            Self::Variable(variable) => {
                unreachable!("inference variable ?{variable} cannot become a source type reference")
            }
        }
    }
}

impl From<TypeRef> for Type {
    fn from(ty: TypeRef) -> Self {
        match ty {
            TypeRef::Void => Self::Void,
            TypeRef::Bool => Self::Bool,
            TypeRef::I8 => Self::I8,
            TypeRef::U8 => Self::U8,
            TypeRef::I16 => Self::I16,
            TypeRef::U16 => Self::U16,
            TypeRef::I32 => Self::I32,
            TypeRef::U32 => Self::U32,
            TypeRef::I64 => Self::I64,
            TypeRef::U64 => Self::U64,
            TypeRef::Address => Self::Address,
            TypeRef::F32 => Self::F32,
            TypeRef::F64 => Self::F64,
            TypeRef::Named(id) => {
                unreachable!("source nominal type name {id} must be resolved before inference")
            }
            TypeRef::Standard(standard) => {
                unreachable!("standard type {standard:?} must be interned before inference")
            }
            TypeRef::Record(id) => Self::Record(id),
            TypeRef::Enum(id) => Self::Enum(id),
            TypeRef::Array(id) => Self::Array(id),
            TypeRef::Option(id) => Self::Option(id),
            TypeRef::Result(id) => Self::Result(id),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Address => "address",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Known(id) => return write!(formatter, "type#{}", id.index()),
            Self::Record(id) => return write!(formatter, "record#{id}"),
            Self::Enum(id) => return write!(formatter, "enum#{id}"),
            Self::Array(id) => return write!(formatter, "Array#{id}"),
            Self::Option(id) => return write!(formatter, "Option#{id}"),
            Self::Result(id) => return write!(formatter, "Result#{id}"),
            Self::Variable(id) => return write!(formatter, "?{id}"),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Requirements(u8);

impl Requirements {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const EQUATABLE: Self = Self(1 << 0);
    pub(crate) const NUMERIC: Self = Self(1 << 1);
    pub(crate) const INTEGER: Self = Self(1 << 2);
    pub(crate) const SIGNED: Self = Self(1 << 3);
    pub(crate) const FLOAT: Self = Self(1 << 4);
    pub(crate) const STRING_CAST: Self = Self(1 << 5);
    pub(crate) const MEMORY_READABLE: Self = Self(1 << 6);
    pub(crate) const INTERPOLATABLE: Self = Self(1 << 7);

    fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Requirements {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum InferenceError {
    Message(String),
    TypeMismatch { left: Type, right: Type },
    UnsupportedOperation(Type),
    UnsatisfiedConstraints(Type),
    IntegerLiteralOutOfRange(Type),
}

#[derive(Debug, Clone)]
struct Variable {
    parent: u32,
    binding: Option<Type>,
    requirements: Requirements,
    largest_literal: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrayLayout {
    pub(crate) id: ArrayTypeId,
    pub(crate) element: Type,
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

pub(crate) struct InferenceContext {
    types: TypeStore,
    variables: Vec<Variable>,
    arrays: Vec<ArrayLayout>,
    options: Vec<OptionLayout>,
    results: Vec<ResultLayout>,
    canonical_options: HashMap<OptionTypeId, OptionTypeId>,
    canonical_results: HashMap<ResultTypeId, ResultTypeId>,
    next_constructed_type_index: u32,
}

impl InferenceContext {
    pub(crate) fn new(
        types: TypeStore,
        first_constructed_type_index: u32,
        arrays: impl IntoIterator<Item = ArrayLayout>,
        options: impl IntoIterator<Item = OptionLayout>,
        results: impl IntoIterator<Item = ResultLayout>,
    ) -> Self {
        let arrays = arrays.into_iter().collect::<Vec<_>>();
        let options = options.into_iter().collect::<Vec<_>>();
        let results = results.into_iter().collect::<Vec<_>>();
        let next_constructed_type_index = arrays
            .iter()
            .map(|layout| layout.id.index() as u32 + 1)
            .chain(options.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(results.iter().map(|layout| layout.id.index() as u32 + 1))
            .fold(first_constructed_type_index, u32::max);
        Self {
            types,
            variables: Vec::new(),
            arrays,
            options,
            results,
            canonical_options: HashMap::new(),
            canonical_results: HashMap::new(),
            next_constructed_type_index,
        }
    }

    pub(crate) fn known_standard(&self, standard: StdlibTypeId) -> Type {
        Type::Known(self.types.id_for_standard(standard))
    }

    pub(crate) fn standard_type(&self, ty: Type) -> Option<StdlibTypeId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.types.kind(id) {
            TypeKind::Standard(standard) => Some(*standard),
            _ => None,
        }
    }

    pub(crate) fn known_type_name(&self, id: TypeId) -> String {
        match self.types.kind(id) {
            TypeKind::Builtin(builtin) => builtin.to_string(),
            TypeKind::Standard(standard) => StandardLibrary::new().type_decl(*standard).name.into(),
            TypeKind::Record(record) => format!("record#{record}"),
            TypeKind::Enum(enumeration) => format!("enum#{enumeration}"),
            TypeKind::Array { .. } => "Array".into(),
            TypeKind::Option { .. } => "Option".into(),
            TypeKind::Result { .. } => "Result".into(),
        }
    }

    pub(crate) fn fresh(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
    ) -> Type {
        let id = self.variables.len() as u32;
        self.variables.push(Variable {
            parent: id,
            binding: None,
            requirements,
            largest_literal,
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

    pub(crate) fn is_unbound_without_default(&mut self, ty: Type) -> bool {
        let Type::Variable(variable) = self.shallow(ty) else {
            return false;
        };
        let requirements = self.variables[variable as usize].requirements;
        !requirements.intersects(
            Requirements::FLOAT
                | Requirements::INTEGER
                | Requirements::NUMERIC
                | Requirements::SIGNED
                | Requirements::STRING_CAST
                | Requirements::INTERPOLATABLE,
        )
    }

    pub(crate) fn unify(&mut self, left: Type, right: Type) -> Result<Type, InferenceError> {
        let left = self.shallow(left);
        let right = self.shallow(right);
        match (left, right) {
            (Type::Variable(left), Type::Variable(right)) => self.unify_variables(left, right),
            (Type::Variable(variable), ty) | (ty, Type::Variable(variable)) => {
                self.bind(variable, ty)
            }
            (Type::Array(left), Type::Array(right)) => {
                let left_element = self.array_element(left);
                let right_element = self.array_element(right);
                self.unify(left_element, right_element)?;
                Ok(Type::Array(left))
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
            (left, right) if left == right => Ok(left),
            (left, right) => Err(InferenceError::TypeMismatch { left, right }),
        }
    }

    pub(crate) fn require(
        &mut self,
        ty: Type,
        requirements: Requirements,
    ) -> Result<(), InferenceError> {
        match self.shallow(ty) {
            Type::Variable(variable) => {
                let variable = self.root(variable);
                let combined = self.variables[variable as usize].requirements | requirements;
                if !requirements_are_possible(&self.types, combined) {
                    return Err(error("incompatible type constraints"));
                }
                self.variables[variable as usize].requirements = combined;
                Ok(())
            }
            concrete if type_meets_requirements(&self.types, concrete, requirements) => Ok(()),
            concrete => Err(InferenceError::UnsupportedOperation(concrete)),
        }
    }

    pub(crate) fn default_unbound(&mut self) -> Vec<InferenceError> {
        let mut errors = Vec::new();
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root != id || self.variables[root as usize].binding.is_some() {
                continue;
            }
            let requirements = self.variables[root as usize].requirements;
            let default = if requirements.intersects(Requirements::FLOAT) {
                Some(Type::F64)
            } else if requirements.intersects(
                Requirements::INTEGER
                    | Requirements::NUMERIC
                    | Requirements::SIGNED
                    | Requirements::STRING_CAST
                    | Requirements::INTERPOLATABLE,
            ) {
                Some(Type::I32)
            } else {
                None
            };
            let Some(default) = default else {
                errors.push(error(format!(
                    "cannot infer type variable `?{root}` without more context"
                )));
                continue;
            };
            match self.validate_binding(root, default) {
                Ok(()) => self.variables[root as usize].binding = Some(default),
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    /// Gives every still-unbound variable a recovery-only representation so
    /// editor queries can retain independent semantic facts after diagnostics.
    /// Strict checking never observes these fallback bindings.
    pub(crate) fn recover_unbound(&mut self) {
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root == id && self.variables[root as usize].binding.is_none() {
                self.variables[root as usize].binding = Some(Type::I32);
            }
        }
    }

    pub(crate) fn resolve(&mut self, ty: Type) -> Type {
        match self.shallow(ty) {
            Type::Variable(variable) => {
                panic!("unresolved type variable ?{variable} after inference")
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
            ty => ty,
        }
    }

    pub(crate) fn array_type(&mut self, element: Type) -> ArrayTypeId {
        if let Some(array) = self.arrays.iter().find(|array| array.element == element) {
            return array.id;
        }
        let id = ArrayTypeId::from_index(self.next_constructed_type_index);
        self.next_constructed_type_index += 1;
        self.arrays.push(ArrayLayout { id, element });
        id
    }

    pub(crate) fn array_element(&self, id: ArrayTypeId) -> Type {
        self.arrays
            .iter()
            .find(|array| array.id == id)
            .expect("checked array type has a layout")
            .element
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

    pub(crate) fn option_type(&mut self, value: Type) -> OptionTypeId {
        if let Some(option) = self.options.iter().find(|option| option.value == value) {
            return option.id;
        }
        let id = OptionTypeId::from_index(self.next_constructed_type_index);
        self.next_constructed_type_index += 1;
        self.options.push(OptionLayout { id, value });
        id
    }

    pub(crate) fn result_type(&mut self, value: Type) -> ResultTypeId {
        if let Some(result) = self.results.iter().find(|result| result.value == value) {
            return result.id;
        }
        let id = ResultTypeId::from_index(self.next_constructed_type_index);
        self.next_constructed_type_index += 1;
        self.results.push(ResultLayout { id, value });
        id
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

        // Constructors can allocate a provisional wrapper before later uses
        // constrain its value type. Once all inference variables resolve,
        // collapse layouts with identical value types to the first stable ID.
        // This keeps WebAssembly GC's nominal type identities aligned with the
        // structural unification performed above.
        loop {
            let previous_options = self.canonical_options.clone();
            let previous_results = self.canonical_results.clone();
            self.canonical_options.clear();
            self.canonical_results.clear();

            let mut option_representatives = Vec::<(Type, OptionTypeId)>::new();
            for option in &self.options {
                let value =
                    canonical_wrapper_type(option.value, &previous_options, &previous_results);
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
                let value =
                    canonical_wrapper_type(result.value, &previous_options, &previous_results);
                let canonical = result_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        result_representatives.push((value, result.id));
                        result.id
                    });
                self.canonical_results.insert(result.id, canonical);
            }

            if self.canonical_options == previous_options
                && self.canonical_results == previous_results
            {
                break;
            }
        }

        let canonical_options = self.canonical_options.clone();
        let canonical_results = self.canonical_results.clone();
        for option in &mut self.options {
            option.value =
                canonical_wrapper_type(option.value, &canonical_options, &canonical_results);
        }
        for result in &mut self.results {
            result.value =
                canonical_wrapper_type(result.value, &canonical_options, &canonical_results);
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
        self.variables[left as usize].requirements =
            left_variable.requirements | right_variable.requirements;
        self.variables[left as usize].largest_literal = left_variable
            .largest_literal
            .max(right_variable.largest_literal);
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
        } else if requirements_are_possible(&self.types, self.variables[left as usize].requirements)
        {
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
        self.validate_binding(variable, ty)?;
        self.variables[variable as usize].binding = Some(ty);
        Ok(ty)
    }

    fn validate_binding(&self, variable: u32, ty: Type) -> Result<(), InferenceError> {
        let inference = &self.variables[variable as usize];
        if !type_meets_requirements(&self.types, ty, inference.requirements) {
            return Err(InferenceError::UnsatisfiedConstraints(ty));
        }
        if let Some(literal) = inference.largest_literal
            && !fits_unsigned_literal(literal, ty)
        {
            return Err(InferenceError::IntegerLiteralOutOfRange(ty));
        }
        Ok(())
    }
}

pub(crate) fn fits_unsigned_literal(value: u64, ty: Type) -> bool {
    match ty {
        Type::Bool => value <= 1,
        Type::U8 => u8::try_from(value).is_ok(),
        Type::I8 => value <= i8::MAX as u64,
        Type::U16 => u16::try_from(value).is_ok(),
        Type::I16 => value <= i16::MAX as u64,
        Type::U32 => u32::try_from(value).is_ok(),
        Type::I32 => value <= i32::MAX as u64,
        Type::U64 | Type::Address => true,
        Type::I64 => value <= i64::MAX as u64,
        _ => false,
    }
}

fn type_meets_requirements(types: &TypeStore, ty: Type, requirements: Requirements) -> bool {
    if matches!(ty, Type::Variable(_)) {
        return true;
    }
    [
        (Requirements::EQUATABLE, StdlibCapabilityId::Equatable),
        (Requirements::NUMERIC, StdlibCapabilityId::Numeric),
        (Requirements::INTEGER, StdlibCapabilityId::Integer),
        (Requirements::SIGNED, StdlibCapabilityId::Signed),
        (Requirements::FLOAT, StdlibCapabilityId::Float),
        (Requirements::STRING_CAST, StdlibCapabilityId::StringCast),
        (
            Requirements::INTERPOLATABLE,
            StdlibCapabilityId::Interpolatable,
        ),
        (
            Requirements::MEMORY_READABLE,
            StdlibCapabilityId::MemoryReadable,
        ),
    ]
    .into_iter()
    .all(|(requirement, capability)| {
        !requirements.intersects(requirement) || type_may_have_capability(types, ty, capability)
    })
}

/// Conservative pre-semantic admissibility check. Derived capabilities are
/// proven recursively by `CapabilityAnalysis` once inference produces a
/// semantic TypeId.
pub(crate) fn type_may_have_capability(
    types: &TypeStore,
    ty: Type,
    capability: StdlibCapabilityId,
) -> bool {
    let library = StandardLibrary::new();
    if let Some(core) = ty.core() {
        return library.core_type_has_capability(core, capability);
    }
    match ty {
        Type::Known(id) => match types.kind(id) {
            TypeKind::Builtin(builtin) => {
                library.core_type_has_capability(builtin.core(), capability)
            }
            TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
            TypeKind::Record(_) => matches!(
                capability,
                StdlibCapabilityId::Equatable | StdlibCapabilityId::MemoryReadable
            ),
            TypeKind::Enum(_) | TypeKind::Option { .. } | TypeKind::Result { .. } => {
                capability == StdlibCapabilityId::Equatable
            }
            TypeKind::Array { .. } => false,
        },
        Type::Record(_) => matches!(
            capability,
            StdlibCapabilityId::Equatable | StdlibCapabilityId::MemoryReadable
        ),
        Type::Enum(_) | Type::Option(_) | Type::Result(_) => {
            capability == StdlibCapabilityId::Equatable
        }
        Type::Array(_) | Type::Variable(_) => false,
        _ => unreachable!("core types were handled above"),
    }
}

fn canonical_wrapper_type(
    ty: Type,
    options: &HashMap<OptionTypeId, OptionTypeId>,
    results: &HashMap<ResultTypeId, ResultTypeId>,
) -> Type {
    match ty {
        Type::Option(option) => Type::Option(options.get(&option).copied().unwrap_or(option)),
        Type::Result(result) => Type::Result(results.get(&result).copied().unwrap_or(result)),
        ty => ty,
    }
}

fn requirements_are_possible(types: &TypeStore, requirements: Requirements) -> bool {
    let library = StandardLibrary::new();
    library
        .core_types()
        .iter()
        .map(|ty| Type::from_core(ty.id))
        .chain(
            library
                .types()
                .iter()
                .map(|ty| Type::Known(types.id_for_standard(ty.id))),
        )
        .any(|ty| type_meets_requirements(types, ty, requirements))
}

fn error(message: impl Into<String>) -> InferenceError {
    InferenceError::Message(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unifies_bidirectionally_and_checks_literal_bounds() {
        let mut inference = InferenceContext::new(TypeStore::default(), 0, [], [], []);
        let value = inference.fresh(Requirements::INTEGER, Some(256));
        let alias = inference.fresh(Requirements::NONE, None);
        inference.unify(alias, value).unwrap();
        assert!(inference.unify(alias, Type::U8).is_err());
        inference.unify(alias, Type::U16).unwrap();
        assert_eq!(inference.resolve(value), Type::U16);
    }

    #[test]
    fn combines_constraints_and_defaults_numeric_variables() {
        let mut inference = InferenceContext::new(TypeStore::default(), 0, [], [], []);
        let value = inference.fresh(Requirements::SIGNED, None);
        inference.require(value, Requirements::NUMERIC).unwrap();
        assert!(inference.default_unbound().is_empty());
        assert_eq!(inference.resolve(value), Type::I32);
    }
}
