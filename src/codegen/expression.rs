//! Structured Wasm-IR block, assignment, expression, and intrinsic emission.

use std::collections::HashMap;

use wasm_encoder::{AbstractHeapType, BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, BinaryOp, EnumDecl, ExprId, RangeKind, RecordDecl, ResultTypeId, ValueId},
    intrinsic_registry::RuntimeHelperId,
    memory::MemoryLayouts,
    semantic::{
        FunctionInstance, ResolvedMember, ResolvedReceiver, ResolvedRecordFieldId,
        ResolvedRecordId, ResolvedValue, SemanticModel, ValueConversionKind,
    },
    stdlib::{
        IntrinsicId, MANAGED_BINDINGS_TYPE, MANAGED_POINTER_SIZE_FIELD, RuntimeRepresentation,
        StandardLibrary, StdlibFieldId, StdlibOwner, StdlibTypeConstructorId, StdlibTypeId,
        managed_field_offset_name, managed_static_table_name,
    },
    types::{EnumTypeId, ResolvedArrayType, TypeId},
    wasm_ir::{self, TemporaryId},
};

use super::{
    DisplayFunctions, EqualityFunctions, GcLayout, MemoryByteOrder, RuntimeHelperPlan, STATE_TYPE,
    SetFunctions, SettingStorage, Type, application_type_argument, array_element_type,
    async_frame::{AsyncFrameRef, IntrinsicFutureInstance, IntrinsicFutureLayout},
    emit_array_get, emit_default, emit_failure_transfer, emit_int, emit_memory_value,
    emit_result_error, emit_result_success, emit_string_literal, emit_struct_get,
    emit_typed_struct_get, enum_variant_payload,
    global_plan::RuntimeGlobals,
    imports::Abi,
    managed_state_reads::ManagedStateReadCache,
    memarg,
    memory_plan::AbiReadScratch,
    range_bound_type, record_field_type, resolved_intrinsic, result_value_type,
    runtime_helpers::emit_value_equality,
    script_functions::emit_action_default,
    semantic_type, standard_field_type, state_storage_index, try_array_element_type, value_type,
};

#[derive(Default)]
pub(super) struct MatchLayout {
    pub values: HashMap<ExprId, u32>,
    pub fallback_values: HashMap<ExprId, u32>,
    pub intrinsic_temps: HashMap<ExprId, Vec<u32>>,
    pub suspension_temps: HashMap<ExprId, u32>,
    pub temporaries: HashMap<TemporaryId, (u32, Type)>,
}

#[derive(Clone, Copy)]
pub(super) enum LocalStorage<'a> {
    Wasm {
        values: &'a HashMap<ValueId, (u32, Type)>,
        temporaries: &'a HashMap<TemporaryId, (u32, Type)>,
    },
    Hybrid {
        frame: AsyncFrameRef,
        wasm_values: &'a HashMap<ValueId, (u32, Type)>,
        frame_values: &'a HashMap<ValueId, (u32, Type)>,
        wasm_temporaries: &'a HashMap<TemporaryId, (u32, Type)>,
        frame_temporaries: &'a HashMap<TemporaryId, (u32, Type)>,
    },
}

impl LocalStorage<'_> {
    pub(super) fn continuation_frame(self) -> Option<AsyncFrameRef> {
        match self {
            Self::Hybrid { frame, .. } => Some(frame),
            Self::Wasm { .. } => None,
        }
    }

    pub(super) fn frame(self) -> AsyncFrameRef {
        self.continuation_frame()
            .expect("direct bodies do not have continuation frames")
    }
}

#[derive(Clone, Copy)]
pub(super) enum BareReturn {
    None,
    Action(ActionKind),
    AsyncAttach {
        result_global: Option<u32>,
    },
    AsyncFuture {
        frame: AsyncFrameRef,
        completion: Option<(u32, Type)>,
    },
}

#[derive(Clone, Copy)]
pub(super) struct ExprContext<'a> {
    pub standard_library: &'a StandardLibrary,
    pub reachability: &'a super::reachability::Reachability,
    pub abi: &'a Abi,
    pub state: &'a crate::ast::StateDecl,
    pub locals: LocalStorage<'a>,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub settings: &'a HashMap<ValueId, SettingStorage>,
    pub runtime_globals: RuntimeGlobals,
    pub runtime_helpers: &'a RuntimeHelperPlan,
    pub functions: &'a HashMap<FunctionInstance, super::function_plan::UserFunctionPlan>,
    pub closures: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub function_values: &'a HashMap<crate::semantic::FunctionValueInstance, u32>,
    pub closure_polls: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub closure_environment: Option<ClosureEnvironment<'a>>,
    pub intrinsic_futures: &'a HashMap<IntrinsicFutureInstance, u32>,
    pub display_functions: &'a DisplayFunctions,
    pub equality_functions: &'a EqualityFunctions,
    pub array_functions: &'a super::ArrayFunctions,
    pub set_functions: &'a SetFunctions,
    pub records: &'a [RecordDecl],
    pub managed: &'a crate::managed::ManagedBindingPlan,
    /// Snapshot-transaction cache. The generated runtime activates it only
    /// while assembling a candidate state, allowing transitively called
    /// helpers to share roots without affecting ordinary lifecycle calls.
    pub managed_state_reads: &'a ManagedStateReadCache,
    pub managed_state_read_functions: &'a HashMap<crate::ast::ManagedFieldId, u32>,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub memory: &'a MemoryLayouts,
    pub abi_read: AbiReadScratch,
    pub signatures: &'a super::data_plan::SignaturePool,
    pub matches: &'a MatchLayout,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
    pub gc: &'a GcLayout,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub intrinsic_capture: Option<IntrinsicCapture<'a>>,
    pub debug: Option<super::debug_artifacts::DebugEmission<'a>>,
    /// Concrete type arguments while emitting a generic function template.
    pub function_instance: Option<&'a FunctionInstance>,
    pub loop_control: Option<LoopControl>,
    pub bare_return: BareReturn,
    /// Whether semantic unit values need a physical operand-stack value.
    /// Discarded expressions and unit-returning ABIs erase `None` entirely.
    pub materialize_none: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ClosureEnvironment<'a> {
    pub local: u32,
    pub struct_type: u32,
    pub captures: &'a HashMap<ValueId, (u32, Type, bool)>,
}

impl ClosureEnvironment<'_> {
    fn emit(self, function: &mut Function) {
        function
            .instruction(&Instruction::LocalGet(self.local))
            .instruction(&Instruction::RefCastNonNull(HeapType::Concrete(
                self.struct_type,
            )));
    }
}

fn emit_capture_cell_get(function: &mut Function, ty: Type, gc: &GcLayout) {
    function.instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, gc.capture_cell_index(ty), 0, ty);
}

#[derive(Clone, Copy)]
pub(super) struct IntrinsicCapture<'a> {
    pub frame: AsyncFrameRef,
    pub layout: &'a IntrinsicFutureLayout,
}

impl ExprContext<'_> {
    pub(super) fn erasing_none(&self) -> Self {
        let mut context = *self;
        context.materialize_none = false;
        context
    }

    pub(super) fn type_id(&self, ty: TypeId) -> TypeId {
        self.function_instance
            .map_or(ty, |instance| self.semantics.specialize_type(instance, ty))
    }

    pub(super) fn ty(&self, ty: TypeId) -> Type {
        semantic_type(self.type_id(ty), self.semantics)
    }

    pub(super) fn expression_type(&self, expression: ExprId) -> Type {
        self.ty(self
            .wasm_ir
            .expression(expression)
            .expect("typed expressions belong to Wasm IR")
            .ty)
    }

    pub(super) fn expression_type_id(&self, expression: ExprId) -> TypeId {
        self.type_id(
            self.wasm_ir
                .expression(expression)
                .expect("typed expressions belong to Wasm IR")
                .ty,
        )
    }

    fn called_instance(&self, function: &FunctionInstance) -> FunctionInstance {
        self.function_instance.map_or_else(
            || function.clone(),
            |owner| self.semantics.specialize_function_instance(owner, function),
        )
    }

    fn nested_loop_control(&self, depth: u32) -> Self {
        let mut nested = *self;
        nested.loop_control = nested.loop_control.map(|control| control.nested(depth));
        nested
    }
}

pub(super) fn compile_block(
    function: &mut Function,
    block: &wasm_ir::Block,
    context: &ExprContext<'_>,
    action: Option<ActionKind>,
) {
    compile_block_with_loop(function, block, context, action, None);
}

#[derive(Clone, Copy)]
pub(super) enum LoopControl {
    Branch {
        break_depth: u32,
        continue_depth: u32,
        break_destination: Option<wasm_ir::TemporaryId>,
    },
    Async {
        break_state: wasm_ir::AsyncStateId,
        continue_state: wasm_ir::AsyncStateId,
        dispatcher_depth: u32,
        break_destination: Option<wasm_ir::TemporaryId>,
    },
}

impl LoopControl {
    pub(super) fn nested(self, depth: u32) -> Self {
        match self {
            Self::Branch {
                break_depth,
                continue_depth,
                break_destination,
            } => Self::Branch {
                break_depth: break_depth + depth,
                continue_depth: continue_depth + depth,
                break_destination,
            },
            Self::Async {
                break_state,
                continue_state,
                dispatcher_depth,
                break_destination,
            } => Self::Async {
                break_state,
                continue_state,
                dispatcher_depth: dispatcher_depth + depth,
                break_destination,
            },
        }
    }

    pub(super) fn break_destination(self) -> Option<wasm_ir::TemporaryId> {
        match self {
            Self::Branch {
                break_destination, ..
            }
            | Self::Async {
                break_destination, ..
            } => break_destination,
        }
    }

    pub(super) fn emit_break(self, function: &mut Function, frame: Option<AsyncFrameRef>) {
        self.emit(function, true, frame);
    }

    pub(super) fn emit_continue(self, function: &mut Function, frame: Option<AsyncFrameRef>) {
        self.emit(function, false, frame);
    }

    fn emit(self, function: &mut Function, is_break: bool, frame: Option<AsyncFrameRef>) {
        match self {
            Self::Branch {
                break_depth,
                continue_depth,
                ..
            } => {
                function.instruction(&Instruction::Br(if is_break {
                    break_depth
                } else {
                    continue_depth
                }));
            }
            Self::Async {
                break_state,
                continue_state,
                dispatcher_depth,
                ..
            } => {
                let frame = frame.expect("async loop control has a continuation frame");
                let state = if is_break {
                    break_state
                } else {
                    continue_state
                };
                frame.emit(function);
                function
                    .instruction(&Instruction::I32Const(state.index() as i32))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: frame.struct_type,
                        field_index: 0,
                    })
                    .instruction(&Instruction::Br(dispatcher_depth));
            }
        }
    }
}

fn compile_block_with_loop(
    function: &mut Function,
    block: &wasm_ir::Block,
    context: &ExprContext<'_>,
    action: Option<ActionKind>,
    loop_control: Option<LoopControl>,
) {
    let expression_context = ExprContext {
        loop_control,
        ..*context
    };
    let context = &expression_context;
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::DebugLocation(source) => {
                if let Some(debug) = context.debug {
                    debug.mark(function, Some(*source));
                }
            }
            wasm_ir::Statement::Store {
                target,
                declaration,
                operation,
                value,
            } => {
                compile_assignment(
                    function,
                    *target,
                    *declaration,
                    operation.as_ref(),
                    *value,
                    context,
                );
            }
            wasm_ir::Statement::StateStore {
                target,
                operation,
                value,
            } => {
                compile_state_assignment(function, *target, operation.as_ref(), *value, context);
            }
            wasm_ir::Statement::StoreTemporary { target, value } => {
                compile_temporary_set(function, *target, *value, context);
            }
            wasm_ir::Statement::IndexStore {
                target,
                operation,
                value,
            } => compile_index_assignment(function, *target, operation, *value, context),
            wasm_ir::Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                compile_expr(function, *condition, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_block_with_loop(
                    function,
                    then_block,
                    context,
                    action,
                    loop_control.map(|control| control.nested(1)),
                );
                function.instruction(&Instruction::Else);
                compile_block_with_loop(
                    function,
                    else_block,
                    context,
                    action,
                    loop_control.map(|control| control.nested(1)),
                );
                function.instruction(&Instruction::End);
            }
            wasm_ir::Statement::Match {
                expression,
                value,
                arms,
            } => compile_match_statement(
                function,
                *expression,
                *value,
                arms,
                context,
                action,
                loop_control,
            ),
            wasm_ir::Statement::Fallback {
                expression,
                value,
                fallback_block,
                success_block,
            } => {
                compile_fallback_condition(function, *expression, *value, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_block_with_loop(
                    function,
                    fallback_block,
                    context,
                    action,
                    loop_control.map(|control| control.nested(1)),
                );
                function.instruction(&Instruction::Else);
                compile_block_with_loop(
                    function,
                    success_block,
                    context,
                    action,
                    loop_control.map(|control| control.nested(1)),
                );
                function.instruction(&Instruction::End);
            }
            wasm_ir::Statement::While {
                condition,
                body,
                result,
            } => {
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_expr(function, *condition, context);
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_block_with_loop(
                    function,
                    body,
                    context,
                    action,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: *result,
                    }),
                );
                function
                    .instruction(&Instruction::Br(0))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::For {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step,
                body,
            } => {
                compile_for_init(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterable,
                    context,
                );
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_for_has_next(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterator_step,
                    context,
                );
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_for_bind_and_advance(
                    function,
                    *binding,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    context,
                );
                compile_block_with_loop(
                    function,
                    body,
                    context,
                    action,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: None,
                    }),
                );
                function
                    .instruction(&Instruction::Br(0))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::ForInit {
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step: _,
                ..
            } => compile_for_init(
                function,
                *iterable_value,
                *index_value,
                *version_value,
                *iterable,
                context,
            ),
            wasm_ir::Statement::Evaluate {
                expression,
                discard_result,
            } => {
                let ty = context.expression_type(*expression);
                // Erase a discarded unit result, but do not erase values nested
                // inside a `Never` expression. A transfer such as
                // `return Some(None)` has no local result while its operand
                // still needs the physical `None` payload required by `Some`.
                let expression_context = if *discard_result && ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, *expression, &expression_context);
                if *discard_result && ty.has_runtime_value() {
                    function.instruction(&Instruction::Drop);
                }
            }
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Fallthrough => {}
        wasm_ir::Terminator::Break(value) => {
            let control = loop_control.expect("checked break expressions belong to loops");
            if let Some(value) = value {
                if let Some(destination) = control.break_destination() {
                    compile_temporary_set(function, destination, *value, context);
                } else {
                    compile_expr(function, *value, &context.erasing_none());
                    if context.expression_type(*value).has_runtime_value() {
                        function.instruction(&Instruction::Drop);
                    }
                }
            }
            control.emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue expressions belong to loops")
                .emit_continue(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::AsyncWhile { .. }
        | wasm_ir::Terminator::AsyncWhileCondition { .. }
        | wasm_ir::Terminator::AsyncFor { .. } => {
            unreachable!("async loops are lowered by the async action compiler")
        }
        wasm_ir::Terminator::Return(value) => {
            if let Some(value) = value {
                let return_context = if matches!(context.bare_return, BareReturn::None)
                    && context.expression_type(*value) == Type::None
                {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, *value, &return_context);
            } else if let Some(action) = action {
                emit_action_default(function, action, context.gc);
            }
            function.instruction(&Instruction::Return);
        }
        wasm_ir::Terminator::Throw { error, target } => match target {
            crate::hir::FailureTarget::Return(target) => {
                let Type::Result(target_result) = context.ty(*target) else {
                    unreachable!("throw targets are result values")
                };
                emit_failure_transfer(
                    function,
                    target_result,
                    result_value_type(target_result, context.semantics),
                    context.gc,
                    |function| compile_expr(function, *error, context),
                );
            }
            crate::hir::FailureTarget::Retry { .. } => {
                compile_expr(function, *error, context);
                function
                    .instruction(&Instruction::Drop)
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            }
        },
        wasm_ir::Terminator::Retry { .. }
        | wasm_ir::Terminator::RetryComplete { .. }
        | wasm_ir::Terminator::Suspend { .. } => {
            unreachable!("suspension is lowered by the async action compiler")
        }
    }
}

pub(super) fn compile_fallback_condition(
    function: &mut Function,
    expression: ExprId,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    let input_local = context.matches.fallback_values[&expression];
    let input_type = context.expression_type(value);
    compile_expr(function, value, context);
    function.instruction(&Instruction::LocalSet(input_local));
    match input_type {
        Type::Option(_) => {
            function
                .instruction(&Instruction::LocalGet(input_local))
                .instruction(&Instruction::RefIsNull);
        }
        Type::Result(result) => {
            function
                .instruction(&Instruction::LocalGet(input_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(
                function,
                context.gc.index(Type::Result(result)),
                1,
                Type::I32,
            );
        }
        _ => unreachable!("typed fallback inputs are optional or result values"),
    }
}

fn compile_fallback_success(
    function: &mut Function,
    source: ExprId,
    ty: Type,
    context: &ExprContext<'_>,
) {
    if ty == Type::None && !context.materialize_none {
        return;
    }
    let source = context
        .wasm_ir
        .expression(source)
        .expect("fallback success source belongs to Wasm IR");
    let wasm_ir::ExpressionKind::Fallback { value, .. } = source.kind else {
        unreachable!("fallback success references a fallback expression")
    };
    let input_local = context.matches.fallback_values[&source.id];
    match context.expression_type(value) {
        Type::Option(option) => {
            function
                .instruction(&Instruction::LocalGet(input_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, context.gc.index(Type::Option(option)), 0, ty);
        }
        Type::Result(result) => {
            function
                .instruction(&Instruction::LocalGet(input_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, context.gc.index(Type::Result(result)), 0, ty);
        }
        _ => unreachable!("typed fallback inputs are optional or result values"),
    }
}

#[derive(Clone, Copy)]
pub(super) struct MatchPatternBinding {
    value: ValueId,
    payload_type: Type,
    struct_type: u32,
    field_index: u32,
}

fn compile_file_version_pattern(
    function: &mut Function,
    components: &[u16; 4],
    value_local: u32,
    value_type: Type,
    context: &ExprContext<'_>,
) {
    // Keep the literal syntax coupled to the catalog's named components, not
    // to the physical field order of FileVersion's GC representation.
    let fields = [
        StdlibFieldId::FileVersionMajor,
        StdlibFieldId::FileVersionMinor,
        StdlibFieldId::FileVersionBuild,
        StdlibFieldId::FileVersionPrivatePart,
    ];
    for (component_index, (field, component)) in fields.iter().zip(components).enumerate() {
        function
            .instruction(&Instruction::LocalGet(value_local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            function,
            context.gc.index(value_type),
            context.gc.standard_field_index(*field),
            Type::U16,
        );
        function
            .instruction(&Instruction::I32Const(i32::from(*component)))
            .instruction(&Instruction::I32Eq);
        if component_index != 0 {
            function.instruction(&Instruction::I32And);
        }
    }
}

pub(super) fn compile_statement_pattern(
    function: &mut Function,
    pattern: &wasm_ir::LoweredPattern,
    value_local: u32,
    value_type: Type,
    context: &ExprContext<'_>,
) -> Option<MatchPatternBinding> {
    let enum_pattern = if let wasm_ir::LoweredPattern::Enum {
        enumeration,
        variant,
        binding,
    } = pattern
    {
        let variant_index = context
            .gc
            .enum_variant_index(*enumeration, *variant, context.enums);
        Some((*enumeration, variant_index, *binding))
    } else {
        None
    };
    let binding = match pattern {
        wasm_ir::LoweredPattern::Enum { .. } => {
            let (_, variant_index, binding) = enum_pattern.unwrap();
            binding.map(|binding| {
                (
                    binding,
                    context.gc.index(value_type),
                    variant_index as u32 + 1,
                )
            })
        }
        wasm_ir::LoweredPattern::OptionSome { binding, .. } => {
            let Type::Option(option) = value_type else {
                unreachable!("Some patterns match Option values")
            };
            binding.map(|binding| (binding, context.gc.index(Type::Option(option)), 0))
        }
        wasm_ir::LoweredPattern::IteratorItem { binding, .. } => {
            let Type::Application(step) = value_type else {
                unreachable!("Item patterns match IteratorStep values")
            };
            binding.map(|binding| (binding, context.gc.index(Type::Application(step)), 0))
        }
        wasm_ir::LoweredPattern::ResultSuccess { binding, .. } => {
            let Type::Result(result) = value_type else {
                unreachable!("Ok patterns match Result values")
            };
            binding.map(|binding| (binding, context.gc.index(Type::Result(result)), 0))
        }
        wasm_ir::LoweredPattern::ResultError { binding, .. } => {
            let Type::Result(result) = value_type else {
                unreachable!("Err patterns match Result values")
            };
            binding.map(|binding| (binding, context.gc.index(Type::Result(result)), 2))
        }
        _ => None,
    };
    match pattern {
        wasm_ir::LoweredPattern::Enum { .. } => {
            let (_, variant_index, _) = enum_pattern.unwrap();
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, context.gc.index(value_type), 0, Type::I32);
            function
                .instruction(&Instruction::I32Const(variant_index as i32))
                .instruction(&Instruction::I32Eq);
        }
        wasm_ir::LoweredPattern::Bool(expected) => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::I32Const(*expected as i32))
                .instruction(&Instruction::I32Eq);
        }
        wasm_ir::LoweredPattern::Char(expected) => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::I32Const(*expected as i32))
                .instruction(&Instruction::I32Eq);
        }
        wasm_ir::LoweredPattern::String(expected) => {
            function.instruction(&Instruction::LocalGet(value_local));
            emit_string_literal(function, expected, context.gc);
            function.instruction(&Instruction::Call(
                context
                    .runtime_helpers
                    .function(RuntimeHelperId::StringEquality),
            ));
        }
        wasm_ir::LoweredPattern::Int(value) => {
            function.instruction(&Instruction::LocalGet(value_local));
            emit_int(function, *value, value_type);
            function.instruction(
                &if matches!(value_type, Type::I64 | Type::U64 | Type::Address) {
                    Instruction::I64Eq
                } else {
                    Instruction::I32Eq
                },
            );
        }
        wasm_ir::LoweredPattern::FileVersion(components) => {
            compile_file_version_pattern(function, components, value_local, value_type, context);
        }
        wasm_ir::LoweredPattern::OptionNone(_) => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefIsNull);
        }
        wasm_ir::LoweredPattern::OptionSome { .. } => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::I32Eqz);
        }
        wasm_ir::LoweredPattern::IteratorEnd(_) => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefIsNull);
        }
        wasm_ir::LoweredPattern::IteratorItem { .. } => {
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::I32Eqz);
        }
        wasm_ir::LoweredPattern::ResultSuccess { .. }
        | wasm_ir::LoweredPattern::ResultError { .. } => {
            let Type::Result(result) = value_type else {
                unreachable!("result patterns match Result values")
            };
            function
                .instruction(&Instruction::LocalGet(value_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(
                function,
                context.gc.index(Type::Result(result)),
                1,
                Type::I32,
            );
            function.instruction(&Instruction::I32Const(matches!(
                pattern,
                wasm_ir::LoweredPattern::ResultError { .. }
            ) as i32));
            function.instruction(&Instruction::I32Eq);
        }
        wasm_ir::LoweredPattern::Wildcard => {
            function.instruction(&Instruction::I32Const(1));
        }
    }
    binding.map(|(value, struct_type, field_index)| MatchPatternBinding {
        value,
        payload_type: context.ty(context
            .semantics
            .value_type(value)
            .expect("checked pattern bindings have types")),
        struct_type,
        field_index,
    })
}

