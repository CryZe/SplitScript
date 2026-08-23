use wasm_encoder::{
    AbstractHeapType, ConstExpr, Function, HeapType, Instruction, MemArg, RefType, ValType,
};

use crate::ast::{
    ActionKind, ArrayTypeId, EnumDecl, EnumVariantId, ExprId, OptionTypeId, Program, RecordFieldId,
    ResultTypeId, ValueId,
};
use crate::equality::EqualityCapabilities;
use crate::memory::{MemoryLayouts, MemoryTypeLayout};
use crate::semantic::{FunctionInstance, ResolvedReceiver, SemanticModel};
use crate::stdlib::{
    Implementation, IntrinsicId, StandardLibrary, StateProviderAttachment, StateProviderProcesses,
    StdlibCapabilityId, StdlibItemId, StdlibTypeId,
};
use crate::types::{
    ResolvedArrayType, ResolvedOptionType, ResolvedRangeType, ResolvedResultType, TypeId, TypeKind,
};
use crate::wasm_ir::{self, BodyOwner};

mod array_functions;
mod array_value;
mod async_frame;
mod async_state;
mod backend_type;
mod code_bodies;
mod context;
mod data_plan;
mod debug_artifacts;
mod dependencies;
mod display;
mod display_plan;
mod equality_plan;
mod expression;
mod function_plan;
mod gc_layout;
mod gc_types;
mod global_plan;
mod imports;
mod memory_plan;
mod module_assembly;
mod module_start;
mod reachability;
mod runtime_helper_registry;
mod runtime_helpers;
mod script_functions;
mod set_functions;
mod settings;
mod specialization;
mod unity_layout;
mod update;

use self::array_functions::ArrayFunctions;
use self::async_frame::AsyncFrameLayouts;
use self::async_state::{
    compile_async_attach, compile_async_function_poll, compile_intrinsic_future_poll,
};
use self::backend_type::Type;
use self::context::{AttachContext, EmissionContext};
use self::data_plan::StaticData;
use self::dependencies::BackendDependencies;
use self::display_plan::DisplayFunctions;
use self::equality_plan::EqualityFunctions;
use self::gc_layout::GcLayout;
use self::global_plan::SettingStorage;
use self::module_start::compile_start;
use self::runtime_helper_registry::RuntimeHelperPlan;
use self::script_functions::{
    LocalPlanOptions, compile_action, compile_async_function_init, compile_read,
    compile_state_transform, compile_user_function, plan_wasm_locals,
};
use self::set_functions::SetFunctions;
use self::update::{ProviderAttach, StatePollFunctions, compile_update};
use crate::intrinsic_registry::RuntimeHelperId;

const STATE_TYPE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryByteOrder {
    Little,
    Big,
}

impl From<crate::intrinsic_registry::ProviderByteOrder> for MemoryByteOrder {
    fn from(value: crate::intrinsic_registry::ProviderByteOrder) -> Self {
        match value {
            crate::intrinsic_registry::ProviderByteOrder::Little => Self::Little,
            crate::intrinsic_registry::ProviderByteOrder::Big => Self::Big,
        }
    }
}

