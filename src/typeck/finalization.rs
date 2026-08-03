//! Inference recovery, normalization, and semantic-product publication.

use std::collections::HashMap;

use crate::{
    ast::{FunctionId, Program, Span, StateSource},
    inference::Type,
    types::{ResolvedArrayType, ResolvedAsyncType, ResolvedOptionType, ResolvedResultType},
};

use super::{CheckOutput, Checker, RecoveringCheckOutput};

pub(super) fn finish(mut checker: Checker, program: &Program) -> RecoveringCheckOutput {
    checker.resolve_deferred_member_paths();
    checker.diagnose_ambiguous_process_reads();
    let (function_type_parameters, generic_parameter_constraints) = if checker.errors.is_empty() {
        bind_function_generics(&mut checker, program)
    } else {
        (HashMap::new(), HashMap::new())
    };
    if checker.errors.is_empty() {
        checker.default_inference_variables();
    }
    if !checker.errors.is_empty() {
        checker.inference.recover_unbound();
    }
    for field in program.state.as_ref().unwrap().all_fields() {
        if matches!(field.source, StateSource::Pointer(_)) {
            let Some(field_type) = checker
                .declarations
                .state_fields_by_id
                .get(&field.id)
                .copied()
            else {
                continue;
            };
            let poll_result = Type::Result(checker.inference.result_type(field_type));
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    }
    checker.finalize_array_types();
    checker.inference.finalize_wrappers();
    checker.inference.intern_resolved_constructed_types();
    let array_types = checker
        .inference
        .arrays()
        .iter()
        .map(|array| ResolvedArrayType {
            id: array.id,
            element: array.element.to_ref(checker.inference.type_store()),
            length: array.length,
        })
        .collect::<Vec<_>>();
    let option_types = checker
        .inference
        .options()
        .iter()
        .map(|option| ResolvedOptionType {
            id: option.id,
            value: option.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let result_types = checker
        .inference
        .results()
        .iter()
        .map(|result| ResolvedResultType {
            id: result.id,
            value: result.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let async_types = checker
        .inference
        .asyncs()
        .iter()
        .map(|future| ResolvedAsyncType {
            id: future.id,
            value: future.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    for array in &array_types {
        let element = checker.resolved_type_ref(array.element);
        checker
            .semantics
            .resolve_array_element_type(array.id, element);
    }
    let semantics = std::mem::take(&mut checker.semantics);
    let semantic_types = checker.inference.type_store().clone();
    let enum_types = checker.declarations.enums.clone();
    let diagnostics = std::mem::take(&mut checker.errors);
    let mut semantics = semantics.finish(
        semantic_types,
        &array_types,
        &option_types,
        &result_types,
        &async_types,
        |ty| checker.resolved_type(ty),
    );
    semantics.set_function_parameter_types(
        program
            .functions
            .iter()
            .map(|function| {
                (
                    function.id,
                    function
                        .params
                        .iter()
                        .map(|parameter| {
                            semantics
                                .value_type(parameter.id)
                                .expect("checked parameters have semantic types")
                        })
                        .collect(),
                )
            })
            .collect(),
    );
    semantics.set_function_type_parameters(function_type_parameters, generic_parameter_constraints);
    RecoveringCheckOutput {
        output: CheckOutput {
            semantics,
            enum_types,
            array_types,
            option_types,
            result_types,
            async_types,
        },
        diagnostics,
    }
}

fn bind_function_generics(
    checker: &mut Checker,
    program: &Program,
) -> (
    HashMap<FunctionId, Vec<crate::types::TypeId>>,
    HashMap<crate::types::TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
) {
    let mut roots = HashMap::new();
    let mut parameters = HashMap::new();
    let mut constraints = HashMap::new();
    for function in &program.functions {
        let generalized = checker.declarations.function_signatures[&function.id]
            .generalized
            .clone();
        let mut function_parameters = Vec::with_capacity(generalized.len());
        for (index, variable) in generalized.into_iter().enumerate() {
            let parameter = if let Some(parameter) = roots.get(&variable) {
                *parameter
            } else {
                let parameter = checker
                    .inference
                    .intern_generic_parameter(function.id, index as u32);
                constraints.insert(
                    parameter,
                    checker
                        .inference
                        .variable_requirements(variable)
                        .as_slice()
                        .to_vec(),
                );
                roots.insert(variable, parameter);
                parameter
            };
            function_parameters.push(parameter);
        }
        if !function_parameters.is_empty() {
            parameters.insert(function.id, function_parameters);
        }
    }
    for (variable, parameter) in roots {
        checker
            .inference
            .bind_generic_parameter(variable, parameter);
    }
    (parameters, constraints)
}

impl Checker {
    pub(super) fn default_inference_variables(&mut self) {
        for error in self.inference.default_unbound() {
            let message = self.inference_error_message(error);
            self.error(message, Span::default());
        }
    }

    pub(super) fn diagnose_ambiguous_process_reads(&mut self) {
        let reads = self.inferred_process_reads.clone();
        let generalized = self
            .declarations
            .function_signatures
            .values()
            .flat_map(|signature| signature.generalized.iter().copied())
            .collect::<Vec<_>>();
        for (ty, span) in reads {
            let belongs_to_scheme = self
                .inference
                .unbound_variables_in([ty])
                .iter()
                .any(|variable| generalized.contains(variable));
            if !belongs_to_scheme && self.inference.is_unbound_without_default(ty) {
                self.error(
                    "cannot infer the memory type read by `process.read`; add a result annotation such as `let value: i32! = process.read(address)`, or use `process.read<i32>(address)`",
                    span,
                );
            }
        }
    }

    pub(super) fn resolved_type(&mut self, ty: Type) -> Type {
        self.inference.resolve(ty)
    }

    pub(super) fn finalize_array_types(&mut self) {
        self.inference.finalize_arrays();
    }
}