pub(super) fn store_match_binding(
    function: &mut Function,
    binding: MatchPatternBinding,
    value_local: u32,
    context: &ExprContext<'_>,
) {
    compile_value_set(function, binding.value, context, |function| {
        function
            .instruction(&Instruction::LocalGet(value_local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            function,
            binding.struct_type,
            binding.field_index,
            binding.payload_type,
        );
    });
}

fn compile_match_statement(
    function: &mut Function,
    expression: ExprId,
    value: ExprId,
    arms: &[wasm_ir::MatchStatementArm],
    context: &ExprContext<'_>,
    action: Option<ActionKind>,
    loop_control: Option<LoopControl>,
) {
    let value_local = context.matches.values[&expression];
    let value_type = context.expression_type(value);
    compile_expr(function, value, context);
    function.instruction(&Instruction::LocalSet(value_local));
    for (arm_index, arm) in arms.iter().enumerate() {
        let binding =
            compile_statement_pattern(function, &arm.pattern, value_local, value_type, context);
        let arm_context = ExprContext {
            loop_control: loop_control.map(|control| control.nested(arm_index as u32 + 1)),
            ..*context
        };
        if binding.is_some() || arm.guard.is_some() {
            function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            if let Some(binding) = binding {
                store_match_binding(function, binding, value_local, &arm_context);
            }
            if let Some(guard) = arm.guard {
                compile_expr(function, guard, &arm_context);
            } else {
                function.instruction(&Instruction::I32Const(1));
            }
            function
                .instruction(&Instruction::Else)
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::End);
        }
        function.instruction(&Instruction::If(BlockType::Empty));
        compile_block_with_loop(
            function,
            &arm.block,
            &arm_context,
            action,
            arm_context.loop_control,
        );
        function.instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::Unreachable);
    for _ in arms {
        function.instruction(&Instruction::End);
    }
}

pub(super) fn compile_assignment(
    function: &mut Function,
    target: ValueId,
    declaration: bool,
    operation: Option<&wasm_ir::AssignmentOperation>,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    if let Some(environment) = context.closure_environment
        && let Some((field, ty, mutable)) = environment.captures.get(&target).copied()
    {
        if !ty.has_runtime_value() {
            compile_expr(function, value, &context.erasing_none());
            return;
        }
        debug_assert!(mutable, "immutable captures cannot be assignment targets");
        environment.emit(function);
        function.instruction(&Instruction::StructGet {
            struct_type_index: environment.struct_type,
            field_index: field,
        });
        function.instruction(&Instruction::RefAsNonNull);
        compile_assignment_value(function, operation, value, ty, context, |function| {
            environment.emit(function);
            function.instruction(&Instruction::StructGet {
                struct_type_index: environment.struct_type,
                field_index: field,
            });
            emit_capture_cell_get(function, ty, context.gc);
        });
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.gc.capture_cell_index(ty),
            field_index: 0,
        });
        return;
    }
    if context.wasm_ir.is_mutably_captured(target) {
        let ty = stored_value_type(target, context);
        if !ty.has_runtime_value() {
            compile_expr(function, value, &context.erasing_none());
            return;
        }
        if declaration {
            compile_raw_value_set(function, target, context, |function| {
                compile_expr(function, value, context);
                function.instruction(&Instruction::StructNew(context.gc.capture_cell_index(ty)));
            });
            return;
        }
        emit_raw_value_get(function, target, context);
        function.instruction(&Instruction::RefAsNonNull);
        compile_assignment_value(function, operation, value, ty, context, |function| {
            emit_raw_value_get(function, target, context);
            emit_capture_cell_get(function, ty, context.gc);
        });
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.gc.capture_cell_index(ty),
            field_index: 0,
        });
        return;
    }
    match context.locals {
        LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&target) => {
            let (field, ty) = frame_values[&target];
            if !ty.has_runtime_value() {
                let expression_context = if ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, value, &expression_context);
                return;
            }
            let frame = context.locals.frame();
            frame.emit(function);
            compile_assignment_value(function, operation, value, ty, context, |function| {
                frame.emit(function);
                emit_typed_struct_get(function, frame.struct_type, field, ty);
            });
            function.instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: field,
            });
        }
        LocalStorage::Hybrid { wasm_values, .. } if wasm_values.contains_key(&target) => {
            let (local, ty) = wasm_values[&target];
            if !ty.has_runtime_value() {
                let expression_context = if ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, value, &expression_context);
                return;
            }
            compile_assignment_value(function, operation, value, ty, context, |function| {
                function.instruction(&Instruction::LocalGet(local));
            });
            function.instruction(&Instruction::LocalSet(local));
        }
        LocalStorage::Wasm { values, .. } if values.contains_key(&target) => {
            let (local, ty) = values[&target];
            if !ty.has_runtime_value() {
                let expression_context = if ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, value, &expression_context);
                return;
            }
            compile_assignment_value(function, operation, value, ty, context, |function| {
                function.instruction(&Instruction::LocalGet(local));
            });
            function.instruction(&Instruction::LocalSet(local));
        }
        _ => {
            let ty = context.global_types[&target];
            if !ty.has_runtime_value() {
                let expression_context = if ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, value, &expression_context);
                return;
            }
            let global = context.globals[&target];
            compile_assignment_value(function, operation, value, ty, context, |function| {
                function.instruction(&Instruction::GlobalGet(global));
            });
            function.instruction(&Instruction::GlobalSet(global));
        }
    }
}

pub(super) fn compile_state_assignment(
    function: &mut Function,
    target: ValueId,
    operation: Option<&wasm_ir::AssignmentOperation>,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    let (field_index, storage) = state_storage_index(target, context.semantics);
    let ty = value_type(storage, context.semantics);
    function
        .instruction(&Instruction::GlobalGet(context.runtime_globals.current))
        .instruction(&Instruction::RefAsNonNull);
    compile_assignment_value(function, operation, value, ty, context, |function| {
        function
            .instruction(&Instruction::GlobalGet(context.runtime_globals.current))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(function, STATE_TYPE, field_index, ty);
    });
    function.instruction(&Instruction::StructSet {
        struct_type_index: STATE_TYPE,
        field_index,
    });
}

pub(super) fn compile_index_assignment(
    function: &mut Function,
    target: ExprId,
    operation: &wasm_ir::AssignmentOperation,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    let wasm_ir::ExpressionKind::Index { receiver, index } = &context
        .wasm_ir
        .expression(target)
        .expect("indexed assignment target belongs to Wasm IR")
        .kind
    else {
        unreachable!("indexed assignment lowering produces an index target")
    };
    let Type::Array(array_id) = context.expression_type(*receiver) else {
        unreachable!("checked indexed assignment receivers are arrays")
    };
    compile_expr(function, *receiver, context);
    super::array_value::emit_backing(function, context.gc, array_id);
    compile_expr(function, *index, context);
    let element = array_element_type(array_id, context.semantics);
    compile_assignment_value(
        function,
        Some(operation),
        value,
        element,
        context,
        |function| compile_expr(function, target, context),
    );
    function.instruction(&Instruction::ArraySet(context.gc.index(
        Type::ArrayStorage(super::array_value::storage_id(
            array_id,
            context.arrays,
            context.semantics,
        )),
    )));
}

pub(super) fn compile_temporary_set(
    function: &mut Function,
    target: TemporaryId,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    match context.locals {
        LocalStorage::Hybrid {
            frame_temporaries, ..
        } if frame_temporaries.contains_key(&target) => {
            let (field, ty) = frame_temporaries[&target];
            if !ty.has_runtime_value() {
                if ty == Type::None {
                    let erased = context.erasing_none();
                    compile_expr(function, value, &erased);
                } else {
                    compile_expr(function, value, context);
                }
                return;
            }
            let frame = context.locals.frame();
            frame.emit(function);
            compile_expr(function, value, context);
            function.instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: field,
            });
        }
        LocalStorage::Hybrid {
            wasm_temporaries, ..
        } if wasm_temporaries.contains_key(&target) => {
            let ty = wasm_temporaries[&target].1;
            if !ty.has_runtime_value() {
                if ty == Type::None {
                    let erased = context.erasing_none();
                    compile_expr(function, value, &erased);
                } else {
                    compile_expr(function, value, context);
                }
                return;
            }
            compile_expr(function, value, context);
            debug_assert_ne!(wasm_temporaries[&target].0, u32::MAX);
            function.instruction(&Instruction::LocalSet(wasm_temporaries[&target].0));
        }
        LocalStorage::Wasm { temporaries, .. } => {
            let ty = temporaries[&target].1;
            if !ty.has_runtime_value() {
                if ty == Type::None {
                    let erased = context.erasing_none();
                    compile_expr(function, value, &erased);
                } else {
                    compile_expr(function, value, context);
                }
                return;
            }
            compile_expr(function, value, context);
            function.instruction(&Instruction::LocalSet(temporaries[&target].0));
        }
        LocalStorage::Hybrid { .. } => {
            unreachable!("planned compiler temporary has storage")
        }
    }
}

fn compile_temporary_get(
    function: &mut Function,
    temporary: TemporaryId,
    context: &ExprContext<'_>,
) {
    match context.locals {
        LocalStorage::Hybrid {
            frame_temporaries, ..
        } if frame_temporaries.contains_key(&temporary) => {
            let (field, ty) = frame_temporaries[&temporary];
            if !ty.has_runtime_value() {
                if ty == Type::None && context.materialize_none {
                    emit_default(function, Type::None, context.gc);
                }
                return;
            }
            let frame = context.locals.frame();
            frame.emit(function);
            emit_typed_struct_get(function, frame.struct_type, field, ty);
        }
        LocalStorage::Hybrid {
            wasm_temporaries, ..
        } => {
            let (local, ty) = wasm_temporaries[&temporary];
            if ty.has_runtime_value() {
                function.instruction(&Instruction::LocalGet(local));
            } else if ty == Type::None && context.materialize_none {
                emit_default(function, Type::None, context.gc);
            }
        }
        LocalStorage::Wasm { temporaries, .. } => {
            let (local, ty) = temporaries[&temporary];
            if ty.has_runtime_value() {
                function.instruction(&Instruction::LocalGet(local));
            } else if ty == Type::None && context.materialize_none {
                emit_default(function, Type::None, context.gc);
            }
        }
    };
}

fn compile_assignment_value(
    function: &mut Function,
    operation: Option<&wasm_ir::AssignmentOperation>,
    value: ExprId,
    ty: Type,
    context: &ExprContext<'_>,
    emit_current: impl FnOnce(&mut Function),
) {
    match operation {
        Some(wasm_ir::AssignmentOperation::Primitive(op)) => {
            emit_current(function);
            compile_expr(function, value, context);
            emit_binary_instruction(function, *op, ty);
            emit_narrow_integer_result(function, ty);
        }
        Some(wasm_ir::AssignmentOperation::Call(target)) => {
            compile_assignment_call(function, target, value, context)
        }
        None => compile_expr(function, value, context),
    }
}

fn compile_assignment_call(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    argument: ExprId,
    context: &ExprContext<'_>,
) {
    match target {
        wasm_ir::CallTarget::UserMethod {
            function: target_function,
            ..
        } => {
            compile_receiver(function, target, context);
            compile_expr(function, argument, context);
            let target_function = context.called_instance(target_function);
            function.instruction(&Instruction::Call(context.functions[&target_function].call));
        }
        wasm_ir::CallTarget::Intrinsic { intrinsic, .. }
            if intrinsic_binary_op(*intrinsic).is_some() =>
        {
            let receiver = compile_receiver(function, target, context);
            compile_expr(function, argument, context);
            emit_binary_instruction(
                function,
                intrinsic_binary_op(*intrinsic).expect("guarded primitive binary intrinsic"),
                receiver,
            );
            emit_narrow_integer_result(function, receiver);
        }
        _ => unreachable!("validated compound assignments use binary method call targets"),
    }
}

fn resolved_receiver<'a>(
    target: &'a wasm_ir::CallTarget,
    context: &ExprContext<'_>,
) -> (&'a ResolvedReceiver, Type) {
    let (receiver, receiver_type) = match target {
        wasm_ir::CallTarget::UserMethod {
            receiver,
            receiver_type,
            ..
        } => (receiver, *receiver_type),
        wasm_ir::CallTarget::Intrinsic {
            receiver: Some(receiver),
            receiver_type: Some(receiver_type),
            ..
        } => (receiver, *receiver_type),
        wasm_ir::CallTarget::LibraryOverload {
            receiver: Some(receiver),
            receiver_type: Some(receiver_type),
            ..
        } => (receiver, *receiver_type),
        wasm_ir::CallTarget::DefaultDisplay {
            receiver,
            receiver_type,
        } => (receiver, *receiver_type),
        _ => unreachable!("only method calls have receivers"),
    };
    (receiver, context.ty(receiver_type))
}

pub(super) fn compile_receiver(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    context: &ExprContext<'_>,
) -> Type {
    let (receiver, receiver_type) = resolved_receiver(target, context);
    if let Some(capture) = context.intrinsic_capture
        && let Some((field, captured_type)) = capture.layout.receiver
    {
        debug_assert_eq!(captured_type, receiver_type);
        capture.frame.emit(function);
        emit_typed_struct_get(function, capture.frame.struct_type, field, captured_type);
        return receiver_type;
    }
    match receiver {
        ResolvedReceiver::Path { root, members } => {
            let lowered_type = compile_resolved_path(function, *root, members, context);
            debug_assert_eq!(lowered_type, receiver_type);
        }
        ResolvedReceiver::Expression {
            expression,
            members,
        } => {
            compile_expr(function, *expression, context);
            let base_type = context.expression_type(*expression);
            let lowered_type = emit_path_fields(function, members, base_type, context);
            debug_assert_eq!(lowered_type, receiver_type);
        }
    }
    receiver_type
}

fn compile_resolved_path(
    function: &mut Function,
    value: ResolvedValue,
    members: &[ResolvedMember],
    context: &ExprContext<'_>,
) -> Type {
    if let [ResolvedMember::ManagedField(field)] = members
        && resolved_value_type_id(value, context).is_some_and(|ty| {
            matches!(
                context.semantics.types().kind(ty),
                crate::types::TypeKind::ManagedReference(_)
            )
        })
    {
        function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
        let receiver = compile_resolved_path(function, value, &[], context);
        debug_assert_eq!(receiver, Type::Address);
        return emit_managed_field_read(function, *field, context);
    }
    let value_type = match value {
        ResolvedValue::StandardLibraryConstant(item) => {
            let function_instance = context
                .wasm_ir
                .constant_function(item)
                .expect("source-defined constants have hidden function bodies");
            let function_instance = context.called_instance(function_instance);
            function.instruction(&Instruction::Call(
                context.functions[&function_instance].call,
            ));
            let result = function_instance
                .signature
                .last()
                .copied()
                .or_else(|| {
                    context
                        .semantics
                        .function_result(function_instance.function)
                })
                .expect("constant function instances have a result type");
            semantic_type(result, context.semantics)
        }
        ResolvedValue::ProviderValue(provider) => {
            let declaration = context.wasm_ir.standard_library().state_provider(provider);
            if declaration.attachment == crate::stdlib::StateProviderAttachment::Identity {
                function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            } else {
                function.instruction(&Instruction::GlobalGet(
                    context
                        .runtime_globals
                        .provider_value
                        .expect("provider value references require provider storage"),
                ));
            }
            Type::Standard(declaration.process_type)
        }
        ResolvedValue::ManagedStatic { class, field } => {
            emit_managed_static_read(function, class, field, context)
        }
        ResolvedValue::CurrentSnapshot | ResolvedValue::OldSnapshot => {
            function
                .instruction(&Instruction::GlobalGet(
                    if matches!(value, ResolvedValue::OldSnapshot) {
                        context.runtime_globals.old
                    } else {
                        context.runtime_globals.current
                    },
                ))
                .instruction(&Instruction::RefAsNonNull);
            Type::StateSnapshot
        }
        ResolvedValue::SettingsView | ResolvedValue::OldSettingsView => {
            function.instruction(&Instruction::I32Const(i32::from(matches!(
                value,
                ResolvedValue::OldSettingsView
            ))));
            Type::SettingsView
        }
        ResolvedValue::CurrentState(field) | ResolvedValue::OldState(field) => {
            function
                .instruction(&Instruction::GlobalGet(
                    if matches!(value, ResolvedValue::OldState(_)) {
                        context.runtime_globals.old
                    } else {
                        context.runtime_globals.current
                    },
                ))
                .instruction(&Instruction::RefAsNonNull);
            let (index, storage) = state_storage_index(field, context.semantics);
            let field_type = value_type(storage, context.semantics);
            emit_struct_get(function, index, field_type);
            field_type
        }
        ResolvedValue::Setting(setting) | ResolvedValue::OldSetting(setting) => {
            let storage = context.settings[&setting];
            let current = matches!(value, ResolvedValue::Setting(_));
            function.instruction(&Instruction::GlobalGet(if current {
                storage.current
            } else {
                storage.old
            }));
            storage.ty
        }
        ResolvedValue::Variable(value) => {
            if let Some(environment) = context.closure_environment
                && let Some((field, ty, mutable)) = environment.captures.get(&value).copied()
            {
                if ty.has_runtime_value() {
                    environment.emit(function);
                    if mutable {
                        function.instruction(&Instruction::StructGet {
                            struct_type_index: environment.struct_type,
                            field_index: field,
                        });
                        emit_capture_cell_get(function, ty, context.gc);
                    } else {
                        emit_typed_struct_get(function, environment.struct_type, field, ty);
                    }
                } else if ty == Type::None && context.materialize_none {
                    emit_default(function, Type::None, context.gc);
                }
                ty
            } else {
                match context.locals {
                    LocalStorage::Hybrid { frame_values, .. }
                        if frame_values.contains_key(&value) =>
                    {
                        let (field, ty) = frame_values[&value];
                        if !ty.has_runtime_value() {
                            if context.materialize_none && ty == Type::None {
                                emit_default(function, Type::None, context.gc);
                            }
                        } else {
                            let frame = context.locals.frame();
                            frame.emit(function);
                            if context.wasm_ir.is_mutably_captured(value) {
                                function.instruction(&Instruction::StructGet {
                                    struct_type_index: frame.struct_type,
                                    field_index: field,
                                });
                                emit_capture_cell_get(function, ty, context.gc);
                            } else {
                                emit_typed_struct_get(function, frame.struct_type, field, ty);
                            }
                        }
                        ty
                    }
                    LocalStorage::Hybrid { wasm_values, .. }
                        if wasm_values.contains_key(&value) =>
                    {
                        let (local, ty) = wasm_values[&value];
                        if !ty.has_runtime_value() {
                            if context.materialize_none && ty == Type::None {
                                emit_default(function, Type::None, context.gc);
                            }
                        } else {
                            function.instruction(&Instruction::LocalGet(local));
                            if context.wasm_ir.is_mutably_captured(value) {
                                emit_capture_cell_get(function, ty, context.gc);
                            }
                        }
                        ty
                    }
                    LocalStorage::Wasm { values, .. } if values.contains_key(&value) => {
                        let (local, ty) = values[&value];
                        if !ty.has_runtime_value() {
                            if context.materialize_none && ty == Type::None {
                                emit_default(function, Type::None, context.gc);
                            }
                        } else {
                            function.instruction(&Instruction::LocalGet(local));
                            if context.wasm_ir.is_mutably_captured(value) {
                                emit_capture_cell_get(function, ty, context.gc);
                            }
                        }
                        ty
                    }
                    _ => {
                        let ty = context.global_types[&value];
                        if ty.has_runtime_value() {
                            function.instruction(&Instruction::GlobalGet(context.globals[&value]));
                            if context.wasm_ir.is_scoped_global(value)
                                && matches!(context.gc.val_type(ty), ValType::Ref(reference) if !reference.nullable)
                            {
                                // Definite-initialization and layout analysis prove
                                // this storage is populated in every source context
                                // that can read it. Re-establish the non-null source
                                // type after loading the nullable lifetime sentinel.
                                function.instruction(&Instruction::RefAsNonNull);
                            }
                        } else if ty == Type::None && context.materialize_none {
                            emit_default(function, Type::None, context.gc);
                        }
                        ty
                    }
                }
            }
        }
    };
    emit_path_fields(function, members, value_type, context)
}

