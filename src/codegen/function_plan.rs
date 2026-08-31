use std::collections::HashMap;

use wasm_encoder::{FunctionSection, HeapType, RefType, TypeSection, ValType};

use crate::{
    ast::{ActionKind, EnumDecl, ManagedClassId, ManagedFieldId, Program},
    equality::EqualityCapabilities,
    semantic::{ClosureInstance, FunctionInstance, FunctionValueInstance, SemanticModel},
    stdlib::{RuntimeRepresentation, StandardLibrary},
    structural::{StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, ResolvedOptionType, ResolvedResultType, ResolvedSetType},
};

use super::{
    ArrayFunctions, DisplayFunctions, EqualityFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE,
    Type, action_result_val_type,
    async_frame::LeafFutureInstance,
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
    pub array_functions: ArrayFunctions,
    pub sets: SetFunctions,
    pub users: HashMap<FunctionInstance, UserFunctionPlan>,
    pub closures: HashMap<ClosureInstance, u32>,
    pub function_values: HashMap<FunctionValueInstance, u32>,
    pub closure_polls: HashMap<ClosureInstance, u32>,
    pub leaf_futures: HashMap<LeafFutureInstance, u32>,
    pub displays: DisplayFunctions,
    pub managed_state_reads: HashMap<ManagedFieldId, u32>,
    pub managed_snapshots: HashMap<ManagedClassId, u32>,
    pub reads: Vec<u32>,
    pub transforms: Vec<Option<u32>>,
    pub actions: HashMap<ActionKind, u32>,
    pub start: u32,
    pub update: u32,
    pub arrays: &'a [ResolvedArrayType],
    /// Names assigned alongside final function indices. Encoding remains a
    /// separate profile-aware step so release modules cannot leak symbols.
    pub debug_names: Vec<(u32, String)>,
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
    pub structural: &'a StructuralTypes,
    pub dependencies: &'a BackendDependencies,
    pub reachability: &'a reachability::Reachability,
    pub gc: &'a GcLayout,
    pub wasm_ir: &'a crate::wasm_ir::Program,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub managed_state_reads: &'a super::managed_state_reads::ManagedStateReadCache,
    pub pointer_prefixes: &'a super::pointer_prefixes::PointerPrefixPlan,
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
        structural,
        dependencies,
        reachability,
        gc,
        wasm_ir,
        async_frames,
        managed_state_reads,
        pointer_prefixes,
    } = inputs;
    let mut section = FunctionSection::new();
    let mut next_function = imported_functions;
    let mut debug_names = Vec::new();

    let mut declare = |name: String, params: Vec<ValType>, results: Vec<ValType>| {
        let type_index = next_type;
        next_type += 1;
        types.ty().function(params, results);
        section.function(type_index);
        let function_index = next_function;
        next_function += 1;
        debug_names.push((function_index, name));
        function_index
    };

    let mut helper_functions = HashMap::new();
    let ordered_helpers = dependencies.helpers().collect::<Vec<_>>();
    for helper in ordered_helpers.iter().copied() {
        let descriptor = runtime_helper_registry::descriptor(helper);
        let (params, results) =
            runtime_helper_registry::resolve_signature(descriptor.signature, arrays, semantics, gc);
        helper_functions.insert(
            helper,
            declare(
                format!("__splitscript::runtime::{helper:?}"),
                params,
                results,
            ),
        );
    }

    let mut equality = EqualityFunctions {
        standard_library: standard_library.clone(),
        ..EqualityFunctions::default()
    };
    for structure in standard_library.all_types().iter().filter(|structure| {
        reachability.requires_standard_struct_equality(structure.id)
            && matches!(
                structure.representation,
                RuntimeRepresentation::GcStruct { .. }
            )
    }) {
        let struct_type = gc.val_type(Type::Standard(structure.id));
        let function = declare(
            format!("__splitscript::equals::{}", structure.name),
            vec![struct_type, struct_type],
            vec![ValType::I32],
        );
        equality.standard_structs.insert(structure.id, function);
    }
    for (_, structure) in structural.structs() {
        let StructuralTypeId::Struct(struct_id) = structure.id else {
            unreachable!()
        };
        if reachability.requires_struct_equality(struct_id)
            && equality_capabilities.structure(struct_id).is_ok()
        {
            let struct_type = gc.val_type(Type::Struct(struct_id));
            let function = declare(
                format!("__splitscript::equals::{}", structure.name),
                vec![struct_type, struct_type],
                vec![ValType::I32],
            );
            equality.structs.insert(struct_id, function);
        }
    }
    for (_, enumeration) in structural.enums() {
        let StructuralTypeId::Enum(enum_id) = enumeration.id else {
            unreachable!()
        };
        if reachability.requires_enum_equality(enum_id)
            && equality_capabilities.enumeration(enum_id).is_ok()
        {
            let enum_type = gc.val_type(Type::Enum(enum_id));
            let function = declare(
                format!("__splitscript::equals::{}", enumeration.name),
                vec![enum_type, enum_type],
                vec![ValType::I32],
            );
            equality.enums.insert(enum_id, function);
        }
    }
    for option in options {
        if reachability.requires_option_equality(option.id) {
            let option_type = gc.val_type(Type::Option(option.id));
            let function = declare(
                format!("__splitscript::equals::option#{}", option.id.index()),
                vec![option_type, option_type],
                vec![ValType::I32],
            );
            equality.options.insert(option.id, function);
        }
    }
    for result in results {
        if reachability.requires_result_equality(result.id) {
            let result_type = gc.val_type(Type::Result(result.id));
            let function = declare(
                format!("__splitscript::equals::result#{}", result.id.index()),
                vec![result_type, result_type],
                vec![ValType::I32],
            );
            equality.results.insert(result.id, function);
        }
    }

    let mut displays = DisplayFunctions {
        custom: reachability
            .display_functions()
            .map(|(ty, function)| (ty, function.clone()))
            .collect(),
        custom_debug: reachability
            .debug_functions()
            .map(|(ty, function)| (ty, function.clone()))
            .collect(),
        ..DisplayFunctions::default()
    };
    for ty in reachability.derived_debugs() {
        let source_type = super::semantic_type(ty, semantics);
        let name = structural
            .get(ty)
            .map_or_else(|| format!("type#{}", ty.index()), |ty| ty.name.clone());
        displays.derived.insert(
            ty,
            declare(
                format!("__splitscript::debug::{name}"),
                vec![gc.val_type(source_type)],
                vec![gc.val_type(Type::Standard(crate::stdlib::StdlibTypeId::String))],
            ),
        );
    }

    let runtime_helpers = RuntimeHelperPlan {
        ordered: ordered_helpers,
        functions: helper_functions,
    };

    let mut array_functions = ArrayFunctions::default();
    for array in arrays.iter().filter(|array| {
        reachability.requires_array_push(array.id)
            || reachability.requires_array_remove_at(array.id)
            || reachability.requires_array_clear(array.id)
    }) {
        debug_assert!(array.length.is_none());
        let array_type = gc.val_type(Type::Array(array.id));
        if reachability.requires_array_push(array.id) {
            let element_type = gc.val_type(
                super::try_array_element_type(array.id, semantics)
                    .expect("reachable arrays have lowerable element types"),
            );
            array_functions.insert_push(
                array.id,
                declare(
                    format!("__splitscript::array#{}::push", array.id.index()),
                    vec![array_type, element_type],
                    vec![],
                ),
            );
        }
        if reachability.requires_array_remove_at(array.id) {
            array_functions.insert_remove_at(
                array.id,
                declare(
                    format!("__splitscript::array#{}::removeAt", array.id.index()),
                    vec![array_type, ValType::I32],
                    vec![],
                ),
            );
        }
        if reachability.requires_array_clear(array.id) {
            array_functions.insert_clear(
                array.id,
                declare(
                    format!("__splitscript::array#{}::clear", array.id.index()),
                    vec![array_type],
                    vec![],
                ),
            );
        }
    }

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
                new: declare(
                    super::debug_artifacts::set_function_name(set.id, "new"),
                    vec![],
                    vec![set_type],
                ),
                length: declare(
                    super::debug_artifacts::set_function_name(set.id, "length"),
                    vec![set_type],
                    vec![ValType::I32],
                ),
                contains: declare(
                    super::debug_artifacts::set_function_name(set.id, "contains"),
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                insert: declare(
                    super::debug_artifacts::set_function_name(set.id, "insert"),
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                remove: declare(
                    super::debug_artifacts::set_function_name(set.id, "remove"),
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                clear: declare(
                    super::debug_artifacts::set_function_name(set.id, "clear"),
                    vec![set_type],
                    vec![],
                ),
            },
        );
    }

    let mut managed_state_read_functions = HashMap::new();
    for storage in managed_state_reads.entries() {
        managed_state_read_functions.insert(
            storage.field,
            declare(
                format!(
                    "__splitscript::managed::class#{}::field#{}::read",
                    storage.class.index(),
                    storage.field.index(),
                ),
                vec![],
                vec![gc.val_type(Type::Result(storage.result))],
            ),
        );
    }

    let mut managed_snapshot_functions = HashMap::new();
    for class in reachability.managed_snapshots() {
        let class_name = program
            .managed_class(class)
            .expect("reachable managed snapshot classes have declarations")
            .name
            .as_str();
        let snapshot = semantics.types().id_for_managed_class(class);
        let result = semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Result { layout, value } if *value == snapshot => {
                    Some(*layout)
                }
                _ => None,
            })
            .expect("reachable managed snapshot calls have Result layouts");
        managed_snapshot_functions.insert(
            class,
            declare(
                format!("__splitscript::managed::{class_name}::snapshot"),
                vec![ValType::I64],
                vec![gc.val_type(Type::Result(result))],
            ),
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
                ty.has_runtime_value().then(|| gc.val_type(ty))
            })
            .collect::<Vec<_>>();
        let body = wasm_ir
            .body(crate::wasm_ir::BodyOwner::Function(instance.clone()))
            .expect("reachable functions have Wasm IR bodies");
        let source_name = super::debug_artifacts::user_function_name(
            &function.name,
            instance,
            program,
            semantics,
            standard_library,
            enums,
        );
        let plan = if matches!(body.abi, crate::wasm_ir::BodyAbi::AsyncFunction(_)) {
            let frame = ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(gc.function_frame_index(instance)),
            });
            UserFunctionPlan {
                call: declare(format!("{source_name}::init"), params, vec![frame]),
                poll: Some(declare(
                    format!("{source_name}::poll"),
                    vec![frame],
                    vec![ValType::I32],
                )),
            }
        } else {
            UserFunctionPlan {
                call: declare(
                    source_name,
                    params,
                    result
                        .has_runtime_value()
                        .then(|| gc.val_type(result))
                        .into_iter()
                        .collect(),
                ),
                poll: None,
            }
        };
        users.insert(instance.clone(), plan);
    }

    let mut leaf_futures = HashMap::new();
    for (instance, _) in async_frames.leaves() {
        let frame = ValType::Ref(RefType {
            nullable: false,
            heap_type: HeapType::Concrete(gc.leaf_frame_index(instance)),
        });
        leaf_futures.insert(
            instance.clone(),
            declare(
                format!(
                    "__splitscript::future::expr{}::poll",
                    instance.expression.index()
                ),
                vec![frame],
                vec![ValType::I32],
            ),
        );
    }

    let state_ref = ValType::Ref(RefType {
        nullable: false,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
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
            let mut parameters = vec![ValType::I64];
            let has_dependencies = !semantics.state_dependencies(field.id).is_empty();
            if has_dependencies {
                parameters.push(state_ref);
            }
            if pointer_prefixes.field(field.id).is_some() {
                parameters.extend([ValType::I64, ValType::I32]);
            }
            reads.push(declare(
                format!("state::{}::read", field.name),
                parameters,
                vec![gc.val_type(poll_result)],
            ));
            transforms.push(field.transform.as_ref().map(|_| {
                let ty = semantic_type(
                    semantics
                        .value_type(field.id)
                        .expect("checked state fields have types"),
                    semantics,
                );
                let value = gc.val_type(ty);
                declare(
                    format!("state::{}::transform", field.name),
                    if has_dependencies {
                        vec![value, state_ref]
                    } else {
                        vec![value]
                    },
                    vec![gc.val_type(poll_result)],
                )
            }));
        }
    }

    let mut actions = HashMap::new();
    for action in &program.actions {
        let (params, results) = match action.kind {
            ActionKind::Setup
            | ActionKind::SelectProcess
            | ActionKind::OnDetach
            | ActionKind::OnStart
            | ActionKind::OnReset => {
                let results = (action.kind == ActionKind::SelectProcess)
                    .then(|| action_result_val_type(action.kind, semantics, gc))
                    .into_iter()
                    .collect();
                (vec![], results)
            }
            ActionKind::OnAttach => (vec![ValType::I64], vec![ValType::I32]),
            action => (
                vec![state_ref, state_ref],
                (!matches!(
                    action,
                    ActionKind::OnDetach
                        | ActionKind::OnStateReady
                        | ActionKind::OnStart
                        | ActionKind::OnReset
                ))
                .then(|| action_result_val_type(action, semantics, gc))
                .into_iter()
                .collect(),
            ),
        };
        actions.insert(
            action.kind,
            declare(action.kind.name().to_owned(), params, results),
        );
    }

    let start = declare("_start".to_owned(), vec![], vec![]);
    let update = declare("update".to_owned(), vec![], vec![]);
    let mut closure_polls = HashMap::new();
    for (instance, _) in async_frames.closures() {
        let frame = ValType::Ref(RefType {
            nullable: false,
            heap_type: HeapType::Concrete(gc.closure_frame_index(instance)),
        });
        closure_polls.insert(
            instance.clone(),
            declare(
                format!(
                    "__splitscript::closure::expr{}::poll",
                    instance.expression.index()
                ),
                vec![frame],
                vec![ValType::I32],
            ),
        );
    }
    let mut closures = HashMap::new();
    for instance in reachability.closure_instances() {
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("reachable closure instances have bodies");
        let expression = wasm_ir
            .expression(closure.expression)
            .expect("closure expressions belong to Wasm IR");
        let ty = instance.owner.as_ref().map_or(expression.ty, |owner| {
            semantics.specialize_type(owner, expression.ty)
        });
        let crate::types::TypeKind::Callable { layout, .. } = semantics.types().kind(ty) else {
            unreachable!("checked closure expressions have callable types")
        };
        section.function(gc.callable_function_index(*layout));
        let function_index = next_function;
        next_function += 1;
        debug_names.push((
            function_index,
            format!("__splitscript::closure::expr{}", closure.expression.index()),
        ));
        closures.insert(instance.clone(), function_index);
    }
    let mut function_values = HashMap::new();
    for instance in reachability.function_value_instances() {
        let crate::types::TypeKind::Callable { layout, .. } = semantics.types().kind(instance.ty)
        else {
            unreachable!("function-value adapters have callable layouts")
        };
        section.function(gc.callable_function_index(*layout));
        let function_index = next_function;
        next_function += 1;
        let function = program
            .functions
            .iter()
            .find(|function| function.id == instance.function.function)
            .expect("reachable function values have source declarations");
        debug_names.push((
            function_index,
            format!("__splitscript::function-value::{}", function.name),
        ));
        function_values.insert(instance.clone(), function_index);
    }

    FunctionPlan {
        section,
        runtime_helpers,
        equality,
        array_functions,
        sets: set_functions,
        users,
        closures,
        function_values,
        closure_polls,
        leaf_futures,
        displays,
        managed_state_reads: managed_state_read_functions,
        managed_snapshots: managed_snapshot_functions,
        reads,
        transforms,
        actions,
        start,
        update,
        arrays,
        debug_names,
    }
}