fn standard_display_function(
    source: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<(StdlibTypeId, FunctionInstance)> {
    let TypeKind::Standard(standard) = semantics.types().kind(source) else {
        return None;
    };
    let item = standard_library.display_implementation(*standard)?;
    let Implementation::LibraryBody { function_name, .. } = item.implementation else {
        unreachable!("validated custom Display implementations have source bodies")
    };
    let function = program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("custom Display bodies are injected into the program");
    let string = semantics.types().id_for_standard(StdlibTypeId::String);
    Some((
        *standard,
        semantics.function_instance(function.id, vec![source, string]),
    ))
}

fn display_function(
    source: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<(TypeId, FunctionInstance)> {
    if let Some((_, function)) =
        standard_display_function(source, program, semantics, standard_library)
    {
        return Some((source, function));
    }
    let function = capabilities.method_implementation(
        source,
        StdlibCapabilityId::Display,
        StdlibItemId::DisplayToString,
        semantics,
    )?;
    let signature = semantics
        .function_parameter_types(function)
        .iter()
        .copied()
        .chain(semantics.function_result(function))
        .collect();
    Some((source, semantics.function_instance(function, signature)))
}

fn provider_attachment_function(
    provider: &crate::stdlib::StdlibStateProvider,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
) -> Option<FunctionInstance> {
    let StateProviderAttachment::Callable(item) = provider.attachment else {
        return None;
    };
    let Implementation::LibraryBody { function_name, .. } =
        standard_library.item(item).implementation
    else {
        return None;
    };
    let function = program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("source-defined provider attachments are injected into the program");
    let signature = semantics
        .function_parameter_types(function.id)
        .iter()
        .copied()
        .chain(semantics.function_result(function.id))
        .collect();
    Some(semantics.function_instance(function.id, signature))
}

struct ConstructedTypes {
    enums: Vec<EnumDecl>,
    arrays: Vec<ResolvedArrayType>,
    options: Vec<ResolvedOptionType>,
    results: Vec<ResolvedResultType>,
    asyncs: Vec<crate::types::ResolvedAsyncType>,
    ranges: Vec<ResolvedRangeType>,
    sets: Vec<crate::types::ResolvedSetType>,
    applications: Vec<crate::types::ResolvedApplicationType>,
}

/// Complete, immutable input to backend planning and Wasm encoding.
///
/// Earlier compiler products are gathered once when Wasm IR is lowered. The
/// encoder therefore cannot be called with a mismatched syntax tree, semantic
/// model, constructed-type table, or capability analysis.
pub struct BackendProgram<'a> {
    standard_library: StandardLibrary,
    program: &'a Program,
    semantics: SemanticModel,
    wasm_ir: wasm_ir::Program,
    constructed_types: ConstructedTypes,
    memory_layouts: &'a MemoryLayouts,
    equality: &'a EqualityCapabilities,
    capabilities: &'a crate::capabilities::CapabilityAnalysis,
    source_name: &'a str,
    source: &'a str,
}

impl<'a> BackendProgram<'a> {
    pub(crate) fn new(checked: &'a crate::CheckedProgram, wasm_ir: wasm_ir::Program) -> Self {
        let mut semantics = checked.semantics.clone();
        let mut constructed_types = ConstructedTypes {
            enums: checked.enum_types.clone(),
            arrays: checked.array_types.clone(),
            options: checked.option_types.clone(),
            results: checked.result_types.clone(),
            asyncs: checked.async_types.clone(),
            ranges: checked.range_types.clone(),
            sets: checked.set_types.clone(),
            applications: checked.application_types.clone(),
        };
        specialization::materialize(
            &wasm_ir,
            &mut semantics,
            &mut constructed_types.arrays,
            &mut constructed_types.options,
            &mut constructed_types.results,
            &mut constructed_types.asyncs,
            &mut constructed_types.ranges,
            &mut constructed_types.sets,
            &mut constructed_types.applications,
        );
        Self {
            standard_library: checked.context.standard_library(),
            program: &checked.compilation_syntax,
            semantics,
            wasm_ir,
            constructed_types,
            memory_layouts: checked.capabilities.memory(),
            equality: checked.capabilities.equality(),
            capabilities: &checked.capabilities,
            source_name: checked.source_name(),
            source: checked.document.source(),
        }
    }

    pub fn wasm_ir(&self) -> &wasm_ir::Program {
        &self.wasm_ir
    }
}

impl std::ops::Deref for BackendProgram<'_> {
    type Target = wasm_ir::Program;

    fn deref(&self) -> &Self::Target {
        self.wasm_ir()
    }
}

