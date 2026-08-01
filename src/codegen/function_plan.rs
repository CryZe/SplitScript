use std::collections::HashMap;

use wasm_encoder::{FunctionSection, HeapType, RefType, TypeSection, ValType};

use crate::{
    ast::{ActionKind, EnumDecl, Program},
    equality::EqualityCapabilities,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{RuntimeRepresentation, StandardLibrary},
    types::{ResolvedArrayType, ResolvedOptionType, ResolvedResultType},
};

use super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE, Type, action_result_val_type,
    dependencies::BackendDependencies, reachability, runtime_helper_registry, semantic_type,
};

/// The complete, deterministic assignment of generated function signatures
/// and indices. Body generation consumes this plan but cannot mutate the
/// shared type or function index spaces.
pub(super) struct FunctionPlan<'a> {
    pub section: FunctionSection,
    pub runtime_helpers: RuntimeHelperPlan,
    pub equality: EqualityFunctions,
    pub users: HashMap<FunctionInstance, u32>,
    pub reads: Vec<u32>,
    pub actions: HashMap<ActionKind, u32>,
    pub start: u32,
    pub update: u32,
    pub arrays: &'a [ResolvedArrayType],
}

pub(super) struct Inputs<'a> {
    pub standard_library: &'a StandardLibrary,
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub options: &'a [ResolvedOptionType],
    pub results: &'a [ResolvedResultType],
    pub equality: &'a EqualityCapabilities,
    pub dependencies: &'a BackendDependencies,
    pub reachability: &'a reachability::Reachability,
    pub gc: &'a GcLayout,
}

pub(super) fn encode<'a>(
    types: &mut TypeSection,
    mut next_type: u32,
    imported_functions: u32,
    inputs: Inputs<'a>,
) -> FunctionPlan<'a> {
    let Inputs {
        standard_library,
        program,
        semantics,
        enums,
        arrays,
        options,
        results,
        equality: equality_capabilities,
        dependencies,
        reachability,
        gc,
    } = inputs;
    let mut section = FunctionSection::new();
    let mut next_function = imported_functions;

    let mut declare = |params: Vec<ValType>, results: Vec<ValType>| {
        let type_index = next_type;
        next_type += 1;
        types.ty().function(params, results);
        section.function(type_index);
        let function_index = next_function;
        next_function += 1;
        function_index
    };

    let mut helper_functions = HashMap::new();
    let ordered_helpers = dependencies.helpers().collect::<Vec<_>>();
    for helper in ordered_helpers.iter().copied() {
        let descriptor = runtime_helper_registry::descriptor(helper);
        let (params, results) =
            runtime_helper_registry::resolve_signature(descriptor.signature, arrays, semantics, gc);
        helper_functions.insert(helper, declare(params, results));
    }

    let mut equality = EqualityFunctions {
        standard_library: standard_library.clone(),
        ..EqualityFunctions::default()
    };
    for record in standard_library.types().iter().filter(|record| {
        reachability.requires_standard_record_equality(record.id)
            && matches!(
                record.representation,
                RuntimeRepresentation::GcStruct { .. }
            )
    }) {
        let record_type = gc.val_type(Type::Standard(record.id));
        let function = declare(vec![record_type, record_type], vec![ValType::I32]);
        equality.standard_records.insert(record.id, function);
    }
    for record in &program.records {
        if reachability.requires_record_equality(record.id)
            && equality_capabilities.record(record.id).is_ok()
        {
            let record_type = gc.val_type(Type::Record(record.id));
            let function = declare(vec![record_type, record_type], vec![ValType::I32]);
            equality.records.insert(record.id, function);
        }
    }
    for enumeration in enums {
        if reachability.requires_enum_equality(enumeration.id)
            && equality_capabilities.enumeration(enumeration.id).is_ok()
        {
            let enum_type = gc.val_type(Type::Enum(enumeration.id));
            let function = declare(vec![enum_type, enum_type], vec![ValType::I32]);
            equality.enums.insert(enumeration.id, function);
        }
    }
    for option in options {
        if reachability.requires_option_equality(option.id) {
            let option_type = gc.val_type(Type::Option(option.id));
            let function = declare(vec![option_type, option_type], vec![ValType::I32]);
            equality.options.insert(option.id, function);
        }
    }
    for result in results {
        if reachability.requires_result_equality(result.id) {
            let result_type = gc.val_type(Type::Result(result.id));
            let function = declare(vec![result_type, result_type], vec![ValType::I32]);
            equality.results.insert(result.id, function);
        }
    }

    let runtime_helpers = RuntimeHelperPlan {
        ordered: ordered_helpers,
        functions: helper_functions,
    };

    let mut users = HashMap::new();
    for instance in reachability.functions() {
        let function = program
            .functions
            .iter()
            .find(|function| function.id == instance.function)
            .expect("reachable function instances have source declarations");
        let result = semantic_type(
            semantics.specialize_type(
                instance,
                semantics
                    .function_result(instance.function)
                    .expect("checked functions have result types"),
            ),
            semantics,
        );
        let index = declare(
            function
                .params
                .iter()
                .map(|parameter| {
                    gc.val_type(semantic_type(
                        semantics.specialize_type(
                            instance,
                            semantics
                                .value_type(parameter.id)
                                .expect("checked parameters have types"),
                        ),
                        semantics,
                    ))
                })
                .collect(),
            (result != Type::Void)
                .then(|| gc.val_type(result))
                .into_iter()
                .collect(),
        );
        users.insert(instance.clone(), index);
    }

    let mut reads =
        Vec::with_capacity(program.state.as_ref().map_or(0, |state| state.fields.len()));
    if let Some(state) = &program.state {
        for field in &state.fields {
            let poll_result = semantic_type(
                semantics
                    .state_poll_result(field.id)
                    .expect("checked state fields have poll-result types"),
                semantics,
            );
            reads.push(declare(vec![ValType::I64], vec![gc.val_type(poll_result)]));
        }
    }

    let state_ref = ValType::Ref(RefType {
        nullable: false,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    let mut actions = HashMap::new();
    for action in &program.actions {
        let (params, results) = if action.kind == ActionKind::OnAttach {
            (vec![ValType::I64], vec![ValType::I32])
        } else {
            (
                vec![state_ref, state_ref],
                (!matches!(
                    action.kind,
                    ActionKind::OnDetached | ActionKind::OnAttach | ActionKind::WhileAttached
                ))
                .then(|| action_result_val_type(action.kind, gc))
                .into_iter()
                .collect(),
            )
        };
        actions.insert(action.kind, declare(params, results));
    }

    let start = declare(vec![], vec![]);
    let update = declare(vec![], vec![]);

    FunctionPlan {
        section,
        runtime_helpers,
        equality,
        users,
        reads,
        actions,
        start,
        update,
        arrays,
    }
}
