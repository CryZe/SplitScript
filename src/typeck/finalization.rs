//! Inference recovery, normalization, and semantic-product publication.

use crate::{
    ast::{ArrayTypeDecl, OptionTypeDecl, Program, ResultTypeDecl, Span, StateSource},
    inference::Type,
};

use super::{CheckOutput, Checker, RecoveringCheckOutput};

pub(super) fn finish(mut checker: Checker, program: &Program) -> RecoveringCheckOutput {
    checker.resolve_deferred_member_paths();
    checker.diagnose_ambiguous_process_reads();
    if checker.errors.is_empty() {
        checker.default_inference_variables();
    }
    if !checker.errors.is_empty() {
        checker.inference.recover_unbound();
    }
    for field in &program.state.as_ref().unwrap().fields {
        if matches!(field.source, StateSource::Pointer(_)) {
            let field_type = checker.declarations.state_fields[&field.name].1;
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
        .map(|array| ArrayTypeDecl {
            id: array.id,
            element: array.element.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let option_types = checker
        .inference
        .options()
        .iter()
        .map(|option| OptionTypeDecl {
            id: option.id,
            value: option.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let result_types = checker
        .inference
        .results()
        .iter()
        .map(|result| ResultTypeDecl {
            id: result.id,
            value: result.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    for array in &array_types {
        let element = checker.syntax_type(array.element);
        checker
            .semantics
            .resolve_array_element_type(array.id, element);
    }
    let semantics = std::mem::take(&mut checker.semantics);
    let semantic_types = checker.inference.type_store().clone();
    let enum_types = checker.declarations.enums.clone();
    let diagnostics = std::mem::take(&mut checker.errors);
    RecoveringCheckOutput {
        output: CheckOutput {
            semantics: semantics.finish(
                semantic_types,
                &array_types,
                &option_types,
                &result_types,
                |ty| checker.resolved_type(ty),
            ),
            enum_types,
            array_types,
            option_types,
            result_types,
        },
        diagnostics,
    }
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
        for (ty, span) in reads {
            if self.inference.is_unbound_without_default(ty) {
                self.error(
                    "cannot infer the memory type read by `process.read`; add a result annotation such as `let value: i32! = process.read(address)`, or use `process.read.i32(address)`",
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