pub(super) fn compile_value_get(
    function: &mut Function,
    value: ValueId,
    context: &ExprContext<'_>,
) -> Type {
    compile_resolved_path(function, ResolvedValue::Variable(value), &[], context)
}

fn stored_value_type(value: ValueId, context: &ExprContext<'_>) -> Type {
    if let Some(environment) = context.closure_environment
        && let Some((_, ty, _)) = environment.captures.get(&value).copied()
    {
        return ty;
    }
    match context.locals {
        LocalStorage::Hybrid {
            frame_values,
            wasm_values,
            ..
        } => frame_values
            .get(&value)
            .or_else(|| wasm_values.get(&value))
            .map(|(_, ty)| *ty),
        LocalStorage::Wasm { values, .. } => values.get(&value).map(|(_, ty)| *ty),
    }
    .or_else(|| context.global_types.get(&value).copied())
    .unwrap_or_else(|| {
        let source = context
            .semantics
            .value_type(value)
            .expect("checked values have semantic types");
        semantic_type(
            context.function_instance.map_or(source, |instance| {
                context.semantics.specialize_type(instance, source)
            }),
            context.semantics,
        )
    })
}

/// Loads the physical slot of a mutably captured value without dereferencing
/// its cell. Closure construction uses this to share the existing cell.
fn emit_raw_value_get(function: &mut Function, value: ValueId, context: &ExprContext<'_>) -> Type {
    if let Some(environment) = context.closure_environment
        && let Some((field, ty, mutable)) = environment.captures.get(&value).copied()
    {
        debug_assert!(mutable);
        environment.emit(function);
        function.instruction(&Instruction::StructGet {
            struct_type_index: environment.struct_type,
            field_index: field,
        });
        return ty;
    }
    match context.locals {
        LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&value) => {
            let (field, ty) = frame_values[&value];
            context.locals.frame().emit(function);
            function.instruction(&Instruction::StructGet {
                struct_type_index: context.locals.frame().struct_type,
                field_index: field,
            });
            ty
        }
        LocalStorage::Hybrid { wasm_values, .. } if wasm_values.contains_key(&value) => {
            let (local, ty) = wasm_values[&value];
            function.instruction(&Instruction::LocalGet(local));
            ty
        }
        LocalStorage::Wasm { values, .. } if values.contains_key(&value) => {
            let (local, ty) = values[&value];
            function.instruction(&Instruction::LocalGet(local));
            ty
        }
        _ => {
            let ty = context.global_types[&value];
            function.instruction(&Instruction::GlobalGet(context.globals[&value]));
            ty
        }
    }
}

fn compile_value_set(
    function: &mut Function,
    value: ValueId,
    context: &ExprContext<'_>,
    emit_value: impl FnOnce(&mut Function),
) {
    if context.wasm_ir.is_mutably_captured(value) {
        let ty = stored_value_type(value, context);
        compile_raw_value_set(function, value, context, |function| {
            emit_value(function);
            function.instruction(&Instruction::StructNew(context.gc.capture_cell_index(ty)));
        });
    } else {
        compile_raw_value_set(function, value, context, emit_value);
    }
}

fn compile_raw_value_set(
    function: &mut Function,
    value: ValueId,
    context: &ExprContext<'_>,
    emit_value: impl FnOnce(&mut Function),
) {
    match context.locals {
        LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&value) => {
            let (field, ty) = frame_values[&value];
            if !ty.has_runtime_value() {
                emit_value(function);
                if ty == Type::None {
                    function.instruction(&Instruction::Drop);
                }
                return;
            }
            let frame = context.locals.frame();
            frame.emit(function);
            emit_value(function);
            function.instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: field,
            });
        }
        LocalStorage::Hybrid { wasm_values, .. } if wasm_values.contains_key(&value) => {
            let ty = wasm_values[&value].1;
            if !ty.has_runtime_value() {
                emit_value(function);
                if ty == Type::None {
                    function.instruction(&Instruction::Drop);
                }
                return;
            }
            emit_value(function);
            debug_assert_ne!(wasm_values[&value].0, u32::MAX);
            function.instruction(&Instruction::LocalSet(wasm_values[&value].0));
        }
        LocalStorage::Wasm { values, .. } if values.contains_key(&value) => {
            let ty = values[&value].1;
            if !ty.has_runtime_value() {
                emit_value(function);
                if ty == Type::None {
                    function.instruction(&Instruction::Drop);
                }
                return;
            }
            emit_value(function);
            debug_assert_ne!(values[&value].0, u32::MAX);
            function.instruction(&Instruction::LocalSet(values[&value].0));
        }
        _ => unreachable!("compiler-owned for-loop values are local"),
    }
}

#[derive(Clone, Copy)]
enum ForCollection {
    Array(crate::ast::ArrayTypeId),
    Set {
        set: crate::ast::TypeApplicationId,
        backing: crate::ast::ArrayTypeId,
    },
    /// A first-class range stored as an immutable GC object.
    Range {
        range: crate::ast::RangeTypeId,
        bound: Type,
    },
    /// A range literal whose end is kept directly in `iterable_value`.
    DirectRange {
        bound: Type,
    },
    /// A first-class cursor consumed by repeatedly calling `Iterator.next`.
    Iterator {
        step: crate::ast::TypeApplicationId,
    },
}

fn for_collection_type(
    iterable_value: ValueId,
    index_value: ValueId,
    context: &ExprContext<'_>,
) -> (ForCollection, Type) {
    let index_ty = context
        .semantics
        .value_type(index_value)
        .expect("checked for-loop cursor state has a type");
    if let Type::Application(step) = context.ty(index_ty)
        && let Some(item) = context
            .semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Application {
                    layout,
                    constructor: StdlibTypeConstructorId::IteratorStep,
                    arguments,
                } if *layout == step => arguments.first().copied(),
                _ => None,
            })
    {
        return (ForCollection::Iterator { step }, context.ty(item));
    }
    let ty = context
        .semantics
        .value_type(iterable_value)
        .expect("checked for-loop iterable storage has a type");
    match context.ty(ty) {
        Type::Array(array) => (
            ForCollection::Array(array),
            array_element_type(array, context.semantics),
        ),
        Type::Set(set) => {
            let (element, backing) = context
                .semantics
                .types()
                .iter()
                .find_map(|(_, kind)| match kind {
                    crate::types::TypeKind::Set {
                        layout,
                        element,
                        backing,
                    } if *layout == set => Some((context.ty(*element), *backing)),
                    _ => None,
                })
                .expect("checked set layouts have iterable storage");
            (ForCollection::Set { set, backing }, element)
        }
        Type::Range(range) => {
            let bound = context
                .semantics
                .types()
                .iter()
                .find_map(|(_, kind)| match kind {
                    crate::types::TypeKind::Range { layout, bound, .. } if *layout == range => {
                        Some(context.ty(*bound))
                    }
                    _ => None,
                })
                .expect("checked range layouts have a bound type");
            (ForCollection::Range { range, bound }, bound)
        }
        // Only compiler-owned `for` storage can reach this helper. A scalar
        // iterable slot is the allocation-free representation of a direct
        // range literal and contains its upper bound.
        bound => (ForCollection::DirectRange { bound }, bound),
    }
}

pub(super) fn compile_for_init(
    function: &mut Function,
    iterable_value: ValueId,
    index_value: ValueId,
    version_value: ValueId,
    iterable: ExprId,
    context: &ExprContext<'_>,
) {
    let collection = for_collection_type(iterable_value, index_value, context).0;
    match collection {
        ForCollection::DirectRange { .. } => {
            let wasm_ir::ExpressionKind::Range { start, end, .. } = &context
                .wasm_ir
                .expression(iterable)
                .expect("range expression exists")
                .kind
            else {
                unreachable!("direct range storage belongs to a range literal")
            };
            compile_value_set(function, iterable_value, context, |function| {
                compile_expr(function, *end, context);
            });
            compile_value_set(function, index_value, context, |function| {
                compile_expr(function, *start, context);
            });
        }
        ForCollection::Range { range, bound } => {
            compile_value_set(function, iterable_value, context, |function| {
                compile_expr(function, iterable, context);
            });
            compile_value_set(function, index_value, context, |function| {
                compile_value_get(function, iterable_value, context);
                function.instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(function, context.gc.index(Type::Range(range)), 0, bound);
            });
        }
        ForCollection::Array(_) | ForCollection::Set { .. } => {
            compile_value_set(function, iterable_value, context, |function| {
                compile_expr(function, iterable, context);
            });
            compile_value_set(function, index_value, context, |function| {
                function.instruction(&Instruction::I32Const(0));
            });
        }
        ForCollection::Iterator { .. } => {
            compile_value_set(function, iterable_value, context, |function| {
                compile_expr(function, iterable, context);
            });
        }
    }
    compile_value_set(
        function,
        version_value,
        context,
        |function| match collection {
            ForCollection::Array(array) => {
                compile_value_get(function, iterable_value, context);
                super::array_value::emit_version(function, context.gc, array);
            }
            ForCollection::Set { set, .. } => {
                compile_value_get(function, iterable_value, context);
                function
                    .instruction(&Instruction::RefAsNonNull)
                    .instruction(&Instruction::StructGet {
                        struct_type_index: context.gc.index(Type::Set(set)),
                        field_index: super::set_functions::VERSION_FIELD,
                    });
            }
            ForCollection::Range { range, .. } => {
                let kind = context
                    .semantics
                    .types()
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        crate::types::TypeKind::Range { layout, kind, .. } if *layout == range => {
                            Some(*kind)
                        }
                        _ => None,
                    })
                    .expect("checked range layouts retain their endpoint kind");
                function.instruction(&Instruction::I32Const(
                    matches!(kind, RangeKind::Inclusive) as i32
                ));
            }
            ForCollection::DirectRange { .. } => {
                let wasm_ir::ExpressionKind::Range { kind, .. } = &context
                    .wasm_ir
                    .expression(iterable)
                    .expect("range expression exists")
                    .kind
                else {
                    unreachable!("direct range storage belongs to a range literal")
                };
                function.instruction(&Instruction::I32Const(
                    matches!(kind, RangeKind::Inclusive) as i32
                ));
            }
            ForCollection::Iterator { .. } => {
                function.instruction(&Instruction::I32Const(0));
            }
        },
    );
}

/// Leaves whether another element exists on the stack.
pub(super) fn compile_for_has_next(
    function: &mut Function,
    iterable_value: ValueId,
    index_value: ValueId,
    version_value: ValueId,
    iterator_step: Option<ExprId>,
    context: &ExprContext<'_>,
) {
    let (collection, _) = for_collection_type(iterable_value, index_value, context);
    if let ForCollection::Iterator { .. } = collection {
        let iterator_step = iterator_step.expect("iterator loops have a generated next call");
        compile_value_set(function, index_value, context, |function| {
            compile_expr(function, iterator_step, context);
        });
        compile_value_get(function, index_value, context);
        function
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::I32Eqz);
        return;
    }
    if matches!(
        collection,
        ForCollection::Range { .. } | ForCollection::DirectRange { .. }
    ) {
        let bound = match collection {
            ForCollection::Range { bound, .. } | ForCollection::DirectRange { bound } => bound,
            _ => unreachable!(),
        };
        let emit_end = |function: &mut Function| match collection {
            ForCollection::Range { range, .. } => {
                compile_value_get(function, iterable_value, context);
                function.instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(function, context.gc.index(Type::Range(range)), 1, bound);
            }
            ForCollection::DirectRange { .. } => {
                compile_value_get(function, iterable_value, context);
            }
            _ => unreachable!(),
        };

        // `version_value` is 0 for an exclusive range, 1 for an active
        // inclusive range, and 2 after the inclusive endpoint has been
        // yielded. Keeping the exhausted state separately avoids overflowing
        // when the inclusive endpoint is the integer type's maximum value.
        compile_value_get(function, index_value, context);
        emit_end(function);
        function.instruction(&compare(bound, bound.is_signed(), Compare::Lt));
        compile_value_get(function, version_value, context);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Eq);
        compile_value_get(function, index_value, context);
        emit_end(function);
        emit_binary_instruction(function, BinaryOp::Eq, bound);
        function
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Or);
        return;
    }

    compile_value_get(function, iterable_value, context);
    match collection {
        ForCollection::Array(array) => {
            super::array_value::emit_version(function, context.gc, array);
        }
        ForCollection::Set { set, .. } => {
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Set(set)),
                    field_index: super::set_functions::VERSION_FIELD,
                });
        }
        ForCollection::Range { .. } | ForCollection::DirectRange { .. } => unreachable!(),
        ForCollection::Iterator { .. } => unreachable!(),
    }
    compile_value_get(function, version_value, context);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
    compile_value_get(function, index_value, context);
    compile_value_get(function, iterable_value, context);
    match collection {
        ForCollection::Array(array) => {
            super::array_value::emit_length(function, context.gc, array);
        }
        ForCollection::Set { set, .. } => {
            function.instruction(&Instruction::RefAsNonNull);
            function.instruction(&Instruction::StructGet {
                struct_type_index: context.gc.index(Type::Set(set)),
                field_index: super::set_functions::LENGTH_FIELD,
            });
        }
        ForCollection::Range { .. }
        | ForCollection::DirectRange { .. }
        | ForCollection::Iterator { .. } => unreachable!(),
    }
    function.instruction(&Instruction::I32LtU);
}

/// Stores the current element in the source binding and advances before the
/// body, so a `continue` cannot accidentally repeat the same element.
pub(super) fn compile_for_bind_and_advance(
    function: &mut Function,
    binding: ValueId,
    iterable_value: ValueId,
    index_value: ValueId,
    version_value: ValueId,
    context: &ExprContext<'_>,
) {
    let (collection, element) = for_collection_type(iterable_value, index_value, context);
    if let ForCollection::Iterator { step } = collection {
        compile_value_set(function, binding, context, |function| {
            compile_value_get(function, index_value, context);
            function.instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(
                function,
                context.gc.index(Type::Application(step)),
                0,
                element,
            );
        });
        return;
    }
    if matches!(
        collection,
        ForCollection::Range { .. } | ForCollection::DirectRange { .. }
    ) {
        compile_value_set(function, binding, context, |function| {
            compile_value_get(function, index_value, context);
        });

        let emit_end = |function: &mut Function| match collection {
            ForCollection::Range { range, bound } => {
                compile_value_get(function, iterable_value, context);
                function.instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(function, context.gc.index(Type::Range(range)), 1, bound);
            }
            ForCollection::DirectRange { .. } => {
                compile_value_get(function, iterable_value, context);
            }
            _ => unreachable!(),
        };
        // The only non-advancing iteration is the inclusive endpoint. Mark it
        // exhausted before entering the body so both fallthrough and
        // `continue` reach a terminating header.
        compile_value_get(function, version_value, context);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Eq);
        compile_value_get(function, index_value, context);
        emit_end(function);
        emit_binary_instruction(function, BinaryOp::Eq, element);
        function
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::If(BlockType::Empty));
        compile_value_set(function, version_value, context, |function| {
            function.instruction(&Instruction::I32Const(2));
        });
        function.instruction(&Instruction::Else);
        compile_value_set(function, index_value, context, |function| {
            compile_value_get(function, index_value, context);
            function.instruction(
                &if matches!(element, Type::I64 | Type::U64 | Type::Address) {
                    Instruction::I64Const(1)
                } else {
                    Instruction::I32Const(1)
                },
            );
            emit_binary_instruction(function, BinaryOp::Add, element);
            emit_narrow_integer_result(function, element);
        });
        function.instruction(&Instruction::End);
        return;
    }

    compile_value_set(function, binding, context, |function| {
        compile_value_get(function, iterable_value, context);
        let array = match collection {
            ForCollection::Array(array) => {
                super::array_value::emit_backing(function, context.gc, array);
                array
            }
            ForCollection::Set { set, backing } => {
                function.instruction(&Instruction::RefAsNonNull);
                function.instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Set(set)),
                    field_index: super::set_functions::BACKING_FIELD,
                });
                function.instruction(&Instruction::RefAsNonNull);
                backing
            }
            ForCollection::Range { .. }
            | ForCollection::DirectRange { .. }
            | ForCollection::Iterator { .. } => unreachable!(),
        };
        compile_value_get(function, index_value, context);
        let backing_type = match collection {
            ForCollection::Array(_) => Type::ArrayStorage(super::array_value::storage_id(
                array,
                context.arrays,
                context.semantics,
            )),
            ForCollection::Set { .. } => Type::ArrayStorage(array),
            ForCollection::Range { .. }
            | ForCollection::DirectRange { .. }
            | ForCollection::Iterator { .. } => unreachable!(),
        };
        emit_array_get(
            function,
            context.gc.index(backing_type),
            element,
            context.gc,
        );
    });
    compile_value_set(function, index_value, context, |function| {
        compile_value_get(function, index_value, context);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Add);
    });
}

fn emit_path_fields(
    function: &mut Function,
    fields: &[ResolvedMember],
    mut current_type: Type,
    context: &ExprContext<'_>,
) -> Type {
    for field in fields {
        let (struct_type_index, field_index, field_type) = match field {
            ResolvedMember::StateField(field) => {
                let (field_index, storage) = state_storage_index(*field, context.semantics);
                let field_type = value_type(storage, context.semantics);
                debug_assert_eq!(current_type, Type::StateSnapshot);
                (STATE_TYPE, field_index, field_type)
            }
            ResolvedMember::SettingField(setting) => {
                debug_assert_eq!(current_type, Type::SettingsView);
                let storage = context.settings[setting];
                function
                    .instruction(&Instruction::If(BlockType::Result(
                        context.gc.val_type(storage.ty),
                    )))
                    .instruction(&Instruction::GlobalGet(storage.old))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::GlobalGet(storage.current))
                    .instruction(&Instruction::End);
                current_type = storage.ty;
                continue;
            }
            ResolvedMember::StandardField(field) => {
                let library = context.standard_library;
                let declaration = library.field(*field);
                match declaration.owner {
                    StdlibOwner::Type(owner) => {
                        let owner = library.type_decl(owner);
                        let RuntimeRepresentation::GcStruct { .. } = owner.representation else {
                            unreachable!("resolved standard field belongs to a GC struct")
                        };
                        let field_index = library
                            .fields_of(owner.id)
                            .position(|candidate| candidate.id == *field)
                            .expect("declared standard field has a runtime slot")
                            as u32;
                        let owner_type = Type::from_standard(owner.id);
                        debug_assert_eq!(current_type, owner_type);
                        (
                            context.gc.index(owner_type),
                            field_index,
                            standard_field_type(declaration.id, context.semantics),
                        )
                    }
                    StdlibOwner::TypeConstructor(owner) => {
                        let field_index = library
                            .fields_of_constructor(owner)
                            .position(|candidate| candidate.id == *field)
                            .expect("declared constructed field has a runtime slot")
                            as u32;
                        let arguments = match current_type {
                            Type::Range(range) => context
                                .semantics
                                .types()
                                .iter()
                                .find_map(|(_, kind)| match kind {
                                    crate::types::TypeKind::Range {
                                        layout,
                                        bound,
                                        kind,
                                    } if *layout == range => {
                                        let constructor = match kind {
                                            crate::ast::RangeKind::Exclusive => crate::stdlib::StdlibTypeConstructorId::ExclusiveRange,
                                            crate::ast::RangeKind::Inclusive => crate::stdlib::StdlibTypeConstructorId::InclusiveRange,
                                        };
                                        (constructor == owner).then_some(vec![*bound])
                                    }
                                    _ => None,
                                }),
                            Type::Application(application) => context
                                .semantics
                                .types()
                                .iter()
                                .find_map(|(_, kind)| match kind {
                                    crate::types::TypeKind::Application {
                                        layout,
                                        constructor,
                                        arguments,
                                    } if *layout == application && *constructor == owner => {
                                        Some(arguments.clone())
                                    }
                                    _ => None,
                                }),
                            _ => None,
                        }
                        .expect("constructed fields retain their concrete receiver arguments");
                        let variables = library
                            .type_constructor(owner)
                            .parameters
                            .iter()
                            .zip(arguments)
                            .map(|(parameter, argument)| (parameter.name, argument))
                            .collect::<std::collections::HashMap<_, _>>();
                        let field_type = semantic_type(
                            super::gc_types::instantiated_catalog_type(
                                declaration.ty,
                                &variables,
                                context.semantics,
                            ),
                            context.semantics,
                        );
                        (context.gc.index(current_type), field_index, field_type)
                    }
                    _ => unreachable!("fields have type or type-constructor owners"),
                }
            }
            ResolvedMember::RecordField(field) => {
                let (record, field_index, field) = context
                    .records
                    .iter()
                    .find_map(|record| {
                        record
                            .fields
                            .iter()
                            .enumerate()
                            .find(|(_, candidate)| candidate.id == *field)
                            .map(|(index, field)| (record, index as u32, field))
                    })
                    .expect("resolved record field belongs to a checked declaration");
                debug_assert_eq!(current_type, Type::Record(record.id));
                (
                    context.gc.index(Type::Record(record.id)),
                    field_index,
                    record_field_type(field.id, context.semantics),
                )
            }
            ResolvedMember::ManagedField(field) => {
                let (class, field_index, field) = context
                    .managed
                    .classes
                    .iter()
                    .find_map(|class| {
                        class
                            .fields
                            .iter()
                            .filter(|candidate| {
                                candidate.kind == crate::managed::ManagedFieldKind::Instance
                            })
                            .enumerate()
                            .find(|(_, candidate)| candidate.id == *field)
                            .map(|(index, field)| (class, index as u32, field))
                    })
                    .expect("resolved managed field belongs to a checked declaration");
                debug_assert_eq!(current_type, Type::ManagedClass(class.id));
                (
                    context.gc.index(Type::ManagedClass(class.id)),
                    field_index,
                    semantic_type(field.value_type, context.semantics),
                )
            }
        };
        if field_type == Type::None && !context.materialize_none {
            function.instruction(&Instruction::Drop);
        } else {
            function.instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, struct_type_index, field_index, field_type);
        }
        current_type = field_type;
    }
    current_type
}

