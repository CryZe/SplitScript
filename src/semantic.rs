//! Semantic facts produced by type checking and consumed by later stages.

use std::collections::HashMap;

use crate::{
    ast::{
        ArrayTypeId, AssignmentId, EnumVariantId, ExprId, FunctionId, OptionTypeId, PatternId,
        RecordFieldId, RecordId, ResultTypeId, SettingChoiceOptionId, ValueId,
    },
    inference::Type,
    stdlib::{StdlibFieldId, StdlibItemId, StdlibStateProviderId, StdlibTypeId, StdlibVariantId},
    types::{
        ResolvedArrayType, ResolvedOptionType, ResolvedResultType, TypeId, TypeKind, TypeStore,
    },
};

/// A concrete instantiation of a source function.
///
/// Monomorphic functions use an empty argument vector. Generic instances also
/// retain their exact concrete parameter/result signature because nominal GC
/// layouts are not recoverable from type arguments alone. This identity lives
/// at the semantic boundary so typed HIR, reachability, and Wasm emission agree
/// on every concrete body without inventing backend-only function IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionInstance {
    pub function: FunctionId,
    pub type_arguments: Vec<TypeId>,
    pub signature: Vec<TypeId>,
}

impl FunctionInstance {
    pub fn monomorphic(function: FunctionId) -> Self {
        Self {
            function,
            type_arguments: Vec::new(),
            signature: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCall {
    UserFunction {
        function: FunctionId,
        type_arguments: Vec<TypeId>,
        signature: Vec<TypeId>,
    },
    UserMethod {
        function: FunctionId,
        type_arguments: Vec<TypeId>,
        signature: Vec<TypeId>,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<TypeId>,
        /// Concrete receiver/parameter types followed by the declared result.
        /// Library source bodies use this to instantiate their inferred hidden
        /// function template without reconstructing catalog types downstream.
        signature: Vec<TypeId>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<TypeId>,
    },
    ResultError {
        result: crate::ast::ResultTypeId,
    },
    OptionSome {
        option: crate::ast::OptionTypeId,
    },
    ResultSuccess {
        result: crate::ast::ResultTypeId,
    },
}

fn resolved_option_layout(ty: Type, types: &TypeStore) -> OptionTypeId {
    match ty {
        Type::Option(layout) => layout,
        Type::Known(id) => match types.kind(id) {
            TypeKind::Option { layout, .. } => *layout,
            kind => unreachable!("Some/None resolved to non-Option type `{kind:?}`"),
        },
        ty => unreachable!("Some/None resolved to non-Option inference term `{ty}`"),
    }
}

fn resolved_result_layout(ty: Type, types: &TypeStore) -> ResultTypeId {
    match ty {
        Type::Result(layout) => layout,
        Type::Known(id) => match types.kind(id) {
            TypeKind::Result { layout, .. } => *layout,
            kind => unreachable!("result constructor resolved to non-Result type `{kind:?}`"),
        },
        ty => unreachable!("result constructor resolved to non-Result inference term `{ty}`"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueConversionKind {
    LiftOption,
    LiftResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueConversion {
    pub kind: ValueConversionKind,
    pub source: TypeId,
    pub target: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedValue {
    ProviderValue(StdlibStateProviderId),
    Variable(ValueId),
    CurrentSnapshot,
    OldSnapshot,
    SettingsView,
    OldSettingsView,
    CurrentState(ValueId),
    OldState(ValueId),
    Setting(ValueId),
    OldSetting(ValueId),
}

impl ResolvedValue {
    /// Returns the source value identity read by this resolution. Provider
    /// values are compiler/catalog-owned roots and have no source declaration.
    pub fn source_value(self) -> Option<ValueId> {
        match self {
            Self::ProviderValue(_)
            | Self::CurrentSnapshot
            | Self::OldSnapshot
            | Self::SettingsView
            | Self::OldSettingsView => None,
            Self::Variable(value)
            | Self::CurrentState(value)
            | Self::OldState(value)
            | Self::Setting(value)
            | Self::OldSetting(value) => Some(value),
        }
    }
}

impl ResolvedCall {
    /// Returns the receiver resolution for method-shaped calls.
    pub fn receiver(&self) -> Option<&ResolvedReceiver> {
        match self {
            Self::UserMethod { receiver, .. } => Some(receiver),
            Self::StandardLibrary { receiver, .. } => receiver.as_ref(),
            Self::UserFunction { .. }
            | Self::ResultError { .. }
            | Self::OptionSome { .. }
            | Self::ResultSuccess { .. } => None,
        }
    }
}

/// The value on which a method is invoked.
///
/// Plain source paths retain their declaration root and resolved fields for
/// navigation and direct lowering. General postfix calls instead retain the
/// receiver expression, which is evaluated exactly once before the explicit
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedReceiver {
    Path {
        root: ResolvedValue,
        members: Vec<ResolvedMember>,
    },
    Expression {
        expression: ExprId,
        members: Vec<ResolvedMember>,
    },
}

impl ResolvedReceiver {
    pub fn path(&self) -> Option<(ResolvedValue, &[ResolvedMember])> {
        match self {
            Self::Path { root, members } => Some((*root, members)),
            Self::Expression { .. } => None,
        }
    }

    pub fn expression(&self) -> Option<ExprId> {
        match self {
            Self::Expression { expression, .. } => Some(*expression),
            Self::Path { .. } => None,
        }
    }

    pub fn members(&self) -> &[ResolvedMember] {
        match self {
            Self::Path { members, .. } | Self::Expression { members, .. } => members,
        }
    }
}

/// A field selected after the root of a resolved value path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedMember {
    StateField(ValueId),
    SettingField(ValueId),
    RecordField(RecordFieldId),
    StandardField(StdlibFieldId),
}

/// Stable identity of an enum variant selected by checked source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedEnumVariantId {
    Source(EnumVariantId),
    Standard(StdlibVariantId),
}

/// Nominal record selected by a checked record literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedRecordId {
    Source(RecordId),
    Standard(StdlibTypeId),
}

impl std::fmt::Display for ResolvedRecordId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(record) => record.fmt(formatter),
            Self::Standard(record) => write!(formatter, "{record:?}"),
        }
    }
}

/// Field selected by a checked record literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedRecordFieldId {
    Source(RecordFieldId),
    Standard(StdlibFieldId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedWrapperPattern {
    OptionNone(OptionTypeId),
    OptionSome(OptionTypeId),
    ResultSuccess(ResultTypeId),
    ResultError(ResultTypeId),
}

#[derive(Debug, Clone, Default)]
pub struct SemanticModel {
    types: TypeStore,
    state_provider: Option<StdlibStateProviderId>,
    expression_types: HashMap<ExprId, TypeId>,
    calls: HashMap<ExprId, ResolvedCall>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, TypeId>,
    function_results: HashMap<FunctionId, TypeId>,
    function_parameter_types: HashMap<FunctionId, Vec<TypeId>>,
    function_type_parameters: HashMap<FunctionId, Vec<TypeId>>,
    generic_parameter_constraints: HashMap<TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
    specialized_types: HashMap<(FunctionInstance, TypeId), TypeId>,
    record_field_types: HashMap<RecordFieldId, TypeId>,
    standard_field_types: HashMap<StdlibFieldId, TypeId>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<TypeId>>,
    array_element_types: HashMap<ArrayTypeId, TypeId>,
    state_poll_results: HashMap<ValueId, TypeId>,
    propagation_targets: HashMap<ExprId, TypeId>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    record_literals: HashMap<ExprId, ResolvedRecordId>,
    record_literal_fields: HashMap<ExprId, Vec<ResolvedRecordFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    assignment_calls: HashMap<AssignmentId, ResolvedCall>,
    value_conversions: HashMap<ExprId, ValueConversion>,
    visible_expression_count: Option<usize>,
}

impl SemanticModel {
    pub fn types(&self) -> &TypeStore {
        &self.types
    }

    /// The catalog provider selected by `state ProviderName`, if present.
    pub fn state_provider(&self) -> Option<StdlibStateProviderId> {
        self.state_provider
    }

    pub fn expression_type(&self, expression: ExprId) -> Option<TypeId> {
        self.expression_types.get(&expression).copied()
    }

    pub fn expression_types(&self) -> impl Iterator<Item = (ExprId, TypeId)> + '_ {
        self.expression_types
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, ty)| (*expression, *ty))
    }

