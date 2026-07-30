use super::*;

/// The complete, deterministic assignment of generated function signatures
/// and indices. Body generation consumes this plan but cannot mutate the
/// shared type or function index spaces.
pub(super) struct FunctionPlan<'a> {
    pub section: FunctionSection,
    pub stdlib: Stdlib,
    pub equality: EqualityFunctions,
    pub users: HashMap<FunctionId, u32>,
    pub reads: Vec<u32>,
    pub actions: HashMap<ActionKind, u32>,
    pub start: u32,
    pub update: u32,
    pub string_values: Option<&'a ArrayTypeDecl>,
    pub u64_offsets: Option<&'a ArrayTypeDecl>,
}

pub(super) struct Inputs<'a> {
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ArrayTypeDecl],
    pub options: &'a [OptionTypeDecl],
    pub results: &'a [ResultTypeDecl],
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

    let string_values = arrays
        .iter()
        .find(|array| array_element_type(array.id, semantics) == Type::String);
    let u64_offsets = arrays
        .iter()
        .find(|array| array_element_type(array.id, semantics) == Type::U64);
    let string_ref = gc.val_type(Type::String);
    let mut helper_functions = HashMap::new();
    for helper in dependencies
        .core_helpers()
        .chain(dependencies.settings_helpers())
    {
        let (params, results) = match helper {
            GeneratedHelper::PrintString => (vec![string_ref], vec![]),
            GeneratedHelper::TimerSetVariable => (vec![string_ref, string_ref], vec![]),
            GeneratedHelper::FormatI64 => (vec![ValType::I64, ValType::I32], vec![string_ref]),
            GeneratedHelper::ConcatStrings => (
                vec![
                    gc.val_type(Type::Array(
                        string_values
                            .expect("String concatenation has a String array layout")
                            .id,
                    )),
                ],
                vec![string_ref],
            ),
            GeneratedHelper::StringEquality => (vec![string_ref, string_ref], vec![ValType::I32]),
            GeneratedHelper::ScanProcessRange => (
                vec![
                    ValType::I64,
                    ValType::I64,
                    ValType::I64,
                    ValType::I32,
                    ValType::I32,
                    ValType::I32,
                ],
                vec![ValType::I64],
            ),
            GeneratedHelper::ReadRelative32 => {
                (vec![ValType::I64, ValType::I64], vec![ValType::I64])
            }
            GeneratedHelper::ReadManagedString => (
                vec![ValType::I64, ValType::I64, ValType::I32],
                vec![string_ref],
            ),
            GeneratedHelper::FollowAddress => (
                vec![
                    ValType::I64,
                    ValType::I64,
                    gc.val_type(Type::Array(
                        u64_offsets
                            .expect("address following has a u64 array layout")
                            .id,
                    )),
                ],
                vec![ValType::I64],
            ),
            GeneratedHelper::UnityAttach => (
                vec![ValType::I64, ValType::I32],
                vec![gc.val_type(Type::UnityModule)],
            ),
            GeneratedHelper::UnityGetImage => (
                vec![ValType::I64, gc.val_type(Type::UnityModule), string_ref],
                vec![gc.val_type(Type::UnityImage)],
            ),
            GeneratedHelper::UnityGetClass => (
                vec![ValType::I64, gc.val_type(Type::UnityImage), string_ref],
                vec![gc.val_type(Type::UnityClass)],
            ),
            GeneratedHelper::UnityGetFieldOffset => (
                vec![ValType::I64, gc.val_type(Type::UnityClass), string_ref],
                vec![ValType::I64],
            ),
            GeneratedHelper::UnityGetFieldAny => (
                vec![
                    ValType::I64,
                    gc.val_type(Type::UnityClass),
                    gc.val_type(Type::Array(
                        string_values
                            .expect("field alternatives have a String array layout")
                            .id,
                    )),
                ],
                vec![gc.val_type(Type::UnityField)],
            ),
            GeneratedHelper::UnityGetStaticInstance => (
                vec![
                    ValType::I64,
                    gc.val_type(Type::UnityClass),
                    gc.val_type(Type::Array(
                        string_values
                            .expect("static instances have a String array layout")
                            .id,
                    )),
                ],
                vec![ValType::I64],
            ),
            GeneratedHelper::CStringEquality => (
                vec![
                    ValType::I64,
                    ValType::I64,
                    string_ref,
                    ValType::I32,
                    ValType::I32,
                ],
                vec![ValType::I32],
            ),
            GeneratedHelper::BackingFieldEquality => (
                vec![ValType::I64, ValType::I64, string_ref],
                vec![ValType::I32],
            ),
            GeneratedHelper::StringFromMemory => {
                (vec![ValType::I32, ValType::I32], vec![string_ref])
            }
            GeneratedHelper::RefreshSettings => (vec![], vec![]),
        };
        helper_functions.insert(helper, declare(params, results));
    }

    let mut equality = EqualityFunctions::default();
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

    let stdlib = Stdlib {
        helpers: helper_functions,
    };

    let mut users = HashMap::new();
    for function in &program.functions {
        if !reachability.contains_function(function.id) {
            continue;
        }
        let result = function_result(function.id, semantics);
        let index = declare(
            function
                .params
                .iter()
                .map(|parameter| gc.val_type(value_type(parameter.id, semantics)))
                .collect(),
            (result != Type::Void)
                .then(|| gc.val_type(result))
                .into_iter()
                .collect(),
        );
        users.insert(function.id, index);
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
                .then(|| action_result_val_type(action.kind))
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
        stdlib,
        equality,
        users,
        reads,
        actions,
        start,
        update,
        string_values,
        u64_offsets,
    }
}
