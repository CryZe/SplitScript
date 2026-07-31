//! Semantic facts produced by type checking and consumed by later stages.

use std::collections::HashMap;

use crate::{
    ast::{
        ArrayTypeDecl, ArrayTypeId, AssignmentId, EnumVariantId, ExprId, FunctionId,
        OptionTypeDecl, OptionTypeId, PatternId, RecordFieldId, RecordId, ResultTypeDecl,
        ResultTypeId, SettingChoiceOptionId, ValueId,
    },
    inference::Type,
    stdlib::{StdlibFieldId, StdlibItemId, StdlibStateProviderId, StdlibVariantId},
    types::{TypeId, TypeKind, TypeStore},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCall {
    UserFunction {
        function: FunctionId,
    },
    UserMethod {
        function: FunctionId,
        receiver: ResolvedValue,
        receiver_type: TypeId,
        receiver_members: Vec<ResolvedMember>,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<TypeId>,
        receiver: Option<ResolvedValue>,
        receiver_type: Option<TypeId>,
        receiver_members: Vec<ResolvedMember>,
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
    CurrentState(ValueId),
    OldState(ValueId),
    Setting(ValueId),
    OldSetting(ValueId),
}

/// A field selected after the root of a resolved value path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedMember {
    RecordField(RecordFieldId),
    StandardField(StdlibFieldId),
}

/// Stable identity of an enum variant selected by checked source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedEnumVariantId {
    Source(EnumVariantId),
    Standard(StdlibVariantId),
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
    expression_types: HashMap<ExprId, TypeId>,
    calls: HashMap<ExprId, ResolvedCall>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, TypeId>,
    function_results: HashMap<FunctionId, TypeId>,
    record_field_types: HashMap<RecordFieldId, TypeId>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<TypeId>>,
    array_element_types: HashMap<ArrayTypeId, TypeId>,
    state_poll_results: HashMap<ValueId, TypeId>,
    propagation_targets: HashMap<ExprId, TypeId>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    record_literals: HashMap<ExprId, RecordId>,
    record_literal_fields: HashMap<ExprId, Vec<RecordFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    value_conversions: HashMap<ExprId, ValueConversion>,
}

impl SemanticModel {
    pub fn types(&self) -> &TypeStore {
        &self.types
    }

    pub fn expression_type(&self, expression: ExprId) -> Option<TypeId> {
        self.expression_types.get(&expression).copied()
    }

    pub fn expression_types(&self) -> impl Iterator<Item = (ExprId, TypeId)> + '_ {
        self.expression_types
            .iter()
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

    pub fn record_field_type(&self, field: RecordFieldId) -> Option<TypeId> {
        self.record_field_types.get(&field).copied()
    }

    pub fn record_field_types(&self) -> impl Iterator<Item = (RecordFieldId, TypeId)> + '_ {
        self.record_field_types
            .iter()
            .map(|(field, ty)| (*field, *ty))
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

    pub fn record_literal_fields(&self, expression: ExprId) -> Option<&[RecordFieldId]> {
        self.record_literal_fields
            .get(&expression)
            .map(Vec::as_slice)
    }

    pub fn record_literal(&self, expression: ExprId) -> Option<RecordId> {
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

    pub fn assignment_targets(&self) -> impl Iterator<Item = (AssignmentId, ValueId)> + '_ {
        self.assignments
            .iter()
            .map(|(assignment, target)| (*assignment, *target))
    }

    pub fn calls(&self) -> impl Iterator<Item = (ExprId, &ResolvedCall)> {
        self.calls
            .iter()
            .map(|(expression, call)| (*expression, call))
    }

    pub fn value_conversion(&self, expression: ExprId) -> Option<ValueConversion> {
        self.value_conversions.get(&expression).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingResolvedCall {
    UserFunction {
        function: FunctionId,
    },
    UserMethod {
        function: FunctionId,
        receiver: ResolvedValue,
        receiver_type: Type,
        receiver_members: Vec<ResolvedMember>,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<Type>,
        receiver: Option<ResolvedValue>,
        receiver_type: Option<Type>,
        receiver_members: Vec<ResolvedMember>,
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
    expression_types: HashMap<ExprId, Type>,
    calls: HashMap<ExprId, PendingResolvedCall>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, Type>,
    function_results: HashMap<FunctionId, Type>,
    record_field_types: HashMap<RecordFieldId, Type>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<Type>>,
    array_element_types: HashMap<ArrayTypeId, Type>,
    state_poll_results: HashMap<ValueId, Type>,
    propagation_targets: HashMap<ExprId, Type>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    record_literals: HashMap<ExprId, RecordId>,
    record_literal_fields: HashMap<ExprId, Vec<RecordFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    value_conversions: HashMap<ExprId, PendingValueConversion>,
}

impl SemanticBuilder {
    pub(crate) fn resolve_expression_type(&mut self, expression: ExprId, ty: Type) {
        let previous = self.expression_types.insert(expression, ty);
        debug_assert!(previous.is_none(), "expression IDs must be unique");
    }

    pub(crate) fn resolve_call(&mut self, expression: ExprId, call: PendingResolvedCall) {
        let previous = self.calls.insert(expression, call);
        debug_assert!(previous.is_none(), "call expression IDs must be unique");
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
        fields: Vec<RecordFieldId>,
    ) {
        let previous = self.record_literal_fields.insert(expression, fields);
        debug_assert!(previous.is_none(), "record expression IDs must be unique");
    }

    pub(crate) fn resolve_record_literal(&mut self, expression: ExprId, record: RecordId) {
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
        arrays: &[ArrayTypeDecl],
        options: &[OptionTypeDecl],
        results: &[ResultTypeDecl],
        mut resolve: impl FnMut(Type) -> Type,
    ) -> SemanticModel {
        let Self {
            expression_types,
            calls,
            values,
            value_types,
            function_results,
            record_field_types,
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
        let calls = calls
            .into_iter()
            .map(|(expression, call)| {
                let call = match call {
                    PendingResolvedCall::UserFunction { function } => {
                        ResolvedCall::UserFunction { function }
                    }
                    PendingResolvedCall::UserMethod {
                        function,
                        receiver,
                        receiver_type,
                        receiver_members,
                    } => ResolvedCall::UserMethod {
                        function,
                        receiver,
                        receiver_type: types.intern_inferred(
                            resolve(receiver_type),
                            arrays,
                            options,
                            results,
                        ),
                        receiver_members,
                    },
                    PendingResolvedCall::StandardLibrary {
                        item,
                        type_arguments,
                        receiver,
                        receiver_type,
                        receiver_members,
                    } => ResolvedCall::StandardLibrary {
                        item,
                        type_arguments: type_arguments
                            .into_iter()
                            .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results))
                            .collect(),
                        receiver,
                        receiver_type: receiver_type
                            .map(|ty| types.intern_inferred(resolve(ty), arrays, options, results)),
                        receiver_members,
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
                (expression, call)
            })
            .collect();
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
            expression_types,
            calls,
            values,
            value_types,
            function_results,
            record_field_types,
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
            value_conversions,
        }
    }
}