fn resolved_value_type_id(value: ResolvedValue, context: &ExprContext<'_>) -> Option<TypeId> {
    match value {
        ResolvedValue::Variable(value)
        | ResolvedValue::CurrentState(value)
        | ResolvedValue::OldState(value) => context.semantics.value_type(value)?,
        ResolvedValue::Setting(value) | ResolvedValue::OldSetting(value) => {
            context.semantics.value_type(value)?
        }
        ResolvedValue::StandardLibraryConstant(_)
        | ResolvedValue::ProviderValue(_)
        | ResolvedValue::ManagedStatic { .. }
        | ResolvedValue::CurrentSnapshot
        | ResolvedValue::OldSnapshot
        | ResolvedValue::SettingsView
        | ResolvedValue::OldSettingsView => return None,
    }
    .into()
}

fn managed_field_binding<'context>(
    field: crate::ast::ManagedFieldId,
    context: &'context ExprContext<'_>,
) -> &'context crate::managed::ManagedFieldBinding {
    context
        .managed
        .classes
        .iter()
        .flat_map(|class| {
            class.fields.iter().chain(
                class
                    .conditional_fields
                    .iter()
                    .flat_map(|group| &group.fields),
            )
        })
        .find(|candidate| candidate.id == field)
        .expect("resolved managed fields belong to the binding plan")
}

/// Reads a managed static field, sharing the fallible result when this body is
/// part of one state-snapshot transaction. The cache contains the complete
/// `T!`, so both successful singleton addresses and failures are consistent
/// across sibling fields in the same candidate snapshot.
fn emit_managed_static_read(
    function: &mut Function,
    class: crate::ast::ManagedClassId,
    field: crate::ast::ManagedFieldId,
    context: &ExprContext<'_>,
) -> Type {
    if let Some(function_index) = context.managed_state_read_functions.get(&field) {
        let storage = context
            .managed_state_reads
            .get(field)
            .expect("planned managed read functions have cache storage");
        function.instruction(&Instruction::Call(*function_index));
        Type::Result(storage.result)
    } else {
        emit_uncached_managed_static_read(function, class, field, context)
    }
}

pub(super) fn compile_managed_static_read(
    storage: super::managed_state_reads::ManagedStateReadStorage,
    lowering: &super::context::EmissionContext<'_>,
) -> Function {
    let values = HashMap::new();
    let temporaries = HashMap::new();
    let matches = MatchLayout::default();
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        abi: lowering.abi,
        state: lowering.state,
        locals: LocalStorage::Wasm {
            values: &values,
            temporaries: &temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_polls: lowering.closure_polls,
        closure_environment: None,
        intrinsic_futures: lowering.intrinsic_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        records: lowering.records,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        intrinsic_capture: None,
        debug: None,
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    let mut function = Function::new([]);
    emit_cached_managed_static_read(&mut function, storage, &context);
    function.instruction(&Instruction::End);
    function
}

fn emit_cached_managed_static_read(
    function: &mut Function,
    storage: super::managed_state_reads::ManagedStateReadStorage,
    context: &ExprContext<'_>,
) -> Type {
    let result_type = Type::Result(storage.result);
    let active = context
        .managed_state_reads
        .active()
        .expect("managed cache entries have an activity flag");
    function
        .instruction(&Instruction::GlobalGet(active))
        .instruction(&Instruction::GlobalGet(storage.global))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(result_type),
        )))
        .instruction(&Instruction::GlobalGet(storage.global))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::Else);
    emit_uncached_managed_static_read(function, storage.class, storage.field, context);
    function
        // Outside a snapshot transaction this slot is deliberately
        // overwritten but never selected. Doing so keeps one canonical
        // read body instead of duplicating it in both branches; the next
        // transaction clears all slots before enabling cache hits.
        .instruction(&Instruction::GlobalSet(storage.global))
        .instruction(&Instruction::GlobalGet(storage.global))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::End);
    result_type
}

fn emit_uncached_managed_static_read(
    function: &mut Function,
    class: crate::ast::ManagedClassId,
    field: crate::ast::ManagedFieldId,
    context: &ExprContext<'_>,
) -> Type {
    function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    let binding = managed_field_binding(field, context);
    emit_managed_binding_field(function, &managed_static_table_name(class.index()), context);
    emit_managed_binding_field(function, &managed_field_offset_name(field.index()), context);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add);
    emit_managed_read_at_address(function, binding, context)
}

/// Reads an instance field from a `T.Ref` address already on the stack after
/// the attached process handle. Metadata lookup is absent here: the offset is
/// loaded from the attachment-scoped binding record.
fn emit_managed_field_read(
    function: &mut Function,
    field: crate::ast::ManagedFieldId,
    context: &ExprContext<'_>,
) -> Type {
    emit_managed_binding_field(function, &managed_field_offset_name(field.index()), context);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add);
    emit_managed_read_at_address(function, managed_field_binding(field, context), context)
}

/// Consumes `(process, address)` and produces the ordinary `T!` representation
/// for one remote managed field. Managed references honor the detected target
/// pointer width; terminal values use their normal `MemoryReadable` layout.
fn emit_managed_read_at_address(
    function: &mut Function,
    field: &crate::managed::ManagedFieldBinding,
    context: &ExprContext<'_>,
) -> Type {
    let result = context
        .semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            crate::types::TypeKind::Result { layout, value } if *value == field.value_type => {
                Some(*layout)
            }
            _ => None,
        })
        .expect("a checked managed field access has a concrete Result layout");
    let value_type = semantic_type(field.value_type, context.semantics);

    if matches!(
        context.semantics.types().kind(field.value_type),
        crate::types::TypeKind::ManagedReference(_)
    ) {
        function.instruction(&Instruction::I32Const(context.abi_read.destination(8)));
        emit_managed_binding_field(function, MANAGED_POINTER_SIZE_FIELD, context);
        function
            .instruction(&Instruction::Call(
                context.abi.function(AbiImportId::ProcessRead),
            ))
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Result(result)),
            )));
        emit_managed_pointer_from_scratch(function, context);
        function
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Result(result)),
            )));
        emit_result_error(
            function,
            result,
            value_type,
            "managed field contained a null reference",
            context.gc,
        );
        function.instruction(&Instruction::Else);
        emit_managed_pointer_from_scratch(function, context);
        emit_result_success(function, result, context.gc);
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::Else);
        emit_result_error(
            function,
            result,
            value_type,
            "managed field could not be read",
            context.gc,
        );
        function.instruction(&Instruction::End);
    } else {
        let size = context
            .memory
            .layout(field.value_type, context.semantics)
            .expect("checked managed value fields are MemoryReadable")
            .size();
        function
            .instruction(&Instruction::I32Const(context.abi_read.destination(size)))
            .instruction(&Instruction::I32Const(size as i32))
            .instruction(&Instruction::Call(
                context.abi.function(AbiImportId::ProcessRead),
            ))
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Result(result)),
            )));
        emit_memory_value(
            function,
            field.value_type,
            context.abi_read,
            0,
            context.memory,
            context.semantics,
            context.gc,
            MemoryByteOrder::Little,
        );
        emit_result_success(function, result, context.gc);
        function.instruction(&Instruction::Else);
        emit_result_error(
            function,
            result,
            value_type,
            "managed field could not be read",
            context.gc,
        );
        function.instruction(&Instruction::End);
    }
    Type::Result(result)
}

fn emit_managed_pointer_from_scratch(function: &mut Function, context: &ExprContext<'_>) {
    emit_managed_binding_field(function, MANAGED_POINTER_SIZE_FIELD, context);
    function
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I32Const(context.abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(context.abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::End);
}

fn emit_managed_binding_field(function: &mut Function, name: &str, context: &ExprContext<'_>) {
    let record = context
        .records
        .iter()
        .find(|record| record.name == MANAGED_BINDINGS_TYPE)
        .expect("managed schemas generate an attachment binding record");
    let (field_index, field) = record
        .fields
        .iter()
        .enumerate()
        .find(|(_, field)| field.name == name)
        .expect("managed binding records contain every generated metadata field");
    let field_type = record_field_type(field.id, context.semantics);
    function
        .instruction(&Instruction::GlobalGet(
            context
                .runtime_globals
                .provider_preparation_value
                .expect("managed schemas require provider preparation storage"),
        ))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        context.gc.index(Type::Record(record.id)),
        field_index as u32,
        field_type,
    );
}

pub(super) fn compile_expr(function: &mut Function, expression: ExprId, context: &ExprContext<'_>) {
    let expression_ir = context
        .wasm_ir
        .expression(expression)
        .expect("compiled expression belongs to Wasm IR");
    if let Some(debug) = context.debug {
        debug.mark(function, expression_ir.source);
    }
    if let Some(capture) = context.intrinsic_capture
        && let Some(&(field, ty)) = capture.layout.arguments.get(&expression)
    {
        if ty != Type::None || context.materialize_none {
            capture.frame.emit(function);
            emit_typed_struct_get(function, capture.frame.struct_type, field, ty);
        }
        return;
    }
    let ty = context.ty(expression_ir.ty);
    if let Some(conversion) = expression_ir.conversion {
        let source = context.ty(conversion.source);
        let target = context.ty(conversion.target);
        match (conversion.kind, target) {
            (ValueConversionKind::NoneToOptional, Type::Option(option)) => {
                function.instruction(&Instruction::RefNull(HeapType::Concrete(
                    context.gc.index(Type::Option(option)),
                )));
                return;
            }
            (ValueConversionKind::NoneToDomainNullable, Type::Bool) => {
                function.instruction(&Instruction::I32Const(-1));
                return;
            }
            (ValueConversionKind::NoneToDomainNullable, Type::Standard(StdlibTypeId::Duration)) => {
                function.instruction(&Instruction::RefNull(HeapType::Concrete(
                    context.gc.standard_index(StdlibTypeId::Duration),
                )));
                return;
            }
            _ => {}
        }
        compile_expr_unconverted(function, expression_ir, source, context);
        match (conversion.kind, target) {
            (ValueConversionKind::LiftOption, Type::Option(option)) => {
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Option(option)),
                ));
            }
            (ValueConversionKind::LiftResult, Type::Result(result)) => {
                // The successful value is already on the stack. Tag 0 and a
                // null error complete the monomorphized result struct.
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.standard_index(StdlibTypeId::String),
                    )))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Result(result)),
                    ));
            }
            _ => unreachable!("typed wrapper conversions have matching target layouts"),
        }
        return;
    }
    compile_expr_unconverted(function, expression_ir, ty, context);
    // A `Never` expression has no physical result and cannot reach its
    // continuation. Mark that fact in Wasm as well, so code emitted for a
    // structurally present but unreachable continuation remains stack-valid.
    if ty == Type::Never {
        function.instruction(&Instruction::Unreachable);
    }
}

