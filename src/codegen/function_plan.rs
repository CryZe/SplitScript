use std::collections::HashMap;

use wasm_encoder::{FunctionSection, HeapType, RefType, TypeSection, ValType};

use crate::{
    ast::{ActionKind, EnumDecl, Program},
    equality::EqualityCapabilities,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{RuntimeRepresentation, StandardLibrary, StdlibTypeId},
    types::{ResolvedArrayType, ResolvedOptionType, ResolvedResultType, ResolvedSetType},
};

use super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE, Type, action_result_val_type,
    async_frame::IntrinsicFutureInstance,
    dependencies::BackendDependencies,
    reachability, runtime_helper_registry, semantic_type, set_element_type,
    set_functions::{SetFunctionPlan, SetFunctions},
};

/// The complete, deterministic assignment of generated function signatures
/// and indices. Body generation consumes this plan but cannot mutate the
/// shared type or function index spaces.
pub(super) struct FunctionPlan<'a> {
    pub section: FunctionSection,
    pub runtime_helpers: RuntimeHelperPlan,
    pub equality: EqualityFunctions,
    pub sets: SetFunctions,
    pub users: HashMap<FunctionInstance, UserFunctionPlan>,
    pub intrinsic_futures: HashMap<IntrinsicFutureInstance, u32>,
    pub displays: HashMap<StdlibTypeId, FunctionInstance>,
    pub reads: Vec<u32>,
    pub transforms: Vec<Option<u32>>,
    pub actions: HashMap<ActionKind, u32>,
    pub start: u32,
    pub update: u32,
    pub arrays: &'a [ResolvedArrayType],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UserFunctionPlan {
    pub call: u32,
    pub poll: Option<u32>,
}

pub(super) struct Inputs<'a> {
    pub standard_library: &'a StandardLibrary,
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub options: &'a [ResolvedOptionType],
    pub results: &'a [ResolvedResultType],
    pub sets: &'a [ResolvedSetType],
    pub equality: &'a EqualityCapabilities,
    pub dependencies: &'a BackendDependencies,
    pub reachability: &'a reachability::Reachability,
    pub gc: &'a GcLayout,
    pub wasm_ir: &'a crate::wasm_ir::Program,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
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
        sets,
        equality: equality_capabilities,
        dependencies,
        reachability,
        gc,
        wasm_ir,
        async_frames,
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

    let mut set_functions = SetFunctions::default();
    for set in sets
        .iter()
        .filter(|set| reachability.contains_set_type(set.id))
    {
        let set_type = gc.val_type(Type::Set(set.id));
        let element_type = gc.val_type(set_element_type(set.id, semantics));
        set_functions.insert(
            set.id,
            SetFunctionPlan {
                new: declare(vec![], vec![set_type]),
                length: declare(vec![set_type], vec![ValType::I32]),
                contains: declare(vec![set_type, element_type], vec![ValType::I32]),
                insert: declare(vec![set_type, element_type], vec![ValType::I32]),
                remove: declare(vec![set_type, element_type], vec![ValType::I32]),
                clear: declare(vec![set_type], vec![]),
            },
        );
    }

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
                    .function_completion(instance.function)
                    .expect("checked functions have result types"),
            ),
            semantics,
        );
        let params = function
            .params
            .iter()
            .filter_map(|parameter| {
                let ty = semantic_type(
                    semantics.specialize_type(
                        instance,
                        semantics
                            .value_type(parameter.id)
                            .expect("checked parameters have types"),
                    ),
                    semantics,
                );
                (ty != Type::None).then(|| gc.val_type(ty))
            })
            .collect::<Vec<_>>();
        let body = wasm_ir
            .body(crate::wasm_ir::BodyOwner::Function(instance.clone()))
            .expect("reachable functions have Wasm IR bodies");
        let plan = if matches!(body.abi, crate::wasm_ir::BodyAbi::AsyncFunction(_)) {
            let frame = ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(gc.function_frame_index(instance)),
            });
            UserFunctionPlan {
                call: declare(params, vec![frame]),
                poll: Some(declare(vec![frame], vec![ValType::I32])),
            }
        } else {
            UserFunctionPlan {
                call: declare(
                    params,
                    (result != Type::None)
                        .then(|| gc.val_type(result))
                        .into_iter()
                        .collect(),
                ),
                poll: None,
            }
        };
        users.insert(instance.clone(), plan);
    }

    let mut intrinsic_futures = HashMap::new();
    for (instance, _) in async_frames.intrinsics() {
        let frame = ValType::Ref(RefType {
            nullable: false,
            heap_type: HeapType::Concrete(gc.intrinsic_frame_index(instance)),
        });
        intrinsic_futures.insert(instance.clone(), declare(vec![frame], vec![ValType::I32]));
    }

    let mut reads = Vec::with_capacity(
        program
            .state
            .as_ref()
            .map_or(0, |state| state.all_fields().count()),
    );
    let mut transforms = Vec::with_capacity(reads.capacity());
    if let Some(state) = &program.state {
        for field in state.all_fields() {
            let poll_result = semantic_type(
                semantics
                    .state_poll_result(field.id)
                    .expect("checked state fields have poll-result types"),
                semantics,
            );
            reads.push(declare(vec![ValType::I64], vec![gc.val_type(poll_result)]));
            transforms.push(field.transform.as_ref().map(|_| {
                let ty = semantic_type(
                    semantics
                        .value_type(field.id)
                        .expect("checked state fields have types"),
                    semantics,
                );
                let value = gc.val_type(ty);
                declare(vec![value], vec![gc.val_type(poll_result)])
            }));
        }
    }

    let state_ref = ValType::Ref(RefType {
        nullable: false,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    let mut actions = HashMap::new();
    for action in &program.actions {
        let (params, results) = match action.kind {
            ActionKind::Setup => (vec![], vec![]),
            ActionKind::OnAttach => (vec![ValType::I64], vec![ValType::I32]),
            action => (
                vec![state_ref, state_ref],
                (!matches!(action, ActionKind::OnDetached | ActionKind::WhileAttached))
                    .then(|| action_result_val_type(action, gc))
                    .into_iter()
                    .collect(),
            ),
        };
        actions.insert(action.kind, declare(params, results));
    }

    let start = declare(vec![], vec![]);
    let update = declare(vec![], vec![]);

    FunctionPlan {
        section,
        runtime_helpers,
        equality,
        sets: set_functions,
        users,
        intrinsic_futures,
        displays: reachability
            .display_functions()
            .map(|(ty, function)| (ty, function.clone()))
            .collect(),
        reads,
        transforms,
        actions,
        start,
        update,
        arrays,
    }
}
