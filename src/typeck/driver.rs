//! Top-level type-checking pass orchestration.

use crate::{
    ast::Program,
    inference::{ArrayLayout, InferenceContext, OptionLayout, ResultLayout, Type},
    semantic::SemanticBuilder,
    stdlib::StandardLibrary,
    types::TypeStore,
};

use super::{
    Checker, RecoveringCheckOutput, body_pass,
    context::{
        CallableContext, DebugContext, ExpressionMode, FailureContext, LoopContext, NonePolicy,
    },
    declaration_pass,
    declarations::DeclarationEnvironment,
    finalization, syntax_type,
};

pub(super) fn check_recovering(
    program: &Program,
    standard_library: StandardLibrary,
) -> RecoveringCheckOutput {
    let mut checker = initialize_checker(program, standard_library);
    declaration_pass::collect(&mut checker, program);
    body_pass::check(&mut checker, program);
    finalization::finish(checker, program)
}

fn initialize_checker(program: &Program, standard_library: StandardLibrary) -> Checker {
    let records = program.records.clone();
    let enums = program.enums.clone();
    let semantic_types = TypeStore::with_source_types(&standard_library, &records, &enums);
    let array_types = program
        .array_types
        .iter()
        .map(|array| ArrayLayout {
            id: array.id,
            element: syntax_type(array.element, &semantic_types),
        })
        .collect::<Vec<_>>();
    let option_types = program
        .option_types
        .iter()
        .map(|option| OptionLayout {
            id: option.id,
            value: syntax_type(option.value, &semantic_types),
        })
        .collect::<Vec<_>>();
    let result_types = program
        .result_types
        .iter()
        .map(|result| ResultLayout {
            id: result.id,
            value: syntax_type(result.value, &semantic_types),
        })
        .collect::<Vec<_>>();
    let void_type = Type::Known(semantic_types.id_for_core(crate::stdlib::CoreTypeId::Void));
    let inference = InferenceContext::new(
        standard_library.clone(),
        semantic_types,
        records.len() as u32 + enums.len() as u32,
        array_types,
        option_types,
        result_types,
    );
    let provider_value = program
        .state
        .as_ref()
        .and_then(|state| state.provider.as_ref())
        .and_then(|reference| reference.resolved)
        .map(|provider| {
            let declaration = standard_library.state_provider(provider);
            (
                provider,
                Type::Known(
                    inference
                        .type_store()
                        .id_for_standard(declaration.process_type),
                ),
            )
        });

    Checker {
        standard_library,
        errors: Vec::new(),
        declarations: DeclarationEnvironment::new(
            records,
            enums,
            program
                .functions
                .iter()
                .filter(|function| function.debug_only)
                .map(|function| function.id)
                .collect(),
        ),
        inference,
        provider_value,
        scopes: Vec::new(),
        return_ty: void_type,
        callable: CallableContext::TopLevel,
        expression_mode: ExpressionMode::Normal,
        debug_context: DebugContext::Normal,
        loops: LoopContext::default(),
        failure: FailureContext::None,
        inferred_process_reads: Vec::new(),
        deferred_member_paths: Vec::new(),
        none_policy: NonePolicy::OptionalOnly,
        semantics: SemanticBuilder::default(),
    }
}