pub fn compile(inputs: BackendProgram<'_>) -> Vec<u8> {
    let intrinsic_effect_errors = runtime_helper_registry::validate_intrinsic_effects();
    assert!(
        intrinsic_effect_errors.is_empty(),
        "invalid trusted intrinsic implementation contracts: {}",
        intrinsic_effect_errors.join("; ")
    );
    let unity_layout_errors = unity_layout::validate();
    assert!(
        unity_layout_errors.is_empty(),
        "invalid trusted Unity/IL2CPP layout descriptors: {}",
        unity_layout_errors.join("; ")
    );
    let BackendProgram {
        standard_library,
        program,
        semantics,
        wasm_ir,
        constructed_types,
        memory_layouts,
        equality,
        capabilities,
        source_name,
        source,
    } = inputs;
    let ConstructedTypes {
        enums,
        arrays: array_types,
        options: option_types,
        results: result_types,
        asyncs: async_types,
        ranges: range_types,
        sets: set_types,
        applications: application_types,
    } = constructed_types;
    let semantics = &semantics;
    let enums = &enums;
    let array_types = &array_types;
    let option_types = &option_types;
    let result_types = &result_types;
    let set_types = &set_types;
    let application_types = &application_types;
    let wasm_ir = &wasm_ir;
    let state = program.state.as_ref().unwrap();
    let provider = semantics
        .state_provider()
        .map(|provider| standard_library.state_provider(provider))
        .expect("checked state declarations resolve a standard-library provider");
    let process_names = match provider.processes {
        StateProviderProcesses::Declared(processes) => processes.to_vec(),
        StateProviderProcesses::SourceState => state.processes.iter().map(String::as_str).collect(),
    };
    let on_attach = program
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OnAttach)
        .map(|action| action.kind);
    let cancellation_region = on_attach.and_then(|action| {
        wasm_ir
            .body(BodyOwner::Action(action))
            .and_then(|body| body.cancellation_region)
    });
    let provider_attachment =
        provider_attachment_function(provider, program, semantics, &standard_library);
    let mut reachability = reachability::Reachability::analyze(
        program,
        semantics,
        wasm_ir,
        &standard_library,
        capabilities,
        provider_attachment.clone(),
    );
    let async_frames =
        AsyncFrameLayouts::plan(on_attach, program, wasm_ir, semantics, &reachability);
    let async_layout = async_frames.attach.as_ref();
    let dependencies = BackendDependencies::analyze(program, semantics, wasm_ir, &reachability);
    reachability.require_runtime_helper_types(&dependencies, array_types, semantics);
    let static_data = StaticData::collect(
        program,
        &process_names,
        wasm_ir,
        &reachability,
        memory_layouts,
    );
    let strings = &static_data.strings;
    let signatures = &static_data.signatures;

    let gc_types::EncodedTypes {
        section: mut types,
        next_type_index: first_import_type,
        layout: gc,
    } = gc_types::encode(gc_types::Inputs {
        standard_library: &standard_library,
        program,
        semantics,
        async_layout,
        async_frames: &async_frames,
        enums,
        array_types,
        option_types,
        result_types,
        async_types: &async_types,
        set_types,
        application_types,
        range_types: &range_types,
        reachability: &reachability,
    });
    let imports::EncodedImports {
        section: imports,
        abi,
        function_count: imported_functions,
        next_type_index,
    } = imports::encode(&mut types, first_import_type, &dependencies);

    let global_plan::GlobalPlan {
        section: globals,
        runtime: runtime_globals,
        variables: global_indices,
        variable_types: global_types,
        settings: setting_indices,
    } = global_plan::encode(
        program,
        semantics,
        &gc,
        wasm_ir,
        provider_attachment.as_ref(),
    );

    let function_plan::FunctionPlan {
        section: functions,
        runtime_helpers,
        equality: equality_functions,
        array_functions,
        sets: set_functions,
        users: user_functions,
        intrinsic_futures,
        displays: display_functions,
        reads: read_functions,
        transforms: transform_functions,
        actions: action_functions,
        start: start_function,
        update: update_function,
        arrays: helper_arrays,
        debug_names: function_debug_names,
    } = function_plan::encode(
        &mut types,
        next_type_index,
        imported_functions,
        function_plan::Inputs {
            standard_library: &standard_library,
            program,
            semantics,
            enums,
            arrays: array_types,
            options: option_types,
            results: result_types,
            sets: set_types,
            equality,
            structural: capabilities.structural_types(),
            dependencies: &dependencies,
            reachability: &reachability,
            gc: &gc,
            wasm_ir,
            async_frames: &async_frames,
        },
    );
    let debug_recorder = (wasm_ir.profile() == crate::BuildProfile::Debug)
        .then(debug_artifacts::DebugRecorder::default);
    let mut codes = code_bodies::CodeBodies::new(
        imported_functions,
        function_debug_names.len(),
        debug_recorder.as_ref(),
    );
    let lowering = EmissionContext {
        standard_library: &standard_library,
        abi: &abi,
        state,
        globals: &global_indices,
        global_types: &global_types,
        settings: &setting_indices,
        runtime_globals,
        runtime_helpers: &runtime_helpers,
        functions: &user_functions,
        intrinsic_futures: &intrinsic_futures,
        display_functions: &display_functions,
        equality_functions: &equality_functions,
        array_functions: &array_functions,
        set_functions: &set_functions,
        records: &program.records,
        enums,
        arrays: array_types,
        memory: memory_layouts,
        abi_read: static_data.layout().scratch().abi_read,
        signatures,
        semantics,
        wasm_ir,
        gc: &gc,
        async_frames: &async_frames,
        debug: debug_recorder.as_ref(),
    };
    let runtime = AttachContext {
        abi: &abi,
        strings,
        lowering: &lowering,
    };
    let settings_context = settings::SettingsContext {
        standard_library: &standard_library,
        abi: &abi,
        enums,
        gc: &gc,
        globals: &global_indices,
        runtime_globals,
        semantics,
        wasm_ir,
    };
    let provider_attach = provider_attachment.as_ref().map(|instance| {
        let plan = user_functions[instance];
        let layout = async_frames
            .function(instance)
            .expect("source provider attachments must suspend");
        let (completion_field, completion_type) = layout
            .completion
            .expect("source provider attachments return their provider value");
        debug_assert_eq!(completion_type, Type::Standard(provider.process_type));
        ProviderAttach {
            init: plan.call,
            poll: plan.poll.expect("source provider attachments are async"),
            frame_global: runtime_globals
                .provider_attachment_frame
                .expect("source provider attachments have frame storage"),
            frame_type: gc.function_frame_index(instance),
            completion_field,
        }
    });
    let attachment_globals = wasm_ir.attachment_globals().collect::<Vec<_>>();
    let update_context = update::UpdateContext {
        standard_library: &standard_library,
        abi: &abi,
        gc: &gc,
        runtime_globals,
        semantics,
        globals: &global_indices,
        global_types: &global_types,
        attachment_globals: &attachment_globals,
        process_names: &process_names,
        provider_attach,
    };

    let helper_inputs = runtime_helpers::RuntimeHelperInputs {
        abi: &abi,
        strings,
        plan: &runtime_helpers,
        arrays: helper_arrays,
        program,
        semantics,
        settings: &settings_context,
        settings_map: &setting_indices,
        gc: &gc,
        memory: static_data.layout(),
    };
    let helper_bodies = runtime_helpers::compile_runtime(&runtime_helpers, &helper_inputs);
    let equality_bodies = runtime_helpers::compile_equality(
        &runtime_helpers,
        capabilities.structural_types(),
        option_types,
        result_types,
        semantics,
        &equality_functions,
        &gc,
    );
    let display_bodies = display::compile(&display::DisplayInputs {
        structural: capabilities.structural_types(),
        arrays: array_types,
        semantics,
        displays: &display_functions,
        users: &user_functions,
        helpers: &runtime_helpers,
        gc: &gc,
    });
    let array_bodies = array_functions::compile(array_types, &array_functions, semantics, &gc);
    let set_bodies = set_functions::compile(
        set_types,
        &set_functions,
        semantics,
        &equality_functions,
        runtime_helpers
            .optional_function(RuntimeHelperId::StringEquality)
            .unwrap_or(0),
        &gc,
    );
    for body in helper_bodies {
        codes.push(&body);
    }
    let refresh_settings = runtime_helpers.optional_function(RuntimeHelperId::RefreshSettings);
    for body in equality_bodies {
        codes.push(&body);
    }
    for body in display_bodies {
        codes.push(&body);
    }
    for body in array_bodies {
        codes.push(&body);
    }
    for body in set_bodies {
        codes.push(&body);
    }
    for instance in reachability.functions() {
        let function = program
            .functions
            .iter()
            .find(|function| function.id == instance.function)
            .expect("reachable function instances have source declarations");
        if let Some(layout) = async_frames.function(instance) {
            let plan = user_functions
                .get(instance)
                .expect("reachable functions have final function plans");
            let body = compile_async_function_init(function, instance, layout, &lowering);
            codes.push(&body);
            let body = compile_async_function_poll(
                instance,
                plan.poll.expect("async functions have poll entry points"),
                layout,
                &runtime,
            );
            codes.push(&body);
        } else {
            let function_index = user_functions[instance].call;
            let body = compile_user_function(function, instance, function_index, &lowering);
            codes.push(&body);
        }
    }
    for (instance, layout) in async_frames.intrinsics() {
        let function_index = intrinsic_futures[instance];
        let body = compile_intrinsic_future_poll(instance, function_index, layout, &runtime);
        codes.push(&body);
    }
    for (field_index, field) in state.all_fields().enumerate() {
        let body = compile_read(field, read_functions[field_index], &abi, strings, &lowering);
        codes.push(&body);
        if field.transform.is_some() {
            let function_index = transform_functions[field_index]
                .expect("filtered state fields have transform functions");
            let body = compile_state_transform(field, function_index, &lowering);
            codes.push(&body);
        }
    }
    for action in &program.actions {
        if action.kind == ActionKind::OnAttach {
            let body = compile_async_attach(
                action,
                action_functions[&action.kind],
                async_layout.unwrap(),
                &runtime,
            );
            codes.push(&body);
        } else {
            let body = compile_action(action, action_functions[&action.kind], &lowering);
            codes.push(&body);
        }
    }
    let body = compile_start(
        program,
        &settings_context,
        &lowering,
        strings,
        &setting_indices,
        module_start::StartFunctions {
            start: start_function,
            refresh_settings,
            setup: action_functions.get(&ActionKind::Setup).copied(),
        },
        async_layout.is_some(),
    );
    codes.push(&body);
    let body = compile_update(
        program,
        strings,
        StatePollFunctions {
            reads: &read_functions,
            transforms: &transform_functions,
        },
        &action_functions,
        refresh_settings,
        cancellation_region,
        &update_context,
    );
    codes.push(&body);

    let debug_artifacts = debug_recorder.as_ref().map(|recorder| {
        debug_artifacts::DebugArtifactPlan::new(debug_artifacts::DebugArtifactInputs {
            abi: &abi,
            defined_functions: &function_debug_names,
            recorder,
            global_indices: &global_indices,
            global_types: &global_types,
            program,
            source_name,
            source,
        })
    });

    module_assembly::finish(
        module_assembly::Sections {
            types,
            imports,
            functions,
            globals,
            codes: codes.finish(),
        },
        &static_data,
        start_function,
        update_function,
        debug_artifacts.as_ref(),
    )
}