    pub fn call(&self, expression: ExprId) -> Option<&ResolvedCall> {
        self.calls.get(&expression)
    }

    pub fn value(&self, expression: ExprId) -> Option<ResolvedValue> {
        self.values.get(&expression).copied()
    }

    pub fn values(&self) -> impl Iterator<Item = (ExprId, ResolvedValue)> + '_ {
        self.values
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, value)| (*expression, *value))
    }

    pub fn value_type(&self, value: ValueId) -> Option<TypeId> {
        self.value_types.get(&value).copied()
    }

    pub fn value_types(&self) -> impl Iterator<Item = (ValueId, TypeId)> + '_ {
        self.value_types.iter().map(|(value, ty)| (*value, *ty))
    }

    pub fn function_result(&self, function: FunctionId) -> Option<TypeId> {
        self.function_results.get(&function).copied()
    }

    pub fn function_parameter_types(&self, function: FunctionId) -> &[TypeId] {
        self.function_parameter_types
            .get(&function)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn function_type_parameters(&self, function: FunctionId) -> &[TypeId] {
        self.function_type_parameters
            .get(&function)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Constructs the canonical concrete identity for an inferred function
    /// template from its exact call signature. This is also used when a
    /// catalog call targets a hidden source body: the catalog owns the public
    /// signature, while the hidden declaration may infer a differently shaped
    /// but equivalent set of generalized roots.
    pub fn function_instance(
        &self,
        function: FunctionId,
        signature: Vec<TypeId>,
    ) -> FunctionInstance {
        let templates = self
            .function_parameter_types(function)
            .iter()
            .copied()
            .chain(self.function_result(function))
            .collect::<Vec<_>>();
        debug_assert_eq!(templates.len(), signature.len());
        let type_arguments = self
            .function_type_parameters(function)
            .iter()
            .map(|parameter| {
                templates
                    .iter()
                    .copied()
                    .zip(signature.iter().copied())
                    .find_map(|(template, concrete)| {
                        self.specialize_signature_node(template, concrete, *parameter)
                    })
                    .expect("generalized function roots occur in their signature")
            })
            .collect();
        FunctionInstance {
            function,
            type_arguments,
            signature,
        }
    }

    pub fn generic_parameter_constraints(
        &self,
        parameter: TypeId,
    ) -> &[crate::stdlib::StdlibCapabilityId] {
        self.generic_parameter_constraints
            .get(&parameter)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Substitutes a function template's type parameters for one concrete
    /// instance. Constructed instantiations are already interned by call-site
    /// inference, so specialization preserves the checked program's canonical
    /// type and layout identities.
    pub fn specialize_type(&self, instance: &FunctionInstance, ty: TypeId) -> TypeId {
        if let Some(specialized) = self.specialized_types.get(&(instance.clone(), ty)) {
            return *specialized;
        }
        if let Some(specialized) = self.direct_specialization(instance, ty) {
            return specialized;
        }
        let specialized_child = match self.types.kind(ty) {
            TypeKind::Array { element, .. } => Some((0, self.specialize_type(instance, *element))),
            TypeKind::Option { value, .. } => Some((1, self.specialize_type(instance, *value))),
            TypeKind::Result { value, .. } => Some((2, self.specialize_type(instance, *value))),
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::Record(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. } => None,
        };
        let Some((constructor, child)) = specialized_child else {
            return ty;
        };
        let original_array_length = match self.types.kind(ty) {
            TypeKind::Array { length, .. } => *length,
            _ => None,
        };
        let original_child = match self.types.kind(ty) {
            TypeKind::Array { element, .. } => *element,
            TypeKind::Option { value, .. } | TypeKind::Result { value, .. } => *value,
            _ => unreachable!(),
        };
        if child == original_child {
            return ty;
        }
        self.types
            .iter()
            .find_map(|(candidate, kind)| match (constructor, kind) {
                (
                    0,
                    TypeKind::Array {
                        element, length, ..
                    },
                ) if *element == child && *length == original_array_length => Some(candidate),
                (1, TypeKind::Option { value, .. }) if *value == child => Some(candidate),
                (2, TypeKind::Result { value, .. }) if *value == child => Some(candidate),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "backend specialization did not materialize a concrete instance of {:?}",
                    self.types.kind(ty)
                )
            })
    }

    fn direct_specialization(&self, instance: &FunctionInstance, ty: TypeId) -> Option<TypeId> {
        let parameters = self.function_type_parameters(instance.function);
        if let Some(index) = parameters.iter().position(|parameter| *parameter == ty) {
            return instance.type_arguments.get(index).copied();
        }
        let templates = self
            .function_parameter_types(instance.function)
            .iter()
            .copied()
            .chain(self.function_result(instance.function));
        for (template, concrete) in templates.zip(&instance.signature) {
            if let Some(specialized) = self.specialize_signature_node(template, *concrete, ty) {
                return Some(specialized);
            }
        }
        None
    }

    pub(crate) fn materialize_specialized_type(
        &mut self,
        instance: &FunctionInstance,
        ty: TypeId,
        ids: &mut crate::ast::ConstructedTypeIdAllocator,
        arrays: &mut Vec<ResolvedArrayType>,
        options: &mut Vec<ResolvedOptionType>,
        results: &mut Vec<ResolvedResultType>,
    ) -> TypeId {
        if let Some(specialized) = self.specialized_types.get(&(instance.clone(), ty)) {
            return *specialized;
        }
        if let Some(specialized) = self.direct_specialization(instance, ty) {
            self.specialized_types
                .insert((instance.clone(), ty), specialized);
            return specialized;
        }
        let kind = self.types.kind(ty).clone();
        let specialized = match kind {
            TypeKind::Array {
                element, length, ..
            } => {
                let element = self
                    .materialize_specialized_type(instance, element, ids, arrays, options, results);
                if element
                    == match self.types.kind(ty) {
                        TypeKind::Array { element, .. } => *element,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.array();
                    arrays.push(ResolvedArrayType {
                        id: layout,
                        element: self.resolved_type_ref(element),
                        length,
                    });
                    self.array_element_types.insert(layout, element);
                    self.types.intern(TypeKind::Array {
                        layout,
                        element,
                        length,
                    })
                }
            }
            TypeKind::Option { value, .. } => {
                let value = self
                    .materialize_specialized_type(instance, value, ids, arrays, options, results);
                if value
                    == match self.types.kind(ty) {
                        TypeKind::Option { value, .. } => *value,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.option();
                    options.push(ResolvedOptionType {
                        id: layout,
                        value: self.resolved_type_ref(value),
                    });
                    self.types.intern(TypeKind::Option { layout, value })
                }
            }
            TypeKind::Result { value, .. } => {
                let value = self
                    .materialize_specialized_type(instance, value, ids, arrays, options, results);
                if value
                    == match self.types.kind(ty) {
                        TypeKind::Result { value, .. } => *value,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.result();
                    results.push(ResolvedResultType {
                        id: layout,
                        value: self.resolved_type_ref(value),
                    });
                    self.types.intern(TypeKind::Result { layout, value })
                }
            }
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::Record(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. } => ty,
        };
        self.specialized_types
            .insert((instance.clone(), ty), specialized);
        specialized
    }

    fn resolved_type_ref(&self, ty: TypeId) -> crate::types::ResolvedTypeRef {
        match self.types.kind(ty) {
            TypeKind::Builtin(core) => crate::types::ResolvedTypeRef::Core(*core),
            TypeKind::Standard(standard) => crate::types::ResolvedTypeRef::Standard(*standard),
            TypeKind::StateSnapshot => crate::types::ResolvedTypeRef::StateSnapshot,
            TypeKind::SettingsView => crate::types::ResolvedTypeRef::SettingsView,
            TypeKind::Record(record) => crate::types::ResolvedTypeRef::Record(*record),
            TypeKind::Enum(enumeration) => crate::types::ResolvedTypeRef::Enum(*enumeration),
            TypeKind::GenericParameter { .. } => {
                crate::types::ResolvedTypeRef::GenericParameter(ty)
            }
            TypeKind::Array { layout, .. } => crate::types::ResolvedTypeRef::Array(*layout),
            TypeKind::Option { layout, .. } => crate::types::ResolvedTypeRef::Option(*layout),
            TypeKind::Result { layout, .. } => crate::types::ResolvedTypeRef::Result(*layout),
        }
    }

    fn specialize_signature_node(
        &self,
        template: TypeId,
        concrete: TypeId,
        searched: TypeId,
    ) -> Option<TypeId> {
        if template == searched {
            return Some(concrete);
        }
        match (self.types.kind(template), self.types.kind(concrete)) {
            (
                TypeKind::Array {
                    element: template, ..
                },
                TypeKind::Array {
                    element: concrete, ..
                },
            )
            | (
                TypeKind::Option {
                    value: template, ..
                },
                TypeKind::Option {
                    value: concrete, ..
                },
            )
            | (
                TypeKind::Result {
                    value: template, ..
                },
                TypeKind::Result {
                    value: concrete, ..
                },
            ) => self.specialize_signature_node(*template, *concrete, searched),
            _ => None,
        }
    }

    pub fn specialize_function_instance(
        &self,
        owner: &FunctionInstance,
        called: &FunctionInstance,
    ) -> FunctionInstance {
        FunctionInstance {
            function: called.function,
            type_arguments: called
                .type_arguments
                .iter()
                .map(|ty| self.specialize_type(owner, *ty))
                .collect(),
            signature: called
                .signature
                .iter()
                .map(|ty| self.specialize_type(owner, *ty))
                .collect(),
        }
    }

    pub(crate) fn set_function_type_parameters(
        &mut self,
        parameters: HashMap<FunctionId, Vec<TypeId>>,
        constraints: HashMap<TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
    ) {
        self.function_type_parameters = parameters;
        self.generic_parameter_constraints = constraints;
    }

    pub(crate) fn set_function_parameter_types(
        &mut self,
        parameters: HashMap<FunctionId, Vec<TypeId>>,
    ) {
        self.function_parameter_types = parameters;
    }

    pub fn record_field_type(&self, field: RecordFieldId) -> Option<TypeId> {
        self.record_field_types.get(&field).copied()
    }

    pub fn record_field_types(&self) -> impl Iterator<Item = (RecordFieldId, TypeId)> + '_ {
        self.record_field_types
            .iter()
            .map(|(field, ty)| (*field, *ty))
    }

    pub fn standard_field_type(&self, field: StdlibFieldId) -> Option<TypeId> {
        self.standard_field_types.get(&field).copied()
    }

    pub fn enum_variant_payload(&self, variant: EnumVariantId) -> Option<TypeId> {
        self.enum_variant_payloads.get(&variant).copied().flatten()
    }

    pub fn enum_variant_payloads(
        &self,
    ) -> impl Iterator<Item = (EnumVariantId, Option<TypeId>)> + '_ {
        self.enum_variant_payloads
            .iter()
            .map(|(variant, payload)| (*variant, *payload))
    }

    pub fn array_element_type(&self, array: ArrayTypeId) -> Option<TypeId> {
        self.array_element_types.get(&array).copied()
    }

    pub fn array_element_types(&self) -> impl Iterator<Item = (ArrayTypeId, TypeId)> + '_ {
        self.array_element_types
            .iter()
            .map(|(array, element)| (*array, *element))
    }

    pub fn state_poll_result(&self, field: ValueId) -> Option<TypeId> {
        self.state_poll_results.get(&field).copied()
    }

    /// The result type produced by the nearest failure boundary for `value?`.
    pub fn propagation_target(&self, expression: ExprId) -> Option<TypeId> {
        self.propagation_targets.get(&expression).copied()
    }

    pub fn path_members(&self, expression: ExprId) -> Option<&[ResolvedMember]> {
        self.path_members.get(&expression).map(Vec::as_slice)
    }

    pub fn record_literal_fields(&self, expression: ExprId) -> Option<&[ResolvedRecordFieldId]> {
        self.record_literal_fields
            .get(&expression)
            .map(Vec::as_slice)
    }

    pub fn record_literal(&self, expression: ExprId) -> Option<ResolvedRecordId> {
        self.record_literals.get(&expression).copied()
    }

    pub fn enum_variant(&self, expression: ExprId) -> Option<ResolvedEnumVariantId> {
        self.enum_variants.get(&expression).copied()
    }

    pub fn pattern_variant(&self, pattern: PatternId) -> Option<ResolvedEnumVariantId> {
        self.pattern_variants.get(&pattern).copied()
    }

    pub fn wrapper_pattern(&self, pattern: PatternId) -> Option<ResolvedWrapperPattern> {
        self.wrapper_patterns.get(&pattern).copied()
    }

    pub fn setting_choice_default(&self, setting: ValueId) -> Option<EnumVariantId> {
        self.setting_choice_defaults.get(&setting).copied()
    }

    pub fn setting_choice_option(&self, option: SettingChoiceOptionId) -> Option<EnumVariantId> {
        self.setting_choice_options.get(&option).copied()
    }

    pub fn assignment_target(&self, assignment: AssignmentId) -> Option<ValueId> {
        self.assignments.get(&assignment).copied()
    }

    pub fn assignment_call(&self, assignment: AssignmentId) -> Option<&ResolvedCall> {
        self.assignment_calls.get(&assignment)
    }

    pub fn assignment_targets(&self) -> impl Iterator<Item = (AssignmentId, ValueId)> + '_ {
        self.assignments
            .iter()
            .map(|(assignment, target)| (*assignment, *target))
    }

    pub fn calls(&self) -> impl Iterator<Item = (ExprId, &ResolvedCall)> {
        self.calls
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, call)| (*expression, call))
    }

    pub(crate) fn set_visible_expression_count(&mut self, count: usize) {
        self.visible_expression_count = Some(count);
    }

    pub fn value_conversion(&self, expression: ExprId) -> Option<ValueConversion> {
        self.value_conversions.get(&expression).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingResolvedCall {
    UserFunction {
        function: FunctionId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
    },
    UserMethod {
        function: FunctionId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
        receiver: ResolvedReceiver,
        receiver_type: Type,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<Type>,
    },
    ResultError {
        result: crate::ast::ResultTypeId,
    },
    OptionSome {
        option: crate::ast::OptionTypeId,
    },
    ResultSuccess {
        result: crate::ast::ResultTypeId,
    },
}

#[derive(Debug, Clone, Copy)]
struct PendingValueConversion {
    kind: ValueConversionKind,
    source: Type,
    target: Type,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticBuilder {
    state_provider: Option<StdlibStateProviderId>,
    expression_types: HashMap<ExprId, Type>,
    calls: HashMap<ExprId, PendingResolvedCall>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, Type>,
    function_results: HashMap<FunctionId, Type>,
    record_field_types: HashMap<RecordFieldId, Type>,
    standard_field_types: HashMap<StdlibFieldId, Type>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<Type>>,
    array_element_types: HashMap<ArrayTypeId, Type>,
    state_poll_results: HashMap<ValueId, Type>,
    propagation_targets: HashMap<ExprId, Type>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    record_literals: HashMap<ExprId, ResolvedRecordId>,
    record_literal_fields: HashMap<ExprId, Vec<ResolvedRecordFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    assignment_calls: HashMap<AssignmentId, PendingResolvedCall>,
    value_conversions: HashMap<ExprId, PendingValueConversion>,
}

impl SemanticBuilder {
    pub(crate) fn resolve_recursive_call_type_arguments(
        &mut self,
        functions: &HashMap<FunctionId, Vec<Type>>,
    ) {
        for call in self.calls.values_mut() {
            match call {
                PendingResolvedCall::UserFunction {
                    function,
                    type_arguments,
                    ..
                }
                | PendingResolvedCall::UserMethod {
                    function,
                    type_arguments,
                    ..
                } if functions.contains_key(function) && type_arguments.is_empty() => {
                    *type_arguments = functions[function].clone();
                }
                _ => {}
            }
        }
    }
    pub(crate) fn with_state_provider(state_provider: Option<StdlibStateProviderId>) -> Self {
        Self {
            state_provider,
            ..Self::default()
        }
    }

    pub(crate) fn resolve_expression_type(&mut self, expression: ExprId, ty: Type) {
        let previous = self.expression_types.insert(expression, ty);
        debug_assert!(previous.is_none(), "expression IDs must be unique");
    }

    pub(crate) fn resolve_call(&mut self, expression: ExprId, call: PendingResolvedCall) {
        let previous = self.calls.insert(expression, call);
        debug_assert!(previous.is_none(), "call expression IDs must be unique");
    }

    pub(crate) fn resolve_assignment_call(
        &mut self,
        assignment: AssignmentId,
        call: PendingResolvedCall,
    ) {
        let previous = self.assignment_calls.insert(assignment, call);
        debug_assert!(
            previous.is_none(),
            "assignment operator call must be unique"
        );
    }

    pub(crate) fn resolve_value(&mut self, expression: ExprId, value: ResolvedValue) {
        let previous = self.values.insert(expression, value);
        debug_assert!(previous.is_none(), "path expression IDs must be unique");
    }

    pub(crate) fn resolve_value_type(&mut self, value: ValueId, ty: Type) {
        let previous = self.value_types.insert(value, ty);
        debug_assert!(previous.is_none(), "value IDs must be unique");
    }

    pub(crate) fn resolve_function_result(&mut self, function: FunctionId, ty: Type) {
        let previous = self.function_results.insert(function, ty);
        debug_assert!(previous.is_none(), "function IDs must be unique");
    }

    pub(crate) fn resolve_record_field_type(&mut self, field: RecordFieldId, ty: Type) {
        let previous = self.record_field_types.insert(field, ty);
        debug_assert!(previous.is_none(), "record field IDs must be unique");
    }

    pub(crate) fn resolve_standard_field_type(&mut self, field: StdlibFieldId, ty: Type) {
        let previous = self.standard_field_types.insert(field, ty);
        debug_assert!(previous.is_none(), "standard field IDs must be unique");
    }

    pub(crate) fn resolve_enum_variant_payload(
        &mut self,
        variant: EnumVariantId,
        payload: Option<Type>,
    ) {
        let previous = self.enum_variant_payloads.insert(variant, payload);
        debug_assert!(previous.is_none(), "enum variant IDs must be unique");
    }

    pub(crate) fn resolve_array_element_type(&mut self, array: ArrayTypeId, element: Type) {
        let previous = self.array_element_types.insert(array, element);
        debug_assert!(previous.is_none(), "array type IDs must be unique");
    }

    pub(crate) fn resolve_state_poll_result(&mut self, field: ValueId, result: Type) {
        let previous = self.state_poll_results.insert(field, result);
        debug_assert!(previous.is_none(), "state poll result must be unique");
    }

    pub(crate) fn resolve_propagation_target(&mut self, expression: ExprId, result: Type) {
        let previous = self.propagation_targets.insert(expression, result);
        debug_assert!(
            previous.is_none(),
            "propagation expression IDs must be unique"
        );
    }

    pub(crate) fn resolve_path_members(
        &mut self,
        expression: ExprId,
        members: Vec<ResolvedMember>,
    ) {
        let previous = self.path_members.insert(expression, members);
        debug_assert!(previous.is_none(), "path expression IDs must be unique");
    }

    pub(crate) fn resolve_record_literal_fields(
        &mut self,
        expression: ExprId,
        fields: Vec<ResolvedRecordFieldId>,
    ) {
        let previous = self.record_literal_fields.insert(expression, fields);
        debug_assert!(previous.is_none(), "record expression IDs must be unique");
    }

    pub(crate) fn resolve_record_literal(&mut self, expression: ExprId, record: ResolvedRecordId) {
        let previous = self.record_literals.insert(expression, record);
        debug_assert!(previous.is_none(), "record expression IDs must be unique");
    }

    pub(crate) fn resolve_enum_variant(
        &mut self,
        expression: ExprId,
        variant: ResolvedEnumVariantId,
    ) {
        let previous = self.enum_variants.insert(expression, variant);
        debug_assert!(previous.is_none(), "enum expression IDs must be unique");
    }

    pub(crate) fn resolve_pattern_variant(
        &mut self,
        pattern: PatternId,
        variant: ResolvedEnumVariantId,
    ) {
        let previous = self.pattern_variants.insert(pattern, variant);
        debug_assert!(previous.is_none(), "pattern IDs must be unique");
    }

    pub(crate) fn resolve_wrapper_pattern(
        &mut self,
        pattern: PatternId,
        wrapper: ResolvedWrapperPattern,
    ) {
        let previous = self.wrapper_patterns.insert(pattern, wrapper);
        debug_assert!(previous.is_none(), "pattern IDs must be unique");
    }

    pub(crate) fn resolve_setting_choice_default(
        &mut self,
        setting: ValueId,
        variant: EnumVariantId,
    ) {
        let previous = self.setting_choice_defaults.insert(setting, variant);
        debug_assert!(previous.is_none(), "setting IDs must be unique");
    }

    pub(crate) fn resolve_setting_choice_option(
        &mut self,
        option: SettingChoiceOptionId,
        variant: EnumVariantId,
    ) {
        let previous = self.setting_choice_options.insert(option, variant);
        debug_assert!(previous.is_none(), "choice option IDs must be unique");
    }

    pub(crate) fn resolve_assignment(&mut self, assignment: AssignmentId, target: ValueId) {
        let previous = self.assignments.insert(assignment, target);
        debug_assert!(previous.is_none(), "assignment IDs must be unique");
    }

    pub(crate) fn resolve_value_conversion(
        &mut self,
        expression: ExprId,
        kind: ValueConversionKind,
        source: Type,
        target: Type,
    ) {
        let previous = self.value_conversions.insert(
            expression,
            PendingValueConversion {
                kind,
                source,
                target,
            },
        );
        debug_assert!(previous.is_none(), "expression conversion must be unique");
    }

    pub(crate) fn standard_library_item(&self, expression: ExprId) -> Option<StdlibItemId> {
        self.calls.get(&expression).and_then(|call| match call {
            PendingResolvedCall::StandardLibrary { item, .. } => Some(*item),
            PendingResolvedCall::UserFunction { .. }
            | PendingResolvedCall::UserMethod { .. }
            | PendingResolvedCall::ResultError { .. }
            | PendingResolvedCall::OptionSome { .. }
            | PendingResolvedCall::ResultSuccess { .. } => None,
        })
    }

    pub(crate) fn finish(
        self,
        mut types: TypeStore,
        arrays: &[ResolvedArrayType],
        options: &[ResolvedOptionType],
        results: &[ResolvedResultType],
        mut resolve: impl FnMut(Type) -> Type,
    ) -> SemanticModel {
        let Self {
            state_provider,
            expression_types,
            calls,
            values,
            value_types,
            function_results,
            record_field_types,
            standard_field_types,
            enum_variant_payloads,
            array_element_types,
            state_poll_results,
            propagation_targets,
            path_members,
            record_literals,
            record_literal_fields,
            enum_variants,
            pattern_variants,
            wrapper_patterns,
            setting_choice_defaults,
            setting_choice_options,
            assignments,
            assignment_calls,
            value_conversions,
        } = self;
        // WebAssembly GC layouts are nominal. Keep every allocated constructed
        // layout queryable even when inference created it only as a boundary
        // and no source declaration ultimately names it.
        for array in arrays {
            types.intern_inferred(Type::Array(array.id), arrays, options, results);
        }
        for option in options {
            types.intern_inferred(Type::Option(option.id), arrays, options, results);
        }
        for result in results {
            types.intern_inferred(Type::Result(result.id), arrays, options, results);
        }
        let (calls, assignment_calls) = {
            let mut finish_call = |call| match call {
                PendingResolvedCall::UserFunction {
                    function,
                    type_arguments,
                    signature,
                } => ResolvedCall::UserFunction {
                    function,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                },
                PendingResolvedCall::UserMethod {
                    function,
                    type_arguments,
                    signature,
                    receiver,
                    receiver_type,
                } => ResolvedCall::UserMethod {
                    function,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                    receiver,
                    receiver_type: types.intern_inferred(
                        resolve(receiver_type),
                        arrays,
                        options,
                        results,
                    ),
                },
                PendingResolvedCall::StandardLibrary {
                    item,
                    type_arguments,
                    signature,
                    receiver,
                    receiver_type,
                } => ResolvedCall::StandardLibrary {
                    item,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                        .collect(),
                    receiver,
                    receiver_type: receiver_type
                        .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results)),
                },
                PendingResolvedCall::ResultError { result } => {
                    let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                    ResolvedCall::ResultError { result }
                }
                PendingResolvedCall::OptionSome { option } => {
                    let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                    ResolvedCall::OptionSome { option }
                }
                PendingResolvedCall::ResultSuccess { result } => {
                    let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                    ResolvedCall::ResultSuccess { result }
                }
            };
            let calls = calls
                .into_iter()
                .map(|(expression, call)| (expression, finish_call(call)))
                .collect();
            let assignment_calls = assignment_calls
                .into_iter()
                .map(|(assignment, call)| (assignment, finish_call(call)))
                .collect();
            (calls, assignment_calls)
        };
        let expression_types = expression_types
            .into_iter()
            .map(|(expression, ty)| {
                (
                    expression,
                    types.intern_inferred(resolve(ty), arrays, options, results),
                )
            })
            .collect();
        let value_types = value_types
            .into_iter()
            .map(|(value, ty)| {
                (
                    value,
                    types.intern_inferred(resolve(ty), arrays, options, results),
                )
            })
            .collect();
        let function_results = function_results
            .into_iter()
            .map(|(function, ty)| {
                (
                    function,
                    types.intern_inferred(resolve(ty), arrays, options, results),
                )
            })
            .collect();
        let record_field_types = record_field_types
            .into_iter()
            .map(|(field, ty)| {
                (
                    field,
                    types.intern_inferred(resolve(ty), arrays, options, results),
                )
            })
            .collect();
        let standard_field_types = standard_field_types
            .into_iter()
            .map(|(field, ty)| {
                (
                    field,
                    types.intern_inferred(resolve(ty), arrays, options, results),
                )
            })
            .collect();
        let enum_variant_payloads = enum_variant_payloads
            .into_iter()
            .map(|(variant, payload)| {
                (
                    variant,
                    payload.map(|ty| types.intern_inferred(resolve(ty), arrays, options, results)),
                )
            })
            .collect();
        let array_element_types = array_element_types
            .into_iter()
            .map(|(array, element)| {
                (
                    array,
                    types.intern_inferred(resolve(element), arrays, options, results),
                )
            })
            .collect();
        let state_poll_results = state_poll_results
            .into_iter()
            .map(|(field, result)| {
                (
                    field,
                    types.intern_inferred(resolve(result), arrays, options, results),
                )
            })
            .collect();
        let propagation_targets = propagation_targets
            .into_iter()
            .map(|(expression, result)| {
                (
                    expression,
                    types.intern_inferred(resolve(result), arrays, options, results),
                )
            })
            .collect();
        let value_conversions = value_conversions
            .into_iter()
            .map(|(expression, conversion)| {
                (
                    expression,
                    ValueConversion {
                        kind: conversion.kind,
                        source: types.intern_inferred(
                            resolve(conversion.source),
                            arrays,
                            options,
                            results,
                        ),
                        target: types.intern_inferred(
                            resolve(conversion.target),
                            arrays,
                            options,
                            results,
                        ),
                    },
                )
            })
            .collect();
        let wrapper_patterns = wrapper_patterns
            .into_iter()
            .map(|(pattern, wrapper)| {
                let wrapper = match wrapper {
                    ResolvedWrapperPattern::OptionNone(option) => {
                        let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                        ResolvedWrapperPattern::OptionNone(option)
                    }
                    ResolvedWrapperPattern::OptionSome(option) => {
                        let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                        ResolvedWrapperPattern::OptionSome(option)
                    }
                    ResolvedWrapperPattern::ResultSuccess(result) => {
                        let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                        ResolvedWrapperPattern::ResultSuccess(result)
                    }
                    ResolvedWrapperPattern::ResultError(result) => {
                        let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                        ResolvedWrapperPattern::ResultError(result)
                    }
                };
                (pattern, wrapper)
            })
            .collect();
        SemanticModel {
            types,
            state_provider,
            expression_types,
            calls,
            values,
            value_types,
            function_results,
            function_parameter_types: HashMap::new(),
            function_type_parameters: HashMap::new(),
            generic_parameter_constraints: HashMap::new(),
            specialized_types: HashMap::new(),
            record_field_types,
            standard_field_types,
            enum_variant_payloads,
            array_element_types,
            state_poll_results,
            propagation_targets,
            path_members,
            record_literals,
            record_literal_fields,
            enum_variants,
            pattern_variants,
            wrapper_patterns,
            setting_choice_defaults,
            setting_choice_options,
            assignments,
            assignment_calls,
            value_conversions,
            visible_expression_count: None,
        }
    }
}