fn compile_expr_unconverted(
    function: &mut Function,
    expression_ir: &wasm_ir::Expression,
    ty: Type,
    context: &ExprContext<'_>,
) {
    let expression = expression_ir.id;
    match &expression_ir.kind {
        wasm_ir::ExpressionKind::ValueBlock | wasm_ir::ExpressionKind::Loop => {
            unreachable!("value blocks are lowered before expression code generation")
        }
        wasm_ir::ExpressionKind::Suspend { destination, .. } => {
            if ty.has_runtime_value() || (ty == Type::None && context.materialize_none) {
                compile_value_get(function, *destination, context);
            }
        }
        wasm_ir::ExpressionKind::Temporary(temporary) => {
            if ty.has_runtime_value() || (ty == Type::None && context.materialize_none) {
                compile_temporary_get(function, *temporary, context);
            }
        }
        wasm_ir::ExpressionKind::FallbackSuccess { source } => {
            compile_fallback_success(function, *source, ty, context);
        }
        wasm_ir::ExpressionKind::None => match ty {
            Type::None if context.materialize_none => {
                function.instruction(&Instruction::RefNull(HeapType::Abstract {
                    shared: false,
                    ty: AbstractHeapType::None,
                }));
            }
            Type::None => {}
            Type::Bool => {
                function.instruction(&Instruction::I32Const(-1));
            }
            Type::Standard(StdlibTypeId::Duration) => {
                function.instruction(&Instruction::RefNull(HeapType::Concrete(
                    context.gc.standard_index(StdlibTypeId::Duration),
                )));
            }
            Type::Option(option) => {
                function.instruction(&Instruction::RefNull(HeapType::Concrete(
                    context.gc.index(Type::Option(option)),
                )));
            }
            _ => unreachable!(
                "typed None expressions use the unit or nullable representation, found {ty:?} with conversion {:?}",
                expression_ir.conversion
            ),
        },
        wasm_ir::ExpressionKind::IteratorEnd => {
            let Type::Application(step) = ty else {
                unreachable!("End expressions have IteratorStep<T> type")
            };
            function.instruction(&Instruction::RefNull(HeapType::Concrete(
                context.gc.index(Type::Application(step)),
            )));
        }
        wasm_ir::ExpressionKind::Bool(value) => {
            function.instruction(&Instruction::I32Const(*value as i32));
        }
        wasm_ir::ExpressionKind::Int(value) => emit_int(function, *value, ty),
        wasm_ir::ExpressionKind::Char(value) => {
            function.instruction(&Instruction::I32Const(*value as i32));
        }
        wasm_ir::ExpressionKind::Float(literal) => {
            if ty == Type::F32 {
                let value = literal
                    .normalized
                    .parse::<f32>()
                    .expect("checked f32 literals fit their target");
                function.instruction(&Instruction::F32Const(value.into()));
            } else {
                function.instruction(&Instruction::F64Const(literal.value.into()));
            }
        }
        wasm_ir::ExpressionKind::String(value) => emit_string_literal(function, value, context.gc),
        wasm_ir::ExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                match part {
                    wasm_ir::InterpolatedPart::Text(value) => {
                        emit_string_literal(function, value, context.gc)
                    }
                    wasm_ir::InterpolatedPart::Expression {
                        expression,
                        string_conversion_source: None,
                    } => compile_expr(function, *expression, context),
                    wasm_ir::InterpolatedPart::Expression {
                        expression,
                        string_conversion_source: Some(source),
                    } => {
                        debug_assert_eq!(context.ty(*source), context.expression_type(*expression));
                        emit_cast(
                            function,
                            *expression,
                            Type::Standard(StdlibTypeId::String),
                            context,
                        );
                    }
                }
            }
            let strings = context
                .arrays
                .iter()
                .find(|array| {
                    try_array_element_type(array.id, context.semantics)
                        == Some(Type::Standard(StdlibTypeId::String))
                })
                .expect("interpolation creates its String array type");
            super::array_value::emit_new_fixed(
                function,
                context.gc,
                strings.id,
                parts.len() as u32,
            );
            function
                .instruction(&Instruction::RefNull(HeapType::Concrete(
                    context.gc.standard_index(StdlibTypeId::String),
                )))
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::JoinStrings),
                ));
        }
        wasm_ir::ExpressionKind::Signature(signature) => {
            let entry = context.signatures.get(signature);
            let packed = u64::from(entry.needle) | (u64::from(entry.len) << 32);
            function.instruction(&Instruction::I64Const(packed as i64));
        }
        wasm_ir::ExpressionKind::Array(elements) => {
            let Type::Array(array_id) = ty else {
                unreachable!();
            };
            for element in elements {
                compile_expr(function, *element, context);
            }
            super::array_value::emit_new_fixed(
                function,
                context.gc,
                array_id,
                elements.len() as u32,
            );
        }
        wasm_ir::ExpressionKind::Range { start, end, .. } => {
            let Type::Range(range) = ty else {
                unreachable!("typed range literals have range types")
            };
            compile_expr(function, *start, context);
            compile_expr(function, *end, context);
            function.instruction(&Instruction::StructNew(
                context.gc.index(Type::Range(range)),
            ));
        }
        wasm_ir::ExpressionKind::Record { record, fields } => match record {
            ResolvedRecordId::Source(record) => {
                let declaration = context
                    .records
                    .iter()
                    .find(|declaration| declaration.id == *record)
                    .unwrap();
                for declared_field in &declaration.fields {
                    let (_, value) = fields
                        .iter()
                        .find(|(field, _)| {
                            *field == ResolvedRecordFieldId::Source(declared_field.id)
                        })
                        .expect("checked record literals initialize every declared field");
                    compile_expr(function, *value, context);
                }
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Record(*record)),
                ));
            }
            ResolvedRecordId::Standard(record) => {
                for declared_field in context.standard_library.fields_of(*record) {
                    let (_, value) = fields
                        .iter()
                        .find(|(field, _)| {
                            *field == ResolvedRecordFieldId::Standard(declared_field.id)
                        })
                        .expect("checked library record literals initialize every field");
                    compile_expr(function, *value, context);
                }
                function.instruction(&Instruction::StructNew(context.gc.standard_index(*record)));
            }
            ResolvedRecordId::StandardConstructor(_application) => {
                let Type::Application(concrete_application) = ty else {
                    unreachable!("constructed standard records have application types")
                };
                let constructor = context
                    .semantics
                    .types()
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        crate::types::TypeKind::Application {
                            layout,
                            constructor,
                            ..
                        } if *layout == concrete_application => Some(*constructor),
                        _ => None,
                    })
                    .expect("checked standard-library record constructors have layouts");
                for declared_field in context.standard_library.fields_of_constructor(constructor) {
                    let (_, value) = fields
                        .iter()
                        .find(|(field, _)| {
                            *field == ResolvedRecordFieldId::Standard(declared_field.id)
                        })
                        .expect("checked constructed record literals initialize every field");
                    compile_expr(function, *value, context);
                }
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Application(concrete_application)),
                ));
            }
        },
        wasm_ir::ExpressionKind::Enum {
            enumeration,
            variant,
            payload,
        } => {
            let selected = context
                .gc
                .enum_variant_index(*enumeration, *variant, context.enums);
            function.instruction(&Instruction::I32Const(selected as i32));
            match enumeration {
                EnumTypeId::Source(enumeration) => {
                    let declaration = context
                        .enums
                        .iter()
                        .find(|declaration| declaration.id == *enumeration)
                        .unwrap();
                    for (index, declared) in declaration.variants.iter().enumerate() {
                        if index == selected {
                            if let Some(payload) = payload {
                                compile_expr(function, *payload, context);
                            } else {
                                function.instruction(&Instruction::I32Const(0));
                            }
                        } else if let Some(payload_type) =
                            enum_variant_payload(declared.id, context.semantics)
                        {
                            emit_default(function, payload_type, context.gc);
                        } else {
                            function.instruction(&Instruction::I32Const(0));
                        }
                    }
                }
                EnumTypeId::Standard(enumeration) => {
                    debug_assert!(payload.is_none());
                    for _ in context.standard_library.variants_of(*enumeration) {
                        function.instruction(&Instruction::I32Const(0));
                    }
                }
            }
            function.instruction(&Instruction::StructNew(context.gc.index(ty)));
        }
        wasm_ir::ExpressionKind::Path { root, members } => {
            let root = root.expect("lowerable paths have resolved value roots");
            let lowered_type = compile_resolved_path(function, root, members, context);
            debug_assert_eq!(lowered_type, ty);
        }
        wasm_ir::ExpressionKind::Member { receiver, members } => {
            let receiver_type = context.expression_type(*receiver);
            let lowered_type = if let [ResolvedMember::ManagedField(field)] = members.as_slice()
                && matches!(
                    context
                        .semantics
                        .types()
                        .kind(context.expression_type_id(*receiver)),
                    crate::types::TypeKind::ManagedReference(_)
                ) {
                function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
                compile_expr(function, *receiver, context);
                emit_managed_field_read(function, *field, context)
            } else {
                compile_expr(function, *receiver, context);
                emit_path_fields(function, members, receiver_type, context)
            };
            debug_assert_eq!(lowered_type, ty);
        }
        wasm_ir::ExpressionKind::Index { receiver, index } => {
            compile_expr(function, *receiver, context);
            let Type::Array(array_id) = context.expression_type(*receiver) else {
                unreachable!("checked index receivers are arrays")
            };
            super::array_value::emit_backing(function, context.gc, array_id);
            compile_expr(function, *index, context);
            let element = array_element_type(array_id, context.semantics);
            emit_array_get(
                function,
                context
                    .gc
                    .index(Type::ArrayStorage(super::array_value::storage_id(
                        array_id,
                        context.arrays,
                        context.semantics,
                    ))),
                element,
                context.gc,
            );
        }
        wasm_ir::ExpressionKind::Unary { .. } => {
            unreachable!("checked unary operators lower through catalog calls")
        }
        wasm_ir::ExpressionKind::Cast { value } => emit_cast(function, *value, ty, context),
        wasm_ir::ExpressionKind::Binary { op, left, right } => {
            emit_binary(function, *op, *left, *right, context)
        }
        wasm_ir::ExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            compile_expr(function, *condition, context);
            let block_type = if ty == Type::None && !context.materialize_none {
                BlockType::Empty
            } else {
                BlockType::Result(context.gc.val_type(ty))
            };
            function.instruction(&Instruction::If(block_type));
            let nested_context = context.nested_loop_control(1);
            compile_expr(function, *then_expr, &nested_context);
            function.instruction(&Instruction::Else);
            compile_expr(function, *else_expr, &nested_context);
            function.instruction(&Instruction::End);
        }
        wasm_ir::ExpressionKind::Fallback { value, fallback } => {
            let input_local = context.matches.fallback_values[&expression];
            let input_type = context.expression_type(*value);
            compile_expr(function, *value, context);
            function.instruction(&Instruction::LocalSet(input_local));
            let result = if ty == Type::None && !context.materialize_none {
                BlockType::Empty
            } else {
                BlockType::Result(context.gc.val_type(ty))
            };
            match input_type {
                Type::Option(option) => {
                    function
                        .instruction(&Instruction::LocalGet(input_local))
                        .instruction(&Instruction::RefIsNull)
                        .instruction(&Instruction::If(result));
                    let nested_context = context.nested_loop_control(1);
                    compile_expr(function, *fallback, &nested_context);
                    function.instruction(&Instruction::Else);
                    if ty != Type::None || context.materialize_none {
                        function
                            .instruction(&Instruction::LocalGet(input_local))
                            .instruction(&Instruction::RefAsNonNull);
                        emit_typed_struct_get(
                            function,
                            context.gc.index(Type::Option(option)),
                            0,
                            ty,
                        );
                    }
                    function.instruction(&Instruction::End);
                }
                Type::Result(result_type) => {
                    function
                        .instruction(&Instruction::LocalGet(input_local))
                        .instruction(&Instruction::RefAsNonNull);
                    emit_typed_struct_get(
                        function,
                        context.gc.index(Type::Result(result_type)),
                        1,
                        Type::I32,
                    );
                    function.instruction(&Instruction::If(result));
                    let nested_context = context.nested_loop_control(1);
                    compile_expr(function, *fallback, &nested_context);
                    function.instruction(&Instruction::Else);
                    if ty != Type::None || context.materialize_none {
                        function
                            .instruction(&Instruction::LocalGet(input_local))
                            .instruction(&Instruction::RefAsNonNull);
                        emit_typed_struct_get(
                            function,
                            context.gc.index(Type::Result(result_type)),
                            0,
                            ty,
                        );
                    }
                    function.instruction(&Instruction::End);
                }
                _ => unreachable!("typed fallback inputs are optional or result values"),
            }
        }
        wasm_ir::ExpressionKind::Break(value) => {
            let control = context
                .loop_control
                .expect("checked break expressions belong to loops");
            if let Some(value) = value {
                if let Some(destination) = control.break_destination() {
                    compile_temporary_set(function, destination, *value, context);
                } else {
                    compile_expr(function, *value, &context.erasing_none());
                    if context.expression_type(*value).has_runtime_value() {
                        function.instruction(&Instruction::Drop);
                    }
                }
            }
            control.emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::ExpressionKind::Continue => {
            context
                .loop_control
                .expect("checked continue expressions belong to loops")
                .emit_continue(function, context.locals.continuation_frame());
        }
        wasm_ir::ExpressionKind::Return(value) => {
            compile_return_expression(function, *value, context);
        }
        wasm_ir::ExpressionKind::Throw { error, target } => match target {
            crate::hir::FailureTarget::Return(target) => {
                let Type::Result(target_result) = context.ty(*target) else {
                    unreachable!("throw targets are result values")
                };
                emit_failure_transfer(
                    function,
                    target_result,
                    result_value_type(target_result, context.semantics),
                    context.gc,
                    |function| compile_expr(function, *error, context),
                );
            }
            crate::hir::FailureTarget::Retry { .. } => {
                compile_expr(function, *error, context);
                function
                    .instruction(&Instruction::Drop)
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            }
        },
        wasm_ir::ExpressionKind::Propagate { value, target } => {
            let input_local = context.matches.fallback_values[&expression];
            let Type::Result(input_result) = context.expression_type(*value) else {
                unreachable!("typed propagation inputs are result values")
            };
            compile_expr(function, *value, context);
            function
                .instruction(&Instruction::LocalSet(input_local))
                .instruction(&Instruction::LocalGet(input_local))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(
                function,
                context.gc.index(Type::Result(input_result)),
                1,
                Type::I32,
            );
            function.instruction(&Instruction::If(BlockType::Empty));
            match target {
                crate::hir::FailureTarget::Return(target) => {
                    let Type::Result(target_result) = context.ty(*target) else {
                        unreachable!("propagation targets are result values")
                    };
                    emit_failure_transfer(
                        function,
                        target_result,
                        result_value_type(target_result, context.semantics),
                        context.gc,
                        |function| {
                            function
                                .instruction(&Instruction::LocalGet(input_local))
                                .instruction(&Instruction::RefAsNonNull);
                            emit_typed_struct_get(
                                function,
                                context.gc.index(Type::Result(input_result)),
                                2,
                                Type::Standard(StdlibTypeId::String),
                            );
                        },
                    );
                }
                crate::hir::FailureTarget::Retry { .. } => {
                    function
                        .instruction(&Instruction::I32Const(0))
                        .instruction(&Instruction::Return);
                }
            }
            function.instruction(&Instruction::End);
            if ty != Type::None || context.materialize_none {
                function
                    .instruction(&Instruction::LocalGet(input_local))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(
                    function,
                    context.gc.index(Type::Result(input_result)),
                    0,
                    ty,
                );
            }
        }
        wasm_ir::ExpressionKind::Match { value, arms } => {
            let value_local = context.matches.values[&expression];
            let value_type = context.expression_type(*value);
            compile_expr(function, *value, context);
            function.instruction(&Instruction::LocalSet(value_local));
            let block_type = if ty == Type::None && !context.materialize_none {
                BlockType::Empty
            } else {
                BlockType::Result(context.gc.val_type(ty))
            };
            for (arm_index, arm) in arms.iter().enumerate() {
                let enum_pattern = if let wasm_ir::LoweredPattern::Enum {
                    enumeration,
                    variant,
                    binding,
                } = &arm.pattern
                {
                    let variant_index =
                        context
                            .gc
                            .enum_variant_index(*enumeration, *variant, context.enums);
                    Some((*enumeration, variant_index, *binding))
                } else {
                    None
                };
                let binding = match &arm.pattern {
                    wasm_ir::LoweredPattern::Enum { .. } => {
                        let (_, variant_index, binding) = enum_pattern.unwrap();
                        binding.map(|binding| {
                            (
                                binding,
                                context.gc.index(value_type),
                                variant_index as u32 + 1,
                            )
                        })
                    }
                    wasm_ir::LoweredPattern::OptionSome { binding, .. } => {
                        let Type::Option(option) = value_type else {
                            unreachable!("Some patterns match Option values")
                        };
                        binding.map(|binding| (binding, context.gc.index(Type::Option(option)), 0))
                    }
                    wasm_ir::LoweredPattern::IteratorItem { binding, .. } => {
                        let Type::Application(step) = value_type else {
                            unreachable!("Item patterns match IteratorStep values")
                        };
                        binding
                            .map(|binding| (binding, context.gc.index(Type::Application(step)), 0))
                    }
                    wasm_ir::LoweredPattern::ResultSuccess { binding, .. } => {
                        let Type::Result(result) = value_type else {
                            unreachable!("Ok patterns match Result values")
                        };
                        binding.map(|binding| (binding, context.gc.index(Type::Result(result)), 0))
                    }
                    wasm_ir::LoweredPattern::ResultError { binding, .. } => {
                        let Type::Result(result) = value_type else {
                            unreachable!("Err patterns match Result values")
                        };
                        binding.map(|binding| (binding, context.gc.index(Type::Result(result)), 2))
                    }
                    _ => None,
                };
                let binding = binding.map(|(binding, struct_type, field_index)| {
                    (
                        binding,
                        context.ty(context
                            .semantics
                            .value_type(binding)
                            .expect("checked pattern bindings have types")),
                        struct_type,
                        field_index,
                    )
                });
                match &arm.pattern {
                    wasm_ir::LoweredPattern::Enum { .. } => {
                        let (_, variant_index, _) = enum_pattern.unwrap();
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefAsNonNull);
                        emit_typed_struct_get(function, context.gc.index(value_type), 0, Type::I32);
                        function
                            .instruction(&Instruction::I32Const(variant_index as i32))
                            .instruction(&Instruction::I32Eq);
                    }
                    wasm_ir::LoweredPattern::Bool(expected) => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::I32Const(*expected as i32))
                            .instruction(&Instruction::I32Eq);
                    }
                    wasm_ir::LoweredPattern::Char(expected) => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::I32Const(*expected as i32))
                            .instruction(&Instruction::I32Eq);
                    }
                    wasm_ir::LoweredPattern::String(expected) => {
                        function.instruction(&Instruction::LocalGet(value_local));
                        emit_string_literal(function, expected, context.gc);
                        function.instruction(&Instruction::Call(
                            context
                                .runtime_helpers
                                .function(RuntimeHelperId::StringEquality),
                        ));
                    }
                    wasm_ir::LoweredPattern::Int(value) => {
                        function.instruction(&Instruction::LocalGet(value_local));
                        emit_int(function, *value, value_type);
                        function.instruction(&if matches!(
                            value_type,
                            Type::I64 | Type::U64 | Type::Address
                        ) {
                            Instruction::I64Eq
                        } else {
                            Instruction::I32Eq
                        });
                    }
                    wasm_ir::LoweredPattern::FileVersion(components) => {
                        compile_file_version_pattern(
                            function,
                            components,
                            value_local,
                            value_type,
                            context,
                        );
                    }
                    wasm_ir::LoweredPattern::OptionNone(_) => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefIsNull);
                    }
                    wasm_ir::LoweredPattern::OptionSome { .. } => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefIsNull)
                            .instruction(&Instruction::I32Eqz);
                    }
                    wasm_ir::LoweredPattern::IteratorEnd(_) => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefIsNull);
                    }
                    wasm_ir::LoweredPattern::IteratorItem { .. } => {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefIsNull)
                            .instruction(&Instruction::I32Eqz);
                    }
                    wasm_ir::LoweredPattern::ResultSuccess { .. }
                    | wasm_ir::LoweredPattern::ResultError { .. } => {
                        let Type::Result(result) = value_type else {
                            unreachable!("result patterns match Result values")
                        };
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefAsNonNull);
                        emit_typed_struct_get(
                            function,
                            context.gc.index(Type::Result(result)),
                            1,
                            Type::I32,
                        );
                        function.instruction(&Instruction::I32Const(matches!(
                            &arm.pattern,
                            wasm_ir::LoweredPattern::ResultError { .. }
                        )
                            as i32));
                        function.instruction(&Instruction::I32Eq);
                    }
                    wasm_ir::LoweredPattern::Wildcard => {
                        function.instruction(&Instruction::I32Const(1));
                    }
                }
                let arm_context = ExprContext {
                    loop_control: context
                        .loop_control
                        .map(|control| control.nested(arm_index as u32 + 1)),
                    ..*context
                };
                if let Some((binding, payload_type, struct_type, field_index)) = binding {
                    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
                    compile_value_set(function, binding, &arm_context, |function| {
                        function
                            .instruction(&Instruction::LocalGet(value_local))
                            .instruction(&Instruction::RefAsNonNull);
                        emit_typed_struct_get(function, struct_type, field_index, payload_type);
                    });
                    if let Some(guard) = arm.guard {
                        compile_expr(function, guard, &arm_context);
                    } else {
                        function.instruction(&Instruction::I32Const(1));
                    }
                    function
                        .instruction(&Instruction::Else)
                        .instruction(&Instruction::I32Const(0))
                        .instruction(&Instruction::End);
                } else if let Some(guard) = arm.guard {
                    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
                    compile_expr(function, guard, &arm_context);
                    function
                        .instruction(&Instruction::Else)
                        .instruction(&Instruction::I32Const(0))
                        .instruction(&Instruction::End);
                }
                function.instruction(&Instruction::If(block_type));
                compile_expr(function, arm.value, &arm_context);
                function.instruction(&Instruction::Else);
            }
            function.instruction(&Instruction::Unreachable);
            for _ in arms {
                function.instruction(&Instruction::End);
            }
        }
        wasm_ir::ExpressionKind::Invoke { callee, arguments } => {
            let Type::Callable(callable) = (match callee {
                crate::semantic::DynamicCallCallee::Expression(callee) => {
                    context.expression_type(*callee)
                }
                crate::semantic::DynamicCallCallee::Value(value) => context.ty(context
                    .semantics
                    .value_type(*value)
                    .expect("checked callable values have semantic types")),
            }) else {
                unreachable!("checked dynamic callees have callable types")
            };
            let closure_local = context.matches.intrinsic_temps[&expression][0];
            match callee {
                crate::semantic::DynamicCallCallee::Expression(callee) => {
                    compile_expr(function, *callee, context)
                }
                crate::semantic::DynamicCallCallee::Value(value) => {
                    compile_value_get(function, *value, context);
                }
            }
            function.instruction(&Instruction::LocalSet(closure_local));
            function
                .instruction(&Instruction::LocalGet(closure_local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Callable(callable)),
                    field_index: 1,
                });
            for argument in arguments {
                compile_expr(function, *argument, context);
            }
            function
                .instruction(&Instruction::LocalGet(closure_local))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Callable(callable)),
                    field_index: 0,
                })
                .instruction(&Instruction::CallRef(
                    context.gc.callable_function_index(callable),
                ));
        }
        wasm_ir::ExpressionKind::Closure { closure, .. } => {
            let Type::Callable(callable) = ty else {
                unreachable!("checked closure expressions have callable types")
            };
            let closure_body = context
                .wasm_ir
                .closure(*closure)
                .expect("closure expressions have lowered bodies");
            let instance =
                crate::semantic::ClosureInstance::new(context.function_instance.cloned(), *closure);
            function.instruction(&Instruction::RefFunc(context.closures[&instance]));
            if let Some(environment) = context.gc.closure_environment_index(&instance) {
                for capture in &closure_body.captures {
                    if capture.mutable {
                        emit_raw_value_get(function, capture.value, context);
                    } else {
                        compile_value_get(function, capture.value, context);
                    }
                }
                function.instruction(&Instruction::StructNew(environment));
            } else {
                function.instruction(&Instruction::RefNull(HeapType::Abstract {
                    shared: false,
                    ty: AbstractHeapType::Any,
                }));
            }
            function.instruction(&Instruction::StructNew(
                context.gc.index(Type::Callable(callable)),
            ));
        }
        wasm_ir::ExpressionKind::FunctionValue { function: target } => {
            let Type::Callable(callable) = ty else {
                unreachable!("checked function values have callable types")
            };
            let target = context.called_instance(target);
            let instance = crate::semantic::FunctionValueInstance {
                function: target,
                ty: context.expression_type_id(expression),
            };
            function.instruction(&Instruction::RefFunc(context.function_values[&instance]));
            function.instruction(&Instruction::RefNull(HeapType::Abstract {
                shared: false,
                ty: AbstractHeapType::Any,
            }));
            function.instruction(&Instruction::StructNew(
                context.gc.index(Type::Callable(callable)),
            ));
        }
        wasm_ir::ExpressionKind::Call { .. } => {}
    }
    let wasm_ir::ExpressionKind::Call {
        target,
        arguments: args,
    } = &expression_ir.kind
    else {
        return;
    };
    let target =
        context
            .reachability
            .resolved_call_target(context.function_instance, expression, target);
    if matches!(ty, Type::Async(_)) && resolved_intrinsic(target).is_some() {
        let instance = IntrinsicFutureInstance {
            owner: context.function_instance.cloned(),
            expression,
        };
        let layout = context
            .async_frames
            .intrinsic(&instance)
            .expect("reachable intrinsic async calls have frame layouts");
        function
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::I32Const(
                context.gc.intrinsic_frame_tag(&instance) as i32,
            ));
        if layout.receiver.is_some() {
            compile_receiver(function, target, context);
        }
        for argument in args {
            if layout.arguments.contains_key(argument) {
                compile_expr(function, *argument, context);
            }
        }
        for (_, ty) in &layout.state {
            emit_default(function, *ty, context.gc);
        }
        if let Some((_, completion)) = layout.completion {
            emit_default(function, completion, context.gc);
        }
        function.instruction(&Instruction::StructNew(
            context.gc.intrinsic_frame_index(&instance),
        ));
        return;
    }
    if let Some(intrinsic @ (IntrinsicId::NumericMin | IntrinsicId::NumericMax)) =
        resolved_intrinsic(target)
    {
        let temps = &context.matches.intrinsic_temps[&expression];
        let receiver_type = compile_receiver(function, target, context);
        function.instruction(&Instruction::LocalSet(temps[0]));
        for (argument, local) in args.iter().zip(&temps[1..]) {
            compile_expr(function, *argument, context);
            function.instruction(&Instruction::LocalSet(*local));
        }
        emit_numeric_method(function, intrinsic, receiver_type, temps, context.gc);
        return;
    }
    let intrinsic = resolved_intrinsic(target);
    if let Some(contract) = intrinsic.and_then(crate::intrinsic_registry::provider_read_contract) {
        compile_provider_read(function, expression, target, args[0], contract, context);
        return;
    }
    match intrinsic {
        None => match target {
            wasm_ir::CallTarget::UserMethod {
                function: target_function,
                ..
            } => {
                compile_receiver(function, target, context);
                for argument in args {
                    compile_user_argument(function, *argument, context);
                }
                let target_function = context.called_instance(target_function);
                function.instruction(&Instruction::Call(context.functions[&target_function].call));
            }
            wasm_ir::CallTarget::UserFunction { function: target } => {
                for argument in args {
                    compile_user_argument(function, *argument, context);
                }
                let target = context.called_instance(target);
                function.instruction(&Instruction::Call(context.functions[&target].call));
            }
            wasm_ir::CallTarget::LibraryOverload { receiver, .. } => {
                if receiver.is_some() {
                    compile_receiver(function, target, context);
                }
                for argument in args {
                    compile_user_argument(function, *argument, context);
                }
                let target = wasm_ir::resolve_library_overload(
                    target,
                    context.function_instance,
                    context.semantics,
                    context.wasm_ir.standard_library(),
                )
                .expect("library overload calls resolve a hidden function");
                function.instruction(&Instruction::Call(context.functions[&target].call));
            }
            wasm_ir::CallTarget::DefaultDisplay { receiver_type, .. } => {
                compile_receiver(function, target, context);
                emit_display_value(
                    function,
                    context.ty(*receiver_type),
                    context.type_id(*receiver_type),
                    context,
                );
            }
            wasm_ir::CallTarget::Intrinsic { .. } => {
                unreachable!("standard-library implementations have intrinsic IDs")
            }
            wasm_ir::CallTarget::CapabilityRequirement { .. } => {
                unreachable!("reachable capability calls are statically dispatched")
            }
            wasm_ir::CallTarget::ResultError { .. } => {
                let Type::Result(result) = ty else {
                    unreachable!("Err constructors produce Result values")
                };
                emit_default(
                    function,
                    result_value_type(result, context.semantics),
                    context.gc,
                );
                function.instruction(&Instruction::I32Const(1));
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Result(result)),
                ));
            }
            wasm_ir::CallTarget::OptionSome { .. } => {
                let Type::Option(option) = ty else {
                    unreachable!("Some constructors produce Option values")
                };
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Option(option)),
                ));
            }
            wasm_ir::CallTarget::IteratorItem { .. } => {
                let Type::Application(step) = ty else {
                    unreachable!("Item constructors produce IteratorStep values")
                };
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Application(step)),
                ));
            }
            wasm_ir::CallTarget::ResultSuccess { .. } => {
                let Type::Result(result) = ty else {
                    unreachable!("Ok constructors produce Result values")
                };
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.standard_index(StdlibTypeId::String),
                    )))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Result(result)),
                    ));
            }
        },
        Some(builtin) => match builtin {
            IntrinsicId::SetNew => {
                let Type::Set(set) = ty else {
                    unreachable!("Set.new produces a Set value")
                };
                function.instruction(&Instruction::Call(
                    context.set_functions.function(set, builtin),
                ));
            }
            IntrinsicId::SetLength
            | IntrinsicId::SetContains
            | IntrinsicId::SetInsert
            | IntrinsicId::SetRemove
            | IntrinsicId::SetClear => {
                let receiver = compile_receiver(function, target, context);
                let Type::Set(set) = receiver else {
                    unreachable!("set methods have Set receivers")
                };
                for argument in args {
                    compile_expr(function, *argument, context);
                }
                function.instruction(&Instruction::Call(
                    context.set_functions.function(set, builtin),
                ));
            }
            IntrinsicId::Print => {
                compile_as_string(function, args[0], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::PrintString),
                ));
            }
            IntrinsicId::IntegerToStringRadix => {
                let receiver_type = compile_receiver(function, target, context);
                emit_integer_to_i64(function, receiver_type);
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::I32Const(receiver_type.is_signed() as i32))
                    .instruction(&Instruction::Call(
                        context.runtime_helpers.function(RuntimeHelperId::FormatI64),
                    ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "integer radix must be between 2 and 36",
                    context,
                );
            }
            IntrinsicId::StringLength => {
                compile_receiver(function, target, context);
                function
                    .instruction(&Instruction::RefAsNonNull)
                    .instruction(&Instruction::ArrayLen);
            }
            IntrinsicId::StringContains
            | IntrinsicId::StringStartsWith
            | IntrinsicId::StringEndsWith
            | IntrinsicId::StringEqualsIgnoreAsciiCase => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                let mode = match builtin {
                    IntrinsicId::StringContains => 0,
                    IntrinsicId::StringStartsWith => 1,
                    IntrinsicId::StringEndsWith => 2,
                    IntrinsicId::StringEqualsIgnoreAsciiCase => 3,
                    _ => unreachable!(),
                };
                function
                    .instruction(&Instruction::I32Const(mode))
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::StringMatch),
                    ));
            }
            IntrinsicId::StringIndexOf | IntrinsicId::StringLastIndexOf => {
                let found = context.matches.intrinsic_temps[&expression][0];
                let Type::Option(option) = ty else {
                    unreachable!("string position methods return the declared optional u32")
                };
                let option_type = context.gc.val_type(Type::Option(option));
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                if builtin == IntrinsicId::StringIndexOf {
                    function.instruction(&Instruction::I32Const(0));
                }
                let helper = if builtin == IntrinsicId::StringIndexOf {
                    RuntimeHelperId::StringFind
                } else {
                    RuntimeHelperId::StringRFind
                };
                function
                    .instruction(&Instruction::Call(context.runtime_helpers.function(helper)))
                    .instruction(&Instruction::LocalTee(found))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::I32LtS)
                    .instruction(&Instruction::If(BlockType::Result(option_type)))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.index(Type::Option(option)),
                    )))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::LocalGet(found))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Option(option)),
                    ))
                    .instruction(&Instruction::End);
            }
            IntrinsicId::StringToAsciiLowerCase | IntrinsicId::StringToAsciiUpperCase => {
                compile_receiver(function, target, context);
                function
                    .instruction(&Instruction::I32Const(
                        (builtin == IntrinsicId::StringToAsciiUpperCase) as i32,
                    ))
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::StringAsciiCase),
                    ));
            }
            IntrinsicId::StringTrimAsciiWhitespace => {
                compile_receiver(function, target, context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::StringTrimAsciiWhitespace),
                ));
            }
            IntrinsicId::StringPadStart | IntrinsicId::StringPadEnd => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function
                    .instruction(&Instruction::I32Const(
                        (builtin == IntrinsicId::StringPadEnd) as i32,
                    ))
                    .instruction(&Instruction::Call(
                        context.runtime_helpers.function(RuntimeHelperId::StringPad),
                    ));
            }
            IntrinsicId::StringReplaceAll => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::StringReplaceAll),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "string replacement search is empty or its result is too large",
                    context,
                );
            }
            IntrinsicId::StringSplit => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::StringSplit),
                ));
                let array = context
                    .arrays
                    .iter()
                    .find(|array| {
                        try_array_element_type(array.id, context.semantics)
                            == Some(Type::Standard(StdlibTypeId::String))
                    })
                    .expect("String.split has a reachable [String] result layout")
                    .id;
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Array(array),
                    Instruction::RefIsNull,
                    "string split delimiter is empty or its result is too large",
                    context,
                );
            }
            IntrinsicId::StringParse => {
                let Type::Result(result) = ty else {
                    unreachable!("String.parse produces a Result value")
                };
                let parsed_type = result_value_type(result, context.semantics);
                compile_receiver(function, target, context);
                match parsed_type {
                    Type::I8
                    | Type::U8
                    | Type::I16
                    | Type::U16
                    | Type::I32
                    | Type::U32
                    | Type::I64
                    | Type::U64
                    | Type::Address => {
                        let (allow_negative, positive_limit, negative_limit) =
                            integer_parse_limits(parsed_type);
                        function
                            .instruction(&Instruction::I32Const(i32::from(allow_negative)))
                            .instruction(&Instruction::I64Const(positive_limit))
                            .instruction(&Instruction::I64Const(negative_limit))
                            .instruction(&Instruction::Call(
                                context
                                    .runtime_helpers
                                    .function(RuntimeHelperId::StringParseInteger),
                            ));
                        if matches!(
                            parsed_type,
                            Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32
                        ) {
                            function.instruction(&Instruction::I32WrapI64);
                            emit_narrow_i32(function, parsed_type);
                        }
                    }
                    Type::F32 | Type::F64 => {
                        function
                            .instruction(&Instruction::I32Const(i32::from(
                                parsed_type == Type::F32,
                            )))
                            .instruction(&Instruction::Call(
                                context
                                    .runtime_helpers
                                    .function(RuntimeHelperId::StringParseFloat),
                            ));
                        if parsed_type == Type::F32 {
                            function.instruction(&Instruction::F32DemoteF64);
                        }
                    }
                    _ => unreachable!("Numeric parsing requires a concrete numeric type"),
                }
                emit_status_result(
                    function,
                    expression,
                    parsed_type,
                    "string is not valid decimal text for the inferred numeric type",
                    context,
                );
            }
            IntrinsicId::StringByteAt | IntrinsicId::StringCharAt => {
                let (mode, value_type, message) = match builtin {
                    IntrinsicId::StringByteAt => {
                        (0, Type::U8, "string byte index is out of bounds")
                    }
                    IntrinsicId::StringCharAt => (
                        1,
                        Type::Char,
                        "string byte index is out of bounds or not a UTF-8 boundary",
                    ),
                    _ => unreachable!(),
                };
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::I32Const(mode))
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::StringInspect),
                    ));
                emit_status_result(function, expression, value_type, message, context);
            }
            IntrinsicId::StringSlice => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::StringSlice),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "string slice offsets are out of bounds or not UTF-8 boundaries",
                    context,
                );
            }
            IntrinsicId::StringConcat => {
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.standard_index(StdlibTypeId::String),
                    )))
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::JoinStrings),
                    ));
            }
            IntrinsicId::StringJoin => {
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::JoinStrings),
                ));
            }
            IntrinsicId::TimerSetVariable => {
                compile_expr(function, args[0], context);
                compile_as_string(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::TimerSetVariable),
                ));
            }
            IntrinsicId::SettingsEnabled | IntrinsicId::SettingsContains => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                let helper = if intrinsic == Some(IntrinsicId::SettingsEnabled) {
                    RuntimeHelperId::SettingsEnabled
                } else {
                    RuntimeHelperId::SettingsContains
                };
                function.instruction(&Instruction::Call(context.runtime_helpers.function(helper)));
            }
            IntrinsicId::TimerState => {
                let host_state = context.matches.intrinsic_temps[&expression][0];
                let Type::Standard(StdlibTypeId::TimerState) = ty else {
                    unreachable!("timer.state returns the declared standard enum")
                };
                let timer_state = context.standard_library.type_decl(StdlibTypeId::TimerState);
                function
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::TimerGetState),
                    ))
                    .instruction(&Instruction::LocalTee(host_state))
                    .instruction(&Instruction::I32Const(4))
                    .instruction(&Instruction::I32LtU)
                    .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
                    .instruction(&Instruction::LocalGet(host_state))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::I32Const(4))
                    .instruction(&Instruction::End);
                for _ in context.standard_library.variants_of(timer_state.id) {
                    function.instruction(&Instruction::I32Const(0));
                }
                function.instruction(&Instruction::StructNew(
                    context.gc.index(Type::Standard(StdlibTypeId::TimerState)),
                ));
            }
            IntrinsicId::TimerCurrentSplitIndex => {
                let host_index = context.matches.intrinsic_temps[&expression][0];
                let Type::Option(option) = ty else {
                    unreachable!("timer.currentSplitIndex returns the declared optional u64")
                };
                let option_type = context.gc.val_type(Type::Option(option));
                function
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::TimerCurrentSplitIndex),
                    ))
                    .instruction(&Instruction::LocalTee(host_index))
                    .instruction(&Instruction::I64Const(0))
                    .instruction(&Instruction::I64LtS)
                    .instruction(&Instruction::If(BlockType::Result(option_type)))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.index(Type::Option(option)),
                    )))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::LocalGet(host_index))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Option(option)),
                    ))
                    .instruction(&Instruction::End);
            }
            IntrinsicId::TimerSegmentWasSplit => {
                let host_value = context.matches.intrinsic_temps[&expression][0];
                let Type::Option(option) = ty else {
                    unreachable!("timer.segmentWasSplit returns the declared optional bool")
                };
                let option_type = context.gc.val_type(Type::Option(option));
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::TimerSegmentWasSplit),
                    ))
                    .instruction(&Instruction::LocalTee(host_value))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::I32LtS)
                    .instruction(&Instruction::If(BlockType::Result(option_type)))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.index(Type::Option(option)),
                    )))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::LocalGet(host_value))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::I32Ne)
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Option(option)),
                    ))
                    .instruction(&Instruction::End);
            }
            IntrinsicId::TimerSkipSplit => {
                function.instruction(&Instruction::Call(
                    context.abi.function(AbiImportId::TimerSkipSplit),
                ));
            }
            IntrinsicId::TimerUndoSplit => {
                function.instruction(&Instruction::Call(
                    context.abi.function(AbiImportId::TimerUndoSplit),
                ));
            }
            IntrinsicId::TimerPauseGameTime => {
                function.instruction(&Instruction::Call(
                    context.abi.function(AbiImportId::TimerPauseGameTime),
                ));
            }
            IntrinsicId::TimerResumeGameTime => {
                function.instruction(&Instruction::Call(
                    context.abi.function(AbiImportId::TimerResumeGameTime),
                ));
            }
            IntrinsicId::RuntimeSetTickRate => {
                // The public API accepts every Numeric representation while the
                // host ABI deliberately stays one stable f64 function.
                emit_cast(function, args[0], Type::F64, context);
                function.instruction(&Instruction::Call(
                    context.abi.function(AbiImportId::RuntimeSetTickRate),
                ));
            }
            IntrinsicId::InstantNow => {
                let destination = context.abi_read.destination(8);
                function
                    // WASI clock ID 1 is the monotonic clock. A precision of
                    // one requests the finest available nanosecond reading.
                    .instruction(&Instruction::I32Const(1))
                    .instruction(&Instruction::I64Const(1))
                    .instruction(&Instruction::I32Const(destination))
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::WasiClockTimeGet),
                    ))
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::Unreachable)
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::I32Const(destination))
                    .instruction(&Instruction::I64Load(memarg()))
                    .instruction(&Instruction::StructNew(
                        context.gc.standard_index(StdlibTypeId::Instant),
                    ));
            }
            IntrinsicId::ProcessName => {
                // Evaluate the written receiver exactly once even though the
                // matched source name is attachment metadata rather than part
                // of the scalar process handle.
                compile_receiver(function, target, context);
                function.instruction(&Instruction::Drop);
                let provider = context
                    .semantics
                    .state_provider()
                    .map(|provider| context.standard_library.state_provider(provider))
                    .expect("checked states resolve a process provider");
                let names = match provider.processes {
                    crate::stdlib::StateProviderProcesses::Declared(processes) => {
                        processes.to_vec()
                    }
                    crate::stdlib::StateProviderProcesses::SourceState => {
                        context.state.processes.iter().map(String::as_str).collect()
                    }
                };
                debug_assert!(!names.is_empty());
                let string_type = context.gc.val_type(Type::Standard(StdlibTypeId::String));
                for (index, name) in names.iter().enumerate() {
                    function
                        .instruction(&Instruction::GlobalGet(
                            context.runtime_globals.process_name,
                        ))
                        .instruction(&Instruction::I32Const(index as i32))
                        .instruction(&Instruction::I32Eq)
                        .instruction(&Instruction::If(BlockType::Result(string_type)));
                    emit_string_literal(function, name, context.gc);
                    function.instruction(&Instruction::Else);
                }
                function.instruction(&Instruction::Unreachable);
                for _ in names {
                    function.instruction(&Instruction::End);
                }
            }
            IntrinsicId::ProcessMemoryRanges => {
                emit_process_memory_ranges(function, expression, target, context);
            }
            IntrinsicId::ProcessLoadedModule => {
                let Type::Option(option) = context.expression_type(expression) else {
                    unreachable!("process.loadedModule returns its declared optional module")
                };
                let module = context.matches.intrinsic_temps[&expression][0];
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::LoadedModule),
                    ))
                    .instruction(&Instruction::LocalTee(module))
                    .instruction(&Instruction::RefIsNull)
                    .instruction(&Instruction::If(BlockType::Result(
                        context.gc.val_type(Type::Option(option)),
                    )))
                    .instruction(&Instruction::RefNull(HeapType::Concrete(
                        context.gc.index(Type::Option(option)),
                    )))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::LocalGet(module))
                    .instruction(&Instruction::StructNew(
                        context.gc.index(Type::Option(option)),
                    ))
                    .instruction(&Instruction::End);
            }
            IntrinsicId::NextTick
            | IntrinsicId::ProcessClosed
            | IntrinsicId::ProcessMainModule
            | IntrinsicId::ProcessModule
            | IntrinsicId::ProcessFindMemoryRange => {
                unreachable!("suspending functions are lowered as awaits")
            }
            IntrinsicId::ProcessRead => {
                let read_type = match target {
                    wasm_ir::CallTarget::Intrinsic { type_arguments, .. } => {
                        context.type_id(type_arguments[0])
                    }
                    _ => unreachable!("process.read must resolve to its standard-library item"),
                };
                let Type::Result(result_type) = context.expression_type(expression) else {
                    unreachable!("synchronous process reads produce Result values")
                };
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                emit_process_read_from_stack(
                    function,
                    read_type,
                    result_type,
                    "process read failed",
                    context,
                    MemoryByteOrder::Little,
                );
            }
            IntrinsicId::ProcessFollow => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::FollowAddress),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Address,
                    Instruction::I64Eqz,
                    "pointer path could not be followed",
                    context,
                );
            }
            IntrinsicId::ProcessScan
            | IntrinsicId::ModuleScanRelative32Target
            | IntrinsicId::ProcessScanMemory
            | IntrinsicId::ProcessScanMemoryAny => {
                unreachable!("process.scan is lowered as an await")
            }
            IntrinsicId::ProcessReadRelative32 => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadRelative32),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Address,
                    Instruction::I64Eqz,
                    "relative address could not be read",
                    context,
                );
            }
            IntrinsicId::ProcessReadUtf8 => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadUtf8String),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "UTF-8 string could not be read",
                    context,
                );
            }
            IntrinsicId::ProcessReadUtf16Le => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadUtf16LeString),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "UTF-16LE string could not be read",
                    context,
                );
            }
            IntrinsicId::ProcessReadManagedString => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                compile_expr(function, args[1], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadManagedString),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "managed string could not be read",
                    context,
                );
            }
            IntrinsicId::ProcessPath => {
                compile_receiver(function, target, context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ProcessPath),
                ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "process path is unavailable",
                    context,
                );
            }
            IntrinsicId::ModulePath => {
                function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
                compile_receiver(function, target, context);
                function
                    .instruction(&Instruction::RefAsNonNull)
                    .instruction(&Instruction::StructGet {
                        struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                        field_index: context.gc.standard_field_index(StdlibFieldId::ModuleName),
                    })
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::ModulePath),
                    ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "module path is unavailable",
                    context,
                );
            }
            IntrinsicId::RuntimeOperatingSystem | IntrinsicId::RuntimeArchitecture => {
                let helper = if intrinsic == Some(IntrinsicId::RuntimeOperatingSystem) {
                    RuntimeHelperId::RuntimeOperatingSystem
                } else {
                    RuntimeHelperId::RuntimeArchitecture
                };
                function.instruction(&Instruction::Call(context.runtime_helpers.function(helper)));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::String),
                    Instruction::RefIsNull,
                    "runtime metadata is unavailable",
                    context,
                );
            }
            IntrinsicId::GBAEmulatorRead
            | IntrinsicId::GCNEmulatorRead
            | IntrinsicId::WiiEmulatorRead
            | IntrinsicId::Ps2EmulatorRead
            | IntrinsicId::Ps1EmulatorRead
            | IntrinsicId::SmsEmulatorRead
            | IntrinsicId::GenesisEmulatorRead => {
                unreachable!("provider reads are lowered before ordinary intrinsics")
            }
            IntrinsicId::NumericMin | IntrinsicId::NumericMax => {
                unreachable!("numeric intrinsics are lowered before ordinary calls")
            }
            IntrinsicId::EquatableEquals | IntrinsicId::EquatableNotEquals => {
                compile_intrinsic_equality(
                    function,
                    target,
                    args[0],
                    builtin == IntrinsicId::EquatableEquals,
                    context,
                );
            }
            IntrinsicId::BoolNot => {
                compile_receiver(function, target, context);
                function.instruction(&Instruction::I32Eqz);
            }
            IntrinsicId::IntegerBitNot => {
                let receiver = compile_receiver(function, target, context);
                match receiver {
                    Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
                        function
                            .instruction(&Instruction::I32Const(-1))
                            .instruction(&Instruction::I32Xor);
                    }
                    Type::I64 | Type::U64 | Type::Address => {
                        function
                            .instruction(&Instruction::I64Const(-1))
                            .instruction(&Instruction::I64Xor);
                    }
                    _ => unreachable!("bitwise complement requires an integer receiver"),
                }
                emit_narrow_integer_result(function, receiver);
            }
            IntrinsicId::NumericSwapBytes => {
                let receiver = compile_receiver(function, target, context);
                let value = context.matches.intrinsic_temps[&expression][0];
                function.instruction(&Instruction::LocalSet(value));
                match receiver {
                    Type::I8 | Type::U8 => {
                        function.instruction(&Instruction::LocalGet(value));
                    }
                    Type::I16 | Type::U16 => {
                        function
                            .instruction(&Instruction::LocalGet(value))
                            .instruction(&Instruction::I32Const(8))
                            .instruction(&Instruction::I32Shl)
                            .instruction(&Instruction::LocalGet(value))
                            .instruction(&Instruction::I32Const(8))
                            .instruction(&Instruction::I32ShrU)
                            .instruction(&Instruction::I32Const(0xff))
                            .instruction(&Instruction::I32And)
                            .instruction(&Instruction::I32Or);
                        emit_narrow_integer_result(function, receiver);
                    }
                    Type::I32 | Type::U32 => {
                        emit_swap_bytes_i32(function, value, false);
                    }
                    Type::I64 | Type::U64 => {
                        emit_swap_bytes_i64(function, value, false);
                    }
                    Type::F32 => emit_swap_bytes_i32(function, value, true),
                    Type::F64 => emit_swap_bytes_i64(function, value, true),
                    _ => unreachable!("byte swapping requires a numeric receiver"),
                }
            }
            IntrinsicId::SignedNegate => {
                let receiver = compile_receiver(function, target, context);
                match receiver {
                    Type::I8 | Type::I16 | Type::I32 => {
                        // The receiver is already on the stack, so multiply by
                        // negative one rather than reversing subtraction order.
                        function
                            .instruction(&Instruction::I32Const(-1))
                            .instruction(&Instruction::I32Mul);
                        emit_narrow_integer_result(function, receiver);
                    }
                    Type::I64 => {
                        function
                            .instruction(&Instruction::I64Const(-1))
                            .instruction(&Instruction::I64Mul);
                    }
                    Type::F32 => {
                        function.instruction(&Instruction::F32Neg);
                    }
                    Type::F64 => {
                        function.instruction(&Instruction::F64Neg);
                    }
                    _ => unreachable!("signed negation requires a signed numeric receiver"),
                }
            }
            IntrinsicId::NumericAdd
            | IntrinsicId::NumericSubtract
            | IntrinsicId::NumericMultiply
            | IntrinsicId::NumericDivide
            | IntrinsicId::IntegerRemainder
            | IntrinsicId::IntegerBitOr
            | IntrinsicId::IntegerBitXor
            | IntrinsicId::IntegerBitAnd
            | IntrinsicId::IntegerShiftLeft
            | IntrinsicId::IntegerShiftRight => {
                let receiver = compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                emit_binary_instruction(
                    function,
                    intrinsic_binary_op(builtin).expect("matched primitive binary intrinsic"),
                    receiver,
                );
                emit_narrow_integer_result(function, receiver);
            }
            IntrinsicId::FloatSqrt
            | IntrinsicId::FloatTruncate
            | IntrinsicId::FloatFloor
            | IntrinsicId::FloatCeil
            | IntrinsicId::FloatRound => {
                let receiver = compile_receiver(function, target, context);
                function.instruction(&match (receiver, builtin) {
                    (Type::F32, IntrinsicId::FloatSqrt) => Instruction::F32Sqrt,
                    (Type::F32, IntrinsicId::FloatTruncate) => Instruction::F32Trunc,
                    (Type::F32, IntrinsicId::FloatFloor) => Instruction::F32Floor,
                    (Type::F32, IntrinsicId::FloatCeil) => Instruction::F32Ceil,
                    (Type::F32, IntrinsicId::FloatRound) => Instruction::F32Nearest,
                    (Type::F64, IntrinsicId::FloatSqrt) => Instruction::F64Sqrt,
                    (Type::F64, IntrinsicId::FloatTruncate) => Instruction::F64Trunc,
                    (Type::F64, IntrinsicId::FloatFloor) => Instruction::F64Floor,
                    (Type::F64, IntrinsicId::FloatCeil) => Instruction::F64Ceil,
                    (Type::F64, IntrinsicId::FloatRound) => Instruction::F64Nearest,
                    _ => unreachable!("float intrinsics require an f32 or f64 receiver"),
                });
            }
            IntrinsicId::F32FromBits => {
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::F32ReinterpretI32);
            }
            IntrinsicId::F32ToBits => {
                compile_receiver(function, target, context);
                function.instruction(&Instruction::I32ReinterpretF32);
            }
            IntrinsicId::F64FromBits => {
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::F64ReinterpretI64);
            }
            IntrinsicId::F64ToBits => {
                compile_receiver(function, target, context);
                function.instruction(&Instruction::I64ReinterpretF64);
            }
            IntrinsicId::AddressAdd => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::I64Add);
            }
            IntrinsicId::ArrayPush => {
                let receiver_type = compile_receiver(function, target, context);
                let Type::Array(array_id) = receiver_type else {
                    unreachable!("Array.push has an array receiver");
                };
                for argument in args {
                    compile_expr(function, *argument, context);
                }
                function.instruction(&Instruction::Call(context.array_functions.push(array_id)));
            }
            IntrinsicId::ArrayClear => {
                let receiver_type = compile_receiver(function, target, context);
                let Type::Array(array_id) = receiver_type else {
                    unreachable!("Array.clear has an array receiver");
                };
                function.instruction(&Instruction::Call(context.array_functions.clear(array_id)));
            }
            IntrinsicId::ArrayRemoveAt => {
                let receiver_type = compile_receiver(function, target, context);
                let Type::Array(array_id) = receiver_type else {
                    unreachable!("Array.removeAt has an array receiver");
                };
                for argument in args {
                    compile_expr(function, *argument, context);
                }
                function.instruction(&Instruction::Call(
                    context.array_functions.remove_at(array_id),
                ));
            }
            IntrinsicId::ArrayLength | IntrinsicId::ArraySet => {
                let receiver_type = compile_receiver(function, target, context);
                let Type::Array(array_id) = receiver_type else {
                    unreachable!();
                };
                match builtin {
                    IntrinsicId::ArrayLength => {
                        super::array_value::emit_length(function, context.gc, array_id);
                    }
                    IntrinsicId::ArraySet => {
                        super::array_value::emit_backing(function, context.gc, array_id);
                        for argument in args {
                            compile_expr(function, *argument, context);
                        }
                        function.instruction(&Instruction::ArraySet(context.gc.index(
                            Type::ArrayStorage(super::array_value::storage_id(
                                array_id,
                                context.arrays,
                                context.semantics,
                            )),
                        )));
                    }
                    _ => unreachable!(),
                }
            }
            IntrinsicId::ArrayIterator
            | IntrinsicId::SetIterator
            | IntrinsicId::ExclusiveRangeIterator
            | IntrinsicId::InclusiveRangeIterator => {
                emit_iterator_constructor(function, expression, target, builtin, context);
            }
            IntrinsicId::ArrayIteratorNext
            | IntrinsicId::SetIteratorNext
            | IntrinsicId::ExclusiveRangeIteratorNext
            | IntrinsicId::InclusiveRangeIteratorNext => {
                emit_iterator_next(function, expression, target, builtin, context);
            }
            IntrinsicId::ModuleScan
            | IntrinsicId::ModuleScanAny
            | IntrinsicId::UnityModuleImage
            | IntrinsicId::UnityImageClass
            | IntrinsicId::UnityImageClassAny
            | IntrinsicId::UnityClassField
            | IntrinsicId::UnityClassFieldAny
            | IntrinsicId::UnityClassProbeFieldAny
            | IntrinsicId::UnityClassStaticTable
            | IntrinsicId::UnityClassStaticInstance => {
                unreachable!("suspending receiver methods are lowered by await")
            }
        },
    };
    if ty == Type::None && context.materialize_none {
        function.instruction(&Instruction::RefNull(HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::None,
        }));
    }
}