fn resolved_intrinsic(target: &wasm_ir::CallTarget) -> Option<IntrinsicId> {
    match target {
        wasm_ir::CallTarget::Intrinsic { intrinsic, .. } => Some(*intrinsic),
        wasm_ir::CallTarget::UserFunction { .. }
        | wasm_ir::CallTarget::UserMethod { .. }
        | wasm_ir::CallTarget::LibraryOverload { .. }
        | wasm_ir::CallTarget::ResultError { .. }
        | wasm_ir::CallTarget::OptionSome { .. }
        | wasm_ir::CallTarget::IteratorItem { .. }
        | wasm_ir::CallTarget::ResultSuccess { .. } => None,
    }
}

fn call_target(wasm_ir: &wasm_ir::Program, expression: ExprId) -> Option<&wasm_ir::CallTarget> {
    let wasm_ir::ExpressionKind::Call { target, .. } = &wasm_ir
        .expression(expression)
        .expect("checked call belongs to Wasm IR")
        .kind
    else {
        return None;
    };
    Some(target)
}

fn semantic_type(id: TypeId, semantics: &SemanticModel) -> Type {
    match semantics.types().kind(id) {
        TypeKind::Error => unreachable!("failed inference reached code generation"),
        TypeKind::Builtin(builtin) => Type::from_core(*builtin),
        TypeKind::Standard(standard) => Type::Standard(*standard),
        TypeKind::StateSnapshot => Type::StateSnapshot,
        TypeKind::SettingsView => Type::SettingsView,
        TypeKind::Record(record) => Type::Record(*record),
        TypeKind::Enum(enumeration) => Type::Enum(*enumeration),
        TypeKind::GenericParameter { .. } => {
            unreachable!("generic template types must be substituted before code generation")
        }
        TypeKind::Array { layout, .. } => Type::Array(*layout),
        TypeKind::Option { layout, .. } => Type::Option(*layout),
        TypeKind::Result { layout, .. } => Type::Result(*layout),
        TypeKind::Async { layout, .. } => Type::Async(*layout),
        TypeKind::Set { layout, .. } => Type::Set(*layout),
        TypeKind::Range { layout, .. } => Type::Range(*layout),
        TypeKind::Application { layout, .. } => Type::Application(*layout),
    }
}

