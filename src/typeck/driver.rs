//! Top-level type-checking pass orchestration.

use crate::{
    ast::Program,
    inference::{ArrayLayout, InferenceContext, OptionLayout, ResultLayout, Type},
    resolution::ProgramResolutions,
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
    resolutions: &ProgramResolutions,
    standard_library: StandardLibrary,
) -> RecoveringCheckOutput {
    let mut checker = initialize_checker(program, resolutions, standard_library);
    declaration_pass::collect(&mut checker, program);
    body_pass::check(&mut checker, program);
    finalization::finish(checker, program)
}

fn initialize_checker(
    program: &Program,
    resolutions: &ProgramResolutions,
    standard_library: StandardLibrary,
) -> Checker {
    let records = program.records.clone();
    let enums = program.enum_declarations().cloned().collect::<Vec<_>>();
    let semantic_types = TypeStore::with_source_types(&standard_library, &records, &enums);
    let array_types = program
        .array_types
        .iter()
        .map(|array| ArrayLayout {
            id: array.id,
            element: syntax_type(array.element, &semantic_types, resolutions),
            length: array.length,
        })
        .collect::<Vec<_>>();
    let option_types = program
        .option_types
        .iter()
        .map(|option| OptionLayout {
            id: option.id,
            value: syntax_type(option.value, &semantic_types, resolutions),
        })
        .collect::<Vec<_>>();
    let result_types = program
        .result_types
        .iter()
        .map(|result| ResultLayout {
            id: result.id,
            value: syntax_type(result.value, &semantic_types, resolutions),
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
    let provider_value = resolutions.state_provider().map(|provider| {
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

    let mut checker = Checker {
        standard_library,
        resolutions: resolutions.clone(),
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
        layout_value: program.state.as_ref().and_then(|state| state.layout_value),
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
        semantics: SemanticBuilder::with_state_provider(resolutions.state_provider()),
        standard_field_types: std::collections::HashMap::new(),
        active_function_component: std::collections::HashSet::new(),
    };
    let fields = checker.standard_library.fields().to_vec();
    let variables = std::collections::HashMap::new();
    for field in fields {
        let ty = checker.catalog_type(field.ty, &variables);
        checker.standard_field_types.insert(field.id, ty);
        checker.semantics.resolve_standard_field_type(field.id, ty);
    }
    checker
}