fn compile_user_argument(function: &mut Function, argument: ExprId, context: &ExprContext<'_>) {
    if context.expression_type(argument) == Type::None {
        compile_expr(function, argument, &context.erasing_none());
    } else {
        compile_expr(function, argument, context);
    }
}

fn emit_iterator_constructor(
    function: &mut Function,
    expression: ExprId,
    target: &wasm_ir::CallTarget,
    intrinsic: IntrinsicId,
    context: &ExprContext<'_>,
) {
    let Type::Application(cursor) = context.expression_type(expression) else {
        unreachable!("iterator constructors return a concrete cursor application")
    };
    let source = context.matches.intrinsic_temps[&expression][0];
    let receiver = compile_receiver(function, target, context);
    function.instruction(&Instruction::LocalSet(source));

    match intrinsic {
        IntrinsicId::ArrayIterator => {
            let Type::Array(array) = receiver else {
                unreachable!("array.iterator has an array receiver")
            };
            function.instruction(&Instruction::LocalGet(source));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::LocalGet(source));
            super::array_value::emit_version(function, context.gc, array);
        }
        IntrinsicId::SetIterator => {
            let Type::Set(set) = receiver else {
                unreachable!("set.iterator has a set receiver")
            };
            function
                .instruction(&Instruction::LocalGet(source))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::LocalGet(source))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.index(Type::Set(set)),
                    field_index: super::set_functions::VERSION_FIELD,
                });
        }
        IntrinsicId::ExclusiveRangeIterator | IntrinsicId::InclusiveRangeIterator => {
            let Type::Range(range) = receiver else {
                unreachable!("range.iterator has a range receiver")
            };
            let bound = range_bound_type(range, context.semantics);
            for field in [0, 1] {
                function
                    .instruction(&Instruction::LocalGet(source))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(function, context.gc.index(Type::Range(range)), field, bound);
            }
            if intrinsic == IntrinsicId::InclusiveRangeIterator {
                function.instruction(&Instruction::I32Const(0));
            }
        }
        _ => unreachable!("only iterable constructors reach iterator lowering"),
    }
    function.instruction(&Instruction::StructNew(
        context.gc.index(Type::Application(cursor)),
    ));
}