fn application_type_argument(
    application: crate::ast::TypeApplicationId,
    expected_constructor: crate::stdlib::StdlibTypeConstructorId,
    semantics: &SemanticModel,
) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Application {
                layout,
                constructor,
                arguments,
            } if *layout == application && *constructor == expected_constructor => {
                Some(semantic_type(arguments[0], semantics))
            }
            _ => None,
        })
        .expect("checked named application has its declared constructor and argument")
}

fn range_bound_type(range: crate::ast::RangeTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Range { layout, bound, .. } if *layout == range => {
                Some(semantic_type(*bound, semantics))
            }
            _ => None,
        })
        .expect("checked range has a concrete bound type")
}

fn value_type(value: ValueId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .value_type(value)
            .expect("checked value declarations have semantic types"),
        semantics,
    )
}

fn state_storage_index(field: ValueId, semantics: &SemanticModel) -> (u32, ValueId) {
    let storage = semantics
        .state_storage_field(field)
        .expect("checked state fields have physical storage");
    let index = semantics
        .state_storage_fields()
        .iter()
        .position(|candidate| *candidate == storage)
        .expect("physical state field belongs to the snapshot layout");
    (index as u32, storage)
}

fn record_field_type(field: RecordFieldId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .record_field_type(field)
            .expect("checked record fields have semantic types"),
        semantics,
    )
}

fn standard_field_type(field: crate::stdlib::StdlibFieldId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .standard_field_type(field)
            .expect("checked standard fields have semantic types"),
        semantics,
    )
}

fn enum_variant_payload(variant: EnumVariantId, semantics: &SemanticModel) -> Option<Type> {
    semantics
        .enum_variant_payload(variant)
        .map(|payload| semantic_type(payload, semantics))
}

fn array_element_type(array: ArrayTypeId, semantics: &SemanticModel) -> Type {
    try_array_element_type(array, semantics)
        .expect("concrete array layouts have backend-representable element types")
}

fn set_element_type(set: crate::ast::TypeApplicationId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Set {
                layout, element, ..
            } if *layout == set => Some(semantic_type(*element, semantics)),
            _ => None,
        })
        .expect("checked set layouts have semantic element types")
}

fn try_array_element_type(array: ArrayTypeId, semantics: &SemanticModel) -> Option<Type> {
    let element = semantics
        .array_element_type(array)
        .expect("checked array layouts have semantic element types");
    (!matches!(
        semantics.types().kind(element),
        TypeKind::GenericParameter { .. }
    ))
    .then(|| semantic_type(element, semantics))
}

fn option_value_type(option: OptionTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Option { layout, value } if *layout == option => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked option layouts have semantic value types")
}

fn result_value_type(result: ResultTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Result { layout, value } if *layout == result => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked result layouts have semantic value types")
}

fn emit_struct_get(function: &mut Function, field_index: u32, ty: Type) {
    emit_typed_struct_get(function, STATE_TYPE, field_index, ty);
}

fn emit_typed_struct_get(
    function: &mut Function,
    struct_type_index: u32,
    field_index: u32,
    ty: Type,
) {
    let instruction = match ty {
        Type::Bool | Type::U8 | Type::U16 => Instruction::StructGetU {
            struct_type_index,
            field_index,
        },
        Type::I8 | Type::I16 => Instruction::StructGetS {
            struct_type_index,
            field_index,
        },
        _ => Instruction::StructGet {
            struct_type_index,
            field_index,
        },
    };
    function.instruction(&instruction);
}

fn emit_array_get(function: &mut Function, array_type_index: u32, element: Type, gc: &GcLayout) {
    function.instruction(&match element {
        Type::Bool | Type::U8 | Type::U16 => Instruction::ArrayGetU(array_type_index),
        Type::I8 | Type::I16 => Instruction::ArrayGetS(array_type_index),
        _ => Instruction::ArrayGet(array_type_index),
    });
    if matches!(
        gc.val_type(element),
        ValType::Ref(RefType {
            nullable: false,
            ..
        })
    ) {
        function.instruction(&Instruction::RefAsNonNull);
    }
}

fn emit_memory_value(
    function: &mut Function,
    ty: TypeId,
    scratch: memory_plan::AbiReadScratch,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
    byte_order: MemoryByteOrder,
) {
    match memory
        .layout(ty, semantics)
        .expect("checked memory values are MemoryReadable")
    {
        MemoryTypeLayout::Scalar { .. } => {
            emit_memory_load(
                function,
                semantic_type(ty, semantics),
                scratch.at(offset),
                byte_order,
            );
        }
        MemoryTypeLayout::Record(layout) => {
            for field in &layout.fields {
                emit_memory_value(
                    function,
                    field.ty,
                    scratch,
                    offset + field.offset,
                    memory,
                    semantics,
                    gc,
                    byte_order,
                );
            }
            function.instruction(&Instruction::StructNew(
                gc.index(semantic_type(layout.ty, semantics)),
            ));
        }
        MemoryTypeLayout::FixedArray(layout) => {
            for index in 0..layout.length {
                emit_memory_value(
                    function,
                    layout.element,
                    scratch,
                    offset + index * layout.stride,
                    memory,
                    semantics,
                    gc,
                    byte_order,
                );
            }
            let Type::Array(array) = semantic_type(layout.ty, semantics) else {
                unreachable!("fixed memory array layouts have array types")
            };
            array_value::emit_new_fixed(function, gc, array, layout.length);
        }
    }
}

fn emit_memory_load(function: &mut Function, ty: Type, address: i32, byte_order: MemoryByteOrder) {
    if byte_order == MemoryByteOrder::Big && !matches!(ty, Type::Bool | Type::U8 | Type::I8) {
        emit_big_endian_memory_load(function, ty, address);
        return;
    }
    function.instruction(&Instruction::I32Const(address));
    function.instruction(&match ty {
        Type::Bool | Type::U8 => Instruction::I32Load8U(memarg()),
        Type::I8 => Instruction::I32Load8S(memarg()),
        Type::U16 => Instruction::I32Load16U(memarg()),
        Type::I16 => Instruction::I32Load16S(memarg()),
        Type::I32 | Type::U32 => Instruction::I32Load(memarg()),
        Type::I64 | Type::U64 | Type::Address => Instruction::I64Load(memarg()),
        Type::F32 => Instruction::F32Load(memarg()),
        Type::F64 => Instruction::F64Load(memarg()),
        _ => unreachable!(),
    });
}