fn emit_iterator_next(
    function: &mut Function,
    expression: ExprId,
    target: &wasm_ir::CallTarget,
    intrinsic: IntrinsicId,
    context: &ExprContext<'_>,
) {
    let Type::Application(step) = context.expression_type(expression) else {
        unreachable!("iterator.next returns IteratorStep<T>")
    };
    let cursor = context.matches.intrinsic_temps[&expression][0];
    let Type::Application(cursor_type) = compile_receiver(function, target, context) else {
        unreachable!("iterator.next has a concrete cursor receiver")
    };
    function.instruction(&Instruction::LocalSet(cursor));

    match intrinsic {
        IntrinsicId::ArrayIteratorNext => {
            emit_array_iterator_next(function, cursor, cursor_type, step, context);
        }
        IntrinsicId::SetIteratorNext => {
            emit_set_iterator_next(function, cursor, cursor_type, step, context);
        }
        IntrinsicId::ExclusiveRangeIteratorNext => {
            emit_range_iterator_next(function, cursor, cursor_type, step, false, context)
        }
        IntrinsicId::InclusiveRangeIteratorNext => {
            emit_range_iterator_next(function, cursor, cursor_type, step, true, context)
        }
        _ => unreachable!("only iterator next intrinsics reach cursor lowering"),
    }
}

fn emit_array_iterator_next(
    function: &mut Function,
    cursor: u32,
    cursor_type: crate::ast::TypeApplicationId,
    step: crate::ast::TypeApplicationId,
    context: &ExprContext<'_>,
) {
    let element = application_type_argument(
        cursor_type,
        StdlibTypeConstructorId::ArrayIterator,
        context.semantics,
    );
    let array = context
        .arrays
        .iter()
        .find(|array| {
            array.length.is_none()
                && try_array_element_type(array.id, context.semantics) == Some(element)
        })
        .expect("ArrayIterator<T> has a materialized [T] source")
        .id;
    let cursor_index = context.gc.index(Type::Application(cursor_type));

    emit_cursor_field(function, cursor, cursor_index, 0, Type::Array(array));
    super::array_value::emit_version(function, context.gc, array);
    emit_cursor_field(function, cursor, cursor_index, 2, Type::U32);
    emit_iterator_mutation_check(function);

    emit_cursor_field(function, cursor, cursor_index, 1, Type::U32);
    emit_cursor_field(function, cursor, cursor_index, 0, Type::Array(array));
    super::array_value::emit_length(function, context.gc, array);
    function
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Application(step)),
        )));

    emit_cursor_field(function, cursor, cursor_index, 0, Type::Array(array));
    super::array_value::emit_backing(function, context.gc, array);
    emit_cursor_field(function, cursor, cursor_index, 1, Type::U32);
    emit_array_get(
        function,
        context
            .gc
            .index(Type::ArrayStorage(super::array_value::storage_id(
                array,
                context.arrays,
                context.semantics,
            ))),
        element,
        context.gc,
    );
    emit_iterator_item(function, step, context);
    emit_cursor_increment(function, cursor, cursor_index, 1, Type::U32);

    function.instruction(&Instruction::Else);
    emit_iterator_end(function, step, context);
    function.instruction(&Instruction::End);
}

fn emit_set_iterator_next(
    function: &mut Function,
    cursor: u32,
    cursor_type: crate::ast::TypeApplicationId,
    step: crate::ast::TypeApplicationId,
    context: &ExprContext<'_>,
) {
    let element = application_type_argument(
        cursor_type,
        StdlibTypeConstructorId::SetIterator,
        context.semantics,
    );
    let (set, backing) = context
        .semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            crate::types::TypeKind::Set {
                layout,
                element: candidate,
                backing,
            } if !matches!(
                context.semantics.types().kind(*candidate),
                crate::types::TypeKind::GenericParameter { .. }
            ) && context.ty(*candidate) == element =>
            {
                Some((*layout, *backing))
            }
            _ => None,
        })
        .expect("SetIterator<T> has a materialized Set<T> source");
    let cursor_index = context.gc.index(Type::Application(cursor_type));

    emit_cursor_field(function, cursor, cursor_index, 0, Type::Set(set));
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.index(Type::Set(set)),
            field_index: super::set_functions::VERSION_FIELD,
        });
    emit_cursor_field(function, cursor, cursor_index, 2, Type::U32);
    emit_iterator_mutation_check(function);

    emit_cursor_field(function, cursor, cursor_index, 1, Type::U32);
    emit_cursor_field(function, cursor, cursor_index, 0, Type::Set(set));
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.index(Type::Set(set)),
            field_index: super::set_functions::LENGTH_FIELD,
        })
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Application(step)),
        )));

    emit_cursor_field(function, cursor, cursor_index, 0, Type::Set(set));
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.index(Type::Set(set)),
            field_index: super::set_functions::BACKING_FIELD,
        })
        .instruction(&Instruction::RefAsNonNull);
    emit_cursor_field(function, cursor, cursor_index, 1, Type::U32);
    emit_array_get(
        function,
        context.gc.index(Type::ArrayStorage(backing)),
        element,
        context.gc,
    );
    emit_iterator_item(function, step, context);
    emit_cursor_increment(function, cursor, cursor_index, 1, Type::U32);

    function.instruction(&Instruction::Else);
    emit_iterator_end(function, step, context);
    function.instruction(&Instruction::End);
}

fn emit_range_iterator_next(
    function: &mut Function,
    cursor: u32,
    cursor_type: crate::ast::TypeApplicationId,
    step: crate::ast::TypeApplicationId,
    inclusive: bool,
    context: &ExprContext<'_>,
) {
    let constructor = if inclusive {
        StdlibTypeConstructorId::InclusiveRangeIterator
    } else {
        StdlibTypeConstructorId::ExclusiveRangeIterator
    };
    let bound = application_type_argument(cursor_type, constructor, context.semantics);
    let cursor_index = context.gc.index(Type::Application(cursor_type));

    if inclusive {
        emit_cursor_field(function, cursor, cursor_index, 2, Type::Bool);
        function.instruction(&Instruction::I32Eqz);
        emit_cursor_field(function, cursor, cursor_index, 0, bound);
        emit_cursor_field(function, cursor, cursor_index, 1, bound);
        function
            .instruction(&compare(bound, bound.is_signed(), Compare::Le))
            .instruction(&Instruction::I32And);
    } else {
        emit_cursor_field(function, cursor, cursor_index, 0, bound);
        emit_cursor_field(function, cursor, cursor_index, 1, bound);
        function.instruction(&compare(bound, bound.is_signed(), Compare::Lt));
    }
    function.instruction(&Instruction::If(BlockType::Result(
        context.gc.val_type(Type::Application(step)),
    )));

    emit_cursor_field(function, cursor, cursor_index, 0, bound);
    emit_iterator_item(function, step, context);
    if inclusive {
        emit_cursor_field(function, cursor, cursor_index, 0, bound);
        emit_cursor_field(function, cursor, cursor_index, 1, bound);
        emit_binary_instruction(function, BinaryOp::Eq, bound);
        function.instruction(&Instruction::If(BlockType::Empty));
        function
            .instruction(&Instruction::LocalGet(cursor))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::StructSet {
                struct_type_index: cursor_index,
                field_index: 2,
            })
            .instruction(&Instruction::Else);
        emit_cursor_increment(function, cursor, cursor_index, 0, bound);
        function.instruction(&Instruction::End);
    } else {
        emit_cursor_increment(function, cursor, cursor_index, 0, bound);
    }

    function.instruction(&Instruction::Else);
    emit_iterator_end(function, step, context);
    function.instruction(&Instruction::End);
}

fn emit_cursor_field(function: &mut Function, cursor: u32, cursor_type: u32, field: u32, ty: Type) {
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, cursor_type, field, ty);
}

fn emit_cursor_increment(
    function: &mut Function,
    cursor: u32,
    cursor_type: u32,
    field: u32,
    ty: Type,
) {
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::RefAsNonNull);
    emit_cursor_field(function, cursor, cursor_type, field, ty);
    emit_int(function, 1, ty);
    emit_binary_instruction(function, BinaryOp::Add, ty);
    emit_narrow_integer_result(function, ty);
    function.instruction(&Instruction::StructSet {
        struct_type_index: cursor_type,
        field_index: field,
    });
}

fn emit_iterator_mutation_check(function: &mut Function) {
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
}

fn emit_iterator_item(
    function: &mut Function,
    step: crate::ast::TypeApplicationId,
    context: &ExprContext<'_>,
) {
    function.instruction(&Instruction::StructNew(
        context.gc.index(Type::Application(step)),
    ));
}

fn emit_iterator_end(
    function: &mut Function,
    step: crate::ast::TypeApplicationId,
    context: &ExprContext<'_>,
) {
    function.instruction(&Instruction::RefNull(HeapType::Concrete(
        context.gc.index(Type::Application(step)),
    )));
}

fn compile_return_expression(
    function: &mut Function,
    value: Option<ExprId>,
    context: &ExprContext<'_>,
) {
    match context.bare_return {
        BareReturn::AsyncFuture { frame, completion } => {
            if let Some(value) = value {
                if let Some((field, _)) = completion {
                    frame.emit(function);
                    compile_expr(function, value, context);
                    function.instruction(&Instruction::StructSet {
                        struct_type_index: frame.struct_type,
                        field_index: field,
                    });
                } else {
                    compile_expr(function, value, &context.erasing_none());
                }
            }
            frame.emit(function);
            function
                .instruction(&Instruction::I32Const(-1))
                .instruction(&Instruction::StructSet {
                    struct_type_index: frame.struct_type,
                    field_index: 0,
                });
            function.instruction(&Instruction::I32Const(1));
        }
        BareReturn::AsyncAttach { result_global } => {
            if let Some(global) = result_global {
                compile_expr(
                    function,
                    value.expect("layout selection returns a typed layout"),
                    context,
                );
                function.instruction(&Instruction::GlobalSet(global));
            } else {
                debug_assert!(value.is_none());
            }
            function.instruction(&Instruction::I32Const(1));
        }
        BareReturn::None => {
            if let Some(value) = value {
                let return_context = if context.expression_type(value) == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, value, &return_context);
            }
        }
        BareReturn::Action(action) => {
            if let Some(value) = value {
                compile_expr(function, value, context);
            } else {
                emit_action_default(function, action, context.gc);
            }
        }
    }
    function.instruction(&Instruction::Return);
}

fn emit_numeric_method(
    function: &mut Function,
    intrinsic: IntrinsicId,
    ty: Type,
    temps: &[u32],
    gc: &GcLayout,
) {
    if matches!(ty, Type::F32 | Type::F64) {
        function
            .instruction(&Instruction::LocalGet(temps[0]))
            .instruction(&Instruction::LocalGet(temps[1]))
            .instruction(&match (ty, intrinsic) {
                (Type::F32, IntrinsicId::NumericMin) => Instruction::F32Min,
                (Type::F32, IntrinsicId::NumericMax) => Instruction::F32Max,
                (Type::F64, IntrinsicId::NumericMin) => Instruction::F64Min,
                (Type::F64, IntrinsicId::NumericMax) => Instruction::F64Max,
                _ => unreachable!(),
            });
        return;
    }

    let result = BlockType::Result(gc.val_type(ty));
    function
        .instruction(&Instruction::LocalGet(temps[0]))
        .instruction(&Instruction::LocalGet(temps[1]))
        .instruction(&compare(
            ty,
            ty.is_signed(),
            if intrinsic == IntrinsicId::NumericMin {
                Compare::Lt
            } else {
                Compare::Gt
            },
        ))
        .instruction(&Instruction::If(result))
        .instruction(&Instruction::LocalGet(temps[0]))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(temps[1]))
        .instruction(&Instruction::End);
}

/// Copies the host's mapped-range metadata into a stable GC-owned array.
///
/// The array wrapper is not observable until this expression completes, so
/// its version and length metadata double as the loop cursor and current flag
/// word. Both fields are restored to their ordinary array meanings before the
/// value is returned. This keeps synchronous intrinsic scratch strongly typed
/// without adding a special heterogeneous-local policy for one operation.
fn emit_process_memory_ranges(
    function: &mut Function,
    expression: ExprId,
    target: &wasm_ir::CallTarget,
    context: &ExprContext<'_>,
) {
    let Type::Array(array) = context.expression_type(expression) else {
        unreachable!("process.memoryRanges returns its declared array type")
    };
    let output = context.matches.intrinsic_temps[&expression][0];
    let storage = super::array_value::storage_id(array, context.arrays, context.semantics);
    let storage_type = context.gc.index(Type::ArrayStorage(storage));
    let wrapper_type = context.gc.index(Type::Array(array));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            context
                .abi
                .function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::I32Const(0));
    super::array_value::emit_wrap_loaded(function, wrapper_type);
    function
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);
    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_backing(function, context.gc, array);
    function
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));

    // Preserve the current host flag word in the otherwise hidden logical
    // length field while the range record is assembled.
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull);
    compile_receiver(function, target, context);
    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            context
                .abi
                .function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::StructSet {
            struct_type_index: wrapper_type,
            field_index: super::array_value::LENGTH_FIELD,
        });

    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_backing(function, context.gc, array);
    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);

    compile_receiver(function, target, context);
    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            context
                .abi
                .function(AbiImportId::ProcessGetMemoryRangeAddress),
        ));
    compile_receiver(function, target, context);
    function.instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            context.abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ));
    for mask in [2, 4, 8] {
        function
            .instruction(&Instruction::LocalGet(output))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: wrapper_type,
                field_index: super::array_value::LENGTH_FIELD,
            })
            .instruction(&Instruction::I32Const(mask))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::I32Eqz);
    }
    function
        .instruction(&Instruction::StructNew(
            context.gc.standard_index(StdlibTypeId::MemoryRange),
        ))
        .instruction(&Instruction::ArraySet(storage_type))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output));
    super::array_value::emit_version(function, context.gc, array);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: wrapper_type,
            field_index: super::array_value::VERSION_FIELD,
        })
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    // Publish the actual logical length and reset the structural version that
    // was temporarily used as the cursor.
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output));
    super::array_value::emit_backing(function, context.gc, array);
    function
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::StructSet {
            struct_type_index: wrapper_type,
            field_index: super::array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::StructSet {
            struct_type_index: wrapper_type,
            field_index: super::array_value::VERSION_FIELD,
        })
        .instruction(&Instruction::LocalGet(output));
}

fn emit_process_read_from_stack(
    function: &mut Function,
    ty: TypeId,
    result_type: ResultTypeId,
    error: &str,
    context: &ExprContext<'_>,
    byte_order: MemoryByteOrder,
) {
    let ty = context.type_id(ty);
    let physical_type = semantic_type(ty, context.semantics);
    let size = context
        .memory
        .layout(ty, context.semantics)
        .expect("checked process reads are MemoryReadable")
        .size();
    function
        .instruction(&Instruction::I32Const(context.abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(
            context.abi.function(AbiImportId::ProcessRead),
        ))
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Result(result_type)),
        )));
    emit_memory_value(
        function,
        ty,
        context.abi_read,
        0,
        context.memory,
        context.semantics,
        context.gc,
        byte_order,
    );
    emit_result_success(function, result_type, context.gc);
    function.instruction(&Instruction::Else);
    emit_result_error(function, result_type, physical_type, error, context.gc);
    function.instruction(&Instruction::End);
}