fn emit_big_endian_memory_load(function: &mut Function, ty: Type, address: i32) {
    let (bytes, wide) = match ty {
        Type::U16 | Type::I16 => (2, false),
        Type::I32 | Type::U32 | Type::F32 => (4, false),
        Type::I64 | Type::U64 | Type::Address | Type::F64 => (8, true),
        _ => unreachable!("non-scalar or byte-sized values do not need big-endian assembly"),
    };

    for byte in 0..bytes {
        function
            .instruction(&Instruction::I32Const(address))
            .instruction(&Instruction::I32Load8U(MemArg {
                offset: byte,
                align: 0,
                memory_index: 0,
            }));
        let shift = (bytes - byte - 1) * 8;
        if wide {
            function.instruction(&Instruction::I64ExtendI32U);
            if shift != 0 {
                function
                    .instruction(&Instruction::I64Const(shift as i64))
                    .instruction(&Instruction::I64Shl);
            }
            if byte != 0 {
                function.instruction(&Instruction::I64Or);
            }
        } else {
            if shift != 0 {
                function
                    .instruction(&Instruction::I32Const(shift as i32))
                    .instruction(&Instruction::I32Shl);
            }
            if byte != 0 {
                function.instruction(&Instruction::I32Or);
            }
        }
    }

    match ty {
        Type::I16 => {
            function.instruction(&Instruction::I32Extend16S);
        }
        Type::F32 => {
            function.instruction(&Instruction::F32ReinterpretI32);
        }
        Type::F64 => {
            function.instruction(&Instruction::F64ReinterpretI64);
        }
        _ => {}
    }
}

fn emit_default(function: &mut Function, ty: Type, gc: &GcLayout) {
    function.instruction(&match gc.val_type(ty) {
        ValType::I32 => Instruction::I32Const(0),
        ValType::I64 => Instruction::I64Const(0),
        ValType::F32 => Instruction::F32Const(0.0.into()),
        ValType::F64 => Instruction::F64Const(0.0.into()),
        ValType::Ref(reference) => Instruction::RefNull(reference.heap_type),
        ValType::V128 => unreachable!(),
    });
}

/// Wraps the value already on the operand stack in a successful `T!`.
fn emit_result_success(function: &mut Function, result: ResultTypeId, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::String),
        )))
        .instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

fn emit_result_error(
    function: &mut Function,
    result: ResultTypeId,
    value_type: Type,
    message: &str,
    gc: &GcLayout,
) {
    emit_default(function, value_type, gc);
    function.instruction(&Instruction::I32Const(1));
    emit_string_literal(function, message, gc);
    function.instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

/// Transfers an error to the nearest compiled failure boundary.
///
/// Both an explicit `throw` and the failure arm of postfix `?` lower through
/// this operation. A future nested `catch` can replace the final return with a
/// branch to the selected handler without changing either source construct.
fn emit_failure_transfer(
    function: &mut Function,
    target: ResultTypeId,
    target_value: Type,
    gc: &GcLayout,
    emit_error: impl FnOnce(&mut Function),
) {
    emit_default(function, target_value, gc);
    function.instruction(&Instruction::I32Const(1));
    emit_error(function);
    function
        .instruction(&Instruction::StructNew(gc.index(Type::Result(target))))
        .instruction(&Instruction::Return);
}

fn emit_int(function: &mut Function, value: u64, ty: Type) {
    match ty {
        Type::I64 | Type::U64 | Type::Address => {
            function.instruction(&Instruction::I64Const(value as i64));
        }
        Type::F32 => {
            function.instruction(&Instruction::F32Const((value as f32).into()));
        }
        Type::F64 => {
            function.instruction(&Instruction::F64Const((value as f64).into()));
        }
        _ => {
            function.instruction(&Instruction::I32Const(value as i32));
        }
    }
}

fn emit_string_literal(function: &mut Function, value: &str, gc: &GcLayout) {
    for byte in value.bytes() {
        function.instruction(&Instruction::I32Const(byte as i32));
    }
    function.instruction(&Instruction::ArrayNewFixed {
        array_type_index: gc.standard_index(StdlibTypeId::String),
        array_size: value.len() as u32,
    });
}

fn constant(expression: ExprId, wasm_ir: &wasm_ir::Program, ty: Type) -> ConstExpr {
    let expression = wasm_ir
        .expression(expression)
        .expect("global initializer belongs to Wasm IR");
    let (operator, inner) = match &expression.kind {
        wasm_ir::ExpressionKind::Call {
            target:
                wasm_ir::CallTarget::Intrinsic {
                    intrinsic:
                        intrinsic @ (IntrinsicId::SignedNegate
                        | IntrinsicId::BoolNot
                        | IntrinsicId::IntegerBitNot),
                    receiver:
                        Some(ResolvedReceiver::Expression {
                            expression: inner,
                            members,
                        }),
                    ..
                },
            arguments,
        } if members.is_empty() && arguments.is_empty() => (
            Some(*intrinsic),
            wasm_ir
                .expression(*inner)
                .expect("global initializer operand belongs to Wasm IR"),
        ),
        _ => (None, expression),
    };
    let negative = operator == Some(IntrinsicId::SignedNegate);
    let inverted = operator == Some(IntrinsicId::BoolNot);
    let complemented = operator == Some(IntrinsicId::IntegerBitNot);
    match &inner.kind {
        wasm_ir::ExpressionKind::None => ConstExpr::ref_null(HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::None,
        }),
        wasm_ir::ExpressionKind::Bool(value) => ConstExpr::i32_const((*value ^ inverted) as i32),
        wasm_ir::ExpressionKind::Char(value) => ConstExpr::i32_const(*value as i32),
        wasm_ir::ExpressionKind::Int(value) if ty == Type::F32 => ConstExpr::f32_const(
            (if negative {
                -(*value as f32)
            } else {
                *value as f32
            })
            .into(),
        ),
        wasm_ir::ExpressionKind::Int(value) if ty == Type::F64 => ConstExpr::f64_const(
            (if negative {
                -(*value as f64)
            } else {
                *value as f64
            })
            .into(),
        ),
        wasm_ir::ExpressionKind::Int(value)
            if matches!(ty, Type::I64 | Type::U64 | Type::Address) =>
        {
            ConstExpr::i64_const(if complemented {
                !(*value as i64)
            } else if negative {
                -(*value as i64)
            } else {
                *value as i64
            })
        }
        wasm_ir::ExpressionKind::Int(value) => {
            let value = *value as i32;
            ConstExpr::i32_const(if complemented {
                match ty {
                    Type::U8 => !value & 0xff,
                    Type::U16 => !value & 0xffff,
                    _ => !value,
                }
            } else if negative {
                -value
            } else {
                value
            })
        }
        wasm_ir::ExpressionKind::Float(literal) if ty == Type::F32 => {
            let value = literal
                .normalized
                .parse::<f32>()
                .expect("checked f32 literals fit their target");
            ConstExpr::f32_const((if negative { -value } else { value }).into())
        }
        wasm_ir::ExpressionKind::Float(literal) => {
            let value = literal.value;
            ConstExpr::f64_const((if negative { -value } else { value }).into())
        }
        _ => unreachable!(),
    }
}

fn action_result_val_type(action: ActionKind, gc: &GcLayout) -> ValType {
    if action == ActionKind::GameTime {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(gc.standard_index(StdlibTypeId::Duration)),
        })
    } else {
        ValType::I32
    }
}

fn memarg() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

#[cfg(test)]
mod architecture_tests {
    use std::{fs, path::Path};

    use wasm_encoder::{
        CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, FunctionSection,
        MemorySection, MemoryType, Module, TypeSection,
    };

    use super::*;

    #[test]
    fn big_endian_scalar_decoder_preserves_integer_float_and_sign_bits() {
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        types.ty().function([], [ValType::F32]);
        let mut functions = FunctionSection::new();
        functions.function(0);
        functions.function(1);
        functions.function(0);
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        let mut exports = ExportSection::new();
        exports.export("integer", ExportKind::Func, 0);
        exports.export("float", ExportKind::Func, 1);
        exports.export("signed", ExportKind::Func, 2);
        let mut code = CodeSection::new();
        for (ty, address) in [(Type::U32, 0), (Type::F32, 4), (Type::I16, 8)] {
            let mut function = Function::new([]);
            emit_memory_load(&mut function, ty, address, MemoryByteOrder::Big);
            function.instruction(&Instruction::End);
            code.function(&function);
        }
        let mut data = DataSection::new();
        data.active(
            0,
            &ConstExpr::i32_const(0),
            [0x12, 0x34, 0x56, 0x78, 0x3f, 0xc0, 0x00, 0x00, 0xff, 0xfe],
        );
        let mut module = Module::new();
        module.section(&types);
        module.section(&functions);
        module.section(&memories);
        module.section(&exports);
        module.section(&code);
        module.section(&data);

        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, module.finish()).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "integer")
                .unwrap()
                .call(&mut store, ())
                .unwrap() as u32,
            0x1234_5678
        );
        assert_eq!(
            instance
                .get_typed_func::<(), f32>(&mut store, "float")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            1.5
        );
        assert_eq!(
            instance
                .get_typed_func::<(), i32>(&mut store, "signed")
                .unwrap()
                .call(&mut store, ())
                .unwrap(),
            -2
        );
    }

    fn visit_rust_files(path: &Path, check: &mut impl FnMut(&Path)) {
        for entry in fs::read_dir(path).expect("codegen source directory should be readable") {
            let path = entry
                .expect("codegen source entry should be readable")
                .path();
            if path.is_dir() {
                visit_rust_files(&path, check);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                check(&path);
            }
        }
    }

    #[test]
    fn codegen_modules_do_not_reintroduce_parent_wildcard_imports() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen");
        visit_rust_files(&root, &mut |path| {
            let source = fs::read_to_string(path).expect("codegen source should be UTF-8");
            assert!(
                !source.lines().any(|line| line.trim() == "use super::*;"),
                "{} reintroduced an implicit parent-module prelude",
                path.display()
            );
        });
    }
}