/// Converts a helper's zero/null failure sentinel into the language's real
/// `T!` representation. The helper value is evaluated once and retained in a
/// planned scratch local for the success branch.
fn emit_sentinel_result(
    function: &mut Function,
    expression: ExprId,
    value_type: Type,
    failure_test: Instruction<'_>,
    message: &str,
    context: &ExprContext<'_>,
) {
    let Type::Result(result) = context.expression_type(expression) else {
        unreachable!("fallible process helpers produce Result values")
    };
    let value_local = context.matches.intrinsic_temps[&expression][0];
    function
        .instruction(&Instruction::LocalTee(value_local))
        .instruction(&failure_test)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Result(result)),
        )));
    emit_result_error(function, result, value_type, message, context.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(value_local));
    emit_result_success(function, result, context.gc);
    function.instruction(&Instruction::End);
}

/// Converts a helper's `(success, value)` multi-value result into `T!`. The
/// value is stored first, leaving the success flag on the operand stack.
fn emit_status_result(
    function: &mut Function,
    expression: ExprId,
    value_type: Type,
    message: &str,
    context: &ExprContext<'_>,
) {
    let Type::Result(result) = context.expression_type(expression) else {
        unreachable!("status-bearing helpers produce Result values")
    };
    let value_local = context.matches.intrinsic_temps[&expression][0];
    function
        .instruction(&Instruction::LocalSet(value_local))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Result(result)),
        )));
    emit_result_error(function, result, value_type, message, context.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(value_local));
    emit_result_success(function, result, context.gc);
    function.instruction(&Instruction::End);
}

fn integer_parse_limits(ty: Type) -> (bool, i64, i64) {
    match ty {
        Type::I8 => (true, i8::MAX as i64, 1_i64 << 7),
        Type::U8 => (false, u8::MAX as i64, u8::MAX as i64),
        Type::I16 => (true, i16::MAX as i64, 1_i64 << 15),
        Type::U16 => (false, u16::MAX as i64, u16::MAX as i64),
        Type::I32 => (true, i32::MAX as i64, 1_i64 << 31),
        Type::U32 => (false, u32::MAX as i64, u32::MAX as i64),
        Type::I64 => (true, i64::MAX, i64::MIN),
        Type::U64 | Type::Address => (false, -1, -1),
        _ => unreachable!("integer parse limits require an integer type"),
    }
}

fn emit_cast(function: &mut Function, expression: ExprId, target: Type, context: &ExprContext<'_>) {
    let source = context.expression_type(expression);
    compile_expr(function, expression, context);

    if target == Type::Standard(StdlibTypeId::String) {
        if source == target {
            return;
        }
        emit_display_value(
            function,
            source,
            context.expression_type_id(expression),
            context,
        );
        return;
    }

    let source_i32 = matches!(
        source,
        Type::Char | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32
    );
    let source_i64 = matches!(source, Type::I64 | Type::U64 | Type::Address);
    let target_i32 = matches!(
        target,
        Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32
    );
    let target_i64 = matches!(target, Type::I64 | Type::U64 | Type::Address);

    if source_i32 && target_i32 {
        emit_narrow_i32(function, target);
    } else if source_i32 && target_i64 {
        function.instruction(&if source.is_signed() {
            Instruction::I64ExtendI32S
        } else {
            Instruction::I64ExtendI32U
        });
    } else if source_i64 && target_i32 {
        function.instruction(&Instruction::I32WrapI64);
        emit_narrow_i32(function, target);
    } else if source_i64 && target_i64 {
        // All 64-bit integer and address casts have the same Wasm representation.
    } else if source_i32 && matches!(target, Type::F32 | Type::F64) {
        function.instruction(&match (target, source.is_signed()) {
            (Type::F32, true) => Instruction::F32ConvertI32S,
            (Type::F32, false) => Instruction::F32ConvertI32U,
            (Type::F64, true) => Instruction::F64ConvertI32S,
            (Type::F64, false) => Instruction::F64ConvertI32U,
            _ => unreachable!(),
        });
    } else if source_i64 && matches!(target, Type::F32 | Type::F64) {
        function.instruction(&match (target, source.is_signed()) {
            (Type::F32, true) => Instruction::F32ConvertI64S,
            (Type::F32, false) => Instruction::F32ConvertI64U,
            (Type::F64, true) => Instruction::F64ConvertI64S,
            (Type::F64, false) => Instruction::F64ConvertI64U,
            _ => unreachable!(),
        });
    } else if matches!(source, Type::F32 | Type::F64) && target_i32 {
        function.instruction(&match (source, target.is_signed()) {
            (Type::F32, true) => Instruction::I32TruncSatF32S,
            (Type::F32, false) => Instruction::I32TruncSatF32U,
            (Type::F64, true) => Instruction::I32TruncSatF64S,
            (Type::F64, false) => Instruction::I32TruncSatF64U,
            _ => unreachable!(),
        });
        emit_narrow_i32(function, target);
    } else if matches!(source, Type::F32 | Type::F64) && target_i64 {
        function.instruction(&match (source, target.is_signed()) {
            (Type::F32, true) => Instruction::I64TruncSatF32S,
            (Type::F32, false) => Instruction::I64TruncSatF32U,
            (Type::F64, true) => Instruction::I64TruncSatF64S,
            (Type::F64, false) => Instruction::I64TruncSatF64U,
            _ => unreachable!(),
        });
    } else if source == Type::F32 && target == Type::F64 {
        function.instruction(&Instruction::F64PromoteF32);
    } else if source == Type::F64 && target == Type::F32 {
        function.instruction(&Instruction::F32DemoteF64);
    } else if source != target {
        unreachable!("type checking rejected unsupported cast `{source:?} as {target:?}`");
    }
}

/// Converts an already-emitted value through the same lazy Display plan used
/// by casts, interpolation, host output, and explicit `.toString()` calls.
fn emit_display_value(
    function: &mut Function,
    source: Type,
    source_type: TypeId,
    context: &ExprContext<'_>,
) {
    if source == Type::None {
        emit_string_literal(function, "None", context.gc);
        return;
    }
    if source == Type::Bool {
        function.instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
        emit_string_literal(function, "true", context.gc);
        function.instruction(&Instruction::Else);
        emit_string_literal(function, "false", context.gc);
        function.instruction(&Instruction::End);
        return;
    }
    if source == Type::Char {
        function.instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::FormatChar),
        ));
        return;
    }
    if let Some(display) = context.display_functions.custom.get(&source_type) {
        let display = context.called_instance(display);
        function.instruction(&Instruction::Call(context.functions[&display].call));
        return;
    }
    if let Some(debug) = context.display_functions.custom_debug.get(&source_type) {
        let debug = context.called_instance(debug);
        function.instruction(&Instruction::Call(context.functions[&debug].call));
        return;
    }
    if let Some(display) = context.display_functions.derived.get(&source_type) {
        function.instruction(&Instruction::Call(*display));
        return;
    }
    if source == Type::F32 {
        function.instruction(&Instruction::Call(
            context.runtime_helpers.function(RuntimeHelperId::FormatF32),
        ));
        return;
    }
    if source == Type::F64 {
        function.instruction(&Instruction::Call(
            context.runtime_helpers.function(RuntimeHelperId::FormatF64),
        ));
        return;
    }
    emit_integer_to_i64(function, source);
    function
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32Const(source.is_signed() as i32))
        .instruction(&Instruction::Call(
            context.runtime_helpers.function(RuntimeHelperId::FormatI64),
        ));
}

fn emit_integer_to_i64(function: &mut Function, source: Type) {
    if matches!(source, Type::I8 | Type::I16 | Type::I32) {
        function.instruction(&Instruction::I64ExtendI32S);
    } else if matches!(source, Type::U8 | Type::U16 | Type::U32) {
        function.instruction(&Instruction::I64ExtendI32U);
    } else {
        debug_assert!(matches!(source, Type::I64 | Type::U64 | Type::Address));
    }
}

fn compile_as_string(function: &mut Function, expression: ExprId, context: &ExprContext<'_>) {
    emit_cast(
        function,
        expression,
        Type::Standard(StdlibTypeId::String),
        context,
    );
}

fn emit_narrow_i32(function: &mut Function, target: Type) {
    match target {
        Type::I8 => {
            function.instruction(&Instruction::I32Extend8S);
        }
        Type::U8 => {
            function
                .instruction(&Instruction::I32Const(0xff))
                .instruction(&Instruction::I32And);
        }
        Type::I16 => {
            function.instruction(&Instruction::I32Extend16S);
        }
        Type::U16 => {
            function
                .instruction(&Instruction::I32Const(0xffff))
                .instruction(&Instruction::I32And);
        }
        Type::I32 | Type::U32 => {}
        _ => unreachable!(),
    }
}

fn emit_narrow_integer_result(function: &mut Function, ty: Type) {
    if matches!(ty, Type::I8 | Type::U8 | Type::I16 | Type::U16) {
        emit_narrow_i32(function, ty);
    }
}

fn emit_swap_bytes_i32(function: &mut Function, value: u32, float: bool) {
    function.instruction(&Instruction::LocalGet(value));
    if float {
        function.instruction(&Instruction::I32ReinterpretF32);
    }
    function
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::I32Rotl)
        .instruction(&Instruction::I32Const(0x00ff_00ff))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(value));
    if float {
        function.instruction(&Instruction::I32ReinterpretF32);
    }
    function
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::I32Rotr)
        .instruction(&Instruction::I32Const(0xff00_ff00u32 as i32))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or);
    if float {
        function.instruction(&Instruction::F32ReinterpretI32);
    }
}

fn emit_swap_bytes_i64(function: &mut Function, value: u32, float: bool) {
    // A single expression-typed local keeps evaluation one-shot for both i64
    // and f64. Each term moves one input byte directly to its final position,
    // avoiding an extra integer-typed temporary for floating-point receivers.
    let left = [
        (0x0000_0000_0000_00ffu64, 56),
        (0x0000_0000_0000_ff00u64, 40),
        (0x0000_0000_00ff_0000u64, 24),
        (0x0000_0000_ff00_0000u64, 8),
    ];
    let right = [
        (8, 0x0000_0000_ff00_0000u64),
        (24, 0x0000_0000_00ff_0000u64),
        (40, 0x0000_0000_0000_ff00u64),
        (56, 0x0000_0000_0000_00ffu64),
    ];

    for (term, (mask, shift)) in left.into_iter().enumerate() {
        function.instruction(&Instruction::LocalGet(value));
        if float {
            function.instruction(&Instruction::I64ReinterpretF64);
        }
        function
            .instruction(&Instruction::I64Const(mask as i64))
            .instruction(&Instruction::I64And)
            .instruction(&Instruction::I64Const(shift))
            .instruction(&Instruction::I64Shl);
        if term != 0 {
            function.instruction(&Instruction::I64Or);
        }
    }
    for (shift, mask) in right {
        function.instruction(&Instruction::LocalGet(value));
        if float {
            function.instruction(&Instruction::I64ReinterpretF64);
        }
        function
            .instruction(&Instruction::I64Const(shift))
            .instruction(&Instruction::I64ShrU)
            .instruction(&Instruction::I64Const(mask as i64))
            .instruction(&Instruction::I64And)
            .instruction(&Instruction::I64Or);
    }
    if float {
        function.instruction(&Instruction::F64ReinterpretI64);
    }
}

fn emit_binary(
    function: &mut Function,
    op: BinaryOp,
    left: ExprId,
    right: ExprId,
    context: &ExprContext<'_>,
) {
    let operand_type = context.expression_type(left);
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        unreachable!("checked equality lowers through the Equatable catalog intrinsic")
    }
    if matches!(op, BinaryOp::Or | BinaryOp::And) {
        compile_expr(function, left, context);
        function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        let nested_context = context.nested_loop_control(1);
        if op == BinaryOp::Or {
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::Else);
            compile_expr(function, right, &nested_context);
        } else {
            compile_expr(function, right, &nested_context);
            function
                .instruction(&Instruction::Else)
                .instruction(&Instruction::I32Const(0));
        }
        function.instruction(&Instruction::End);
        return;
    }

    compile_expr(function, left, context);
    compile_expr(function, right, context);
    emit_binary_instruction(function, op, operand_type);
    emit_narrow_integer_result(function, operand_type);
}

fn compile_provider_read(
    function: &mut Function,
    expression: ExprId,
    target: &wasm_ir::CallTarget,
    address_expression: ExprId,
    contract: crate::intrinsic_registry::ProviderReadContract,
    context: &ExprContext<'_>,
) {
    let read_type = match target {
        wasm_ir::CallTarget::Intrinsic { type_arguments, .. } => context.type_id(type_arguments[0]),
        _ => unreachable!("provider reads resolve to their standard-library method"),
    };
    let Type::Result(result_type) = context.expression_type(expression) else {
        unreachable!("provider reads produce Result values")
    };
    let status = context.matches.intrinsic_temps[&expression][0];
    function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    compile_receiver(function, target, context);
    compile_expr(function, address_expression, context);
    let size = context
        .memory
        .layout(read_type, context.semantics)
        .expect("checked provider reads are MemoryReadable")
        .size();
    function
        .instruction(&Instruction::I32Const(context.abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(
            context.runtime_helpers.function(contract.reader),
        ))
        .instruction(&Instruction::LocalTee(status))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Result(result_type)),
        )));
    emit_memory_value(
        function,
        read_type,
        context.abi_read,
        0,
        context.memory,
        context.semantics,
        context.gc,
        contract.byte_order.into(),
    );
    emit_result_success(function, result_type, context.gc);
    function.instruction(&Instruction::Else);
    function
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Result(
            context.gc.val_type(Type::Result(result_type)),
        )));
    emit_result_error(
        function,
        result_type,
        context.ty(read_type),
        contract.invalid_address,
        context.gc,
    );
    function.instruction(&Instruction::Else);
    emit_result_error(
        function,
        result_type,
        context.ty(read_type),
        contract.read_failure,
        context.gc,
    );
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
}

fn compile_intrinsic_equality(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    other: ExprId,
    equal: bool,
    context: &ExprContext<'_>,
) {
    let (_, operand_type) = resolved_receiver(target, context);
    if operand_type == Type::None {
        let operand_context = context.erasing_none();
        compile_receiver(function, target, &operand_context);
        compile_expr(function, other, &operand_context);
        function.instruction(&Instruction::I32Const(i32::from(equal)));
        return;
    }

    if matches!(operand_type, Type::Standard(_)) && operand_type.is_enum(context.standard_library) {
        compile_receiver(function, target, context);
        function.instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(function, context.gc.index(operand_type), 0, Type::I32);
        compile_expr(function, other, context);
        function.instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(function, context.gc.index(operand_type), 0, Type::I32);
        function.instruction(&Instruction::I32Eq);
    } else if matches!(
        operand_type,
        Type::Standard(_) | Type::Record(_) | Type::Enum(_) | Type::Option(_) | Type::Result(_)
    ) {
        compile_receiver(function, target, context);
        compile_expr(function, other, context);
        emit_value_equality(
            function,
            operand_type,
            context.equality_functions,
            context
                .runtime_helpers
                .optional_function(RuntimeHelperId::StringEquality)
                .unwrap_or(0),
        );
    } else {
        compile_receiver(function, target, context);
        compile_expr(function, other, context);
        emit_binary_instruction(function, BinaryOp::Eq, operand_type);
    }

    if !equal {
        function.instruction(&Instruction::I32Eqz);
    }
}

fn emit_binary_instruction(function: &mut Function, op: BinaryOp, ty: Type) {
    let signed = ty.is_signed();
    let i64 = matches!(ty, Type::I64 | Type::U64 | Type::Address);
    let instruction = match op {
        BinaryOp::Eq => match ty {
            Type::F32 => Instruction::F32Eq,
            Type::F64 => Instruction::F64Eq,
            _ if i64 => Instruction::I64Eq,
            _ => Instruction::I32Eq,
        },
        BinaryOp::Ne => match ty {
            Type::F32 => Instruction::F32Ne,
            Type::F64 => Instruction::F64Ne,
            _ if i64 => Instruction::I64Ne,
            _ => Instruction::I32Ne,
        },
        BinaryOp::Lt => compare(ty, signed, Compare::Lt),
        BinaryOp::Le => compare(ty, signed, Compare::Le),
        BinaryOp::Gt => compare(ty, signed, Compare::Gt),
        BinaryOp::Ge => compare(ty, signed, Compare::Ge),
        BinaryOp::Add => match ty {
            Type::F32 => Instruction::F32Add,
            Type::F64 => Instruction::F64Add,
            _ if i64 => Instruction::I64Add,
            _ => Instruction::I32Add,
        },
        BinaryOp::Sub => match ty {
            Type::F32 => Instruction::F32Sub,
            Type::F64 => Instruction::F64Sub,
            _ if i64 => Instruction::I64Sub,
            _ => Instruction::I32Sub,
        },
        BinaryOp::Mul => match ty {
            Type::F32 => Instruction::F32Mul,
            Type::F64 => Instruction::F64Mul,
            _ if i64 => Instruction::I64Mul,
            _ => Instruction::I32Mul,
        },
        BinaryOp::Div => match ty {
            Type::F32 => Instruction::F32Div,
            Type::F64 => Instruction::F64Div,
            _ if i64 && signed => Instruction::I64DivS,
            _ if i64 => Instruction::I64DivU,
            _ if signed => Instruction::I32DivS,
            _ => Instruction::I32DivU,
        },
        BinaryOp::Rem => match ty {
            Type::F32 | Type::F64 => unreachable!("float remainder is not a wasm instruction"),
            _ if i64 && signed => Instruction::I64RemS,
            _ if i64 => Instruction::I64RemU,
            _ if signed => Instruction::I32RemS,
            _ => Instruction::I32RemU,
        },
        BinaryOp::BitOr => {
            if i64 {
                Instruction::I64Or
            } else {
                Instruction::I32Or
            }
        }
        BinaryOp::BitXor => {
            if i64 {
                Instruction::I64Xor
            } else {
                Instruction::I32Xor
            }
        }
        BinaryOp::BitAnd => {
            if i64 {
                Instruction::I64And
            } else {
                Instruction::I32And
            }
        }
        BinaryOp::Shl => {
            if i64 {
                Instruction::I64Shl
            } else {
                Instruction::I32Shl
            }
        }
        BinaryOp::Shr => match (i64, signed) {
            (true, true) => Instruction::I64ShrS,
            (true, false) => Instruction::I64ShrU,
            (false, true) => Instruction::I32ShrS,
            (false, false) => Instruction::I32ShrU,
        },
        BinaryOp::Or | BinaryOp::And => unreachable!(),
    };
    function.instruction(&instruction);
}

fn intrinsic_binary_op(intrinsic: IntrinsicId) -> Option<BinaryOp> {
    Some(match intrinsic {
        IntrinsicId::NumericAdd => BinaryOp::Add,
        IntrinsicId::NumericSubtract => BinaryOp::Sub,
        IntrinsicId::NumericMultiply => BinaryOp::Mul,
        IntrinsicId::NumericDivide => BinaryOp::Div,
        IntrinsicId::IntegerRemainder => BinaryOp::Rem,
        IntrinsicId::IntegerBitOr => BinaryOp::BitOr,
        IntrinsicId::IntegerBitXor => BinaryOp::BitXor,
        IntrinsicId::IntegerBitAnd => BinaryOp::BitAnd,
        IntrinsicId::IntegerShiftLeft => BinaryOp::Shl,
        IntrinsicId::IntegerShiftRight => BinaryOp::Shr,
        _ => return None,
    })
}

enum Compare {
    Lt,
    Le,
    Gt,
    Ge,
}

fn compare(ty: Type, signed: bool, op: Compare) -> Instruction<'static> {
    match (ty, signed, op) {
        (Type::F32, _, Compare::Lt) => Instruction::F32Lt,
        (Type::F32, _, Compare::Le) => Instruction::F32Le,
        (Type::F32, _, Compare::Gt) => Instruction::F32Gt,
        (Type::F32, _, Compare::Ge) => Instruction::F32Ge,
        (Type::F64, _, Compare::Lt) => Instruction::F64Lt,
        (Type::F64, _, Compare::Le) => Instruction::F64Le,
        (Type::F64, _, Compare::Gt) => Instruction::F64Gt,
        (Type::F64, _, Compare::Ge) => Instruction::F64Ge,
        (Type::I64 | Type::U64 | Type::Address, true, Compare::Lt) => Instruction::I64LtS,
        (Type::I64 | Type::U64 | Type::Address, true, Compare::Le) => Instruction::I64LeS,
        (Type::I64 | Type::U64 | Type::Address, true, Compare::Gt) => Instruction::I64GtS,
        (Type::I64 | Type::U64 | Type::Address, true, Compare::Ge) => Instruction::I64GeS,
        (Type::I64 | Type::U64 | Type::Address, false, Compare::Lt) => Instruction::I64LtU,
        (Type::I64 | Type::U64 | Type::Address, false, Compare::Le) => Instruction::I64LeU,
        (Type::I64 | Type::U64 | Type::Address, false, Compare::Gt) => Instruction::I64GtU,
        (Type::I64 | Type::U64 | Type::Address, false, Compare::Ge) => Instruction::I64GeU,
        (_, true, Compare::Lt) => Instruction::I32LtS,
        (_, true, Compare::Le) => Instruction::I32LeS,
        (_, true, Compare::Gt) => Instruction::I32GtS,
        (_, true, Compare::Ge) => Instruction::I32GeS,
        (_, false, Compare::Lt) => Instruction::I32LtU,
        (_, false, Compare::Le) => Instruction::I32LeU,
        (_, false, Compare::Gt) => Instruction::I32GtU,
        (_, false, Compare::Ge) => Instruction::I32GeU,
    }
}
