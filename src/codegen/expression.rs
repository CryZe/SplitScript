//! Structured Wasm-IR block, assignment, expression, and intrinsic emission.

use std::collections::HashMap;

use wasm_encoder::{AbstractHeapType, BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, BinaryOp, EnumDecl, ExprId, RecordDecl, ResultTypeId, UnaryOp, ValueId},
    intrinsic_registry::RuntimeHelperId,
    memory::MemoryLayouts,
    semantic::{
        FunctionInstance, ResolvedMember, ResolvedReceiver, ResolvedRecordFieldId,
        ResolvedRecordId, ResolvedValue, SemanticModel, ValueConversionKind,
    },
    stdlib::{IntrinsicId, RuntimeRepresentation, StandardLibrary, StdlibFieldId, StdlibTypeId},
    types::{EnumTypeId, ResolvedArrayType, TypeId},
    wasm_ir::{self, TemporaryId},
};

use super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE, SettingStorage, Type,
    array_element_type,
    async_frame::{AsyncFrameRef, IntrinsicFutureInstance, IntrinsicFutureLayout},
    emit_array_get, emit_default, emit_failure_transfer, emit_int, emit_memory_value,
    emit_result_error, emit_result_success, emit_string_literal, emit_struct_get,
    emit_typed_struct_get, enum_variant_payload,
    global_plan::RuntimeGlobals,
    imports::Abi,
    memarg,
    memory_plan::AbiReadScratch,
    record_field_type, resolved_intrinsic, result_value_type,
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
    AsyncAttach,
    AsyncFuture {
        frame: AsyncFrameRef,
        completion: Option<(u32, Type)>,
    },
}

#[derive(Clone, Copy)]
pub(super) struct ExprContext<'a> {
    pub standard_library: &'a StandardLibrary,
    pub abi: &'a Abi,
    pub state: &'a crate::ast::StateDecl,
    pub locals: LocalStorage<'a>,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub settings: &'a HashMap<ValueId, SettingStorage>,
    pub runtime_globals: RuntimeGlobals,
    pub runtime_helpers: &'a RuntimeHelperPlan,
    pub functions: &'a HashMap<FunctionInstance, super::function_plan::UserFunctionPlan>,
    pub intrinsic_futures: &'a HashMap<IntrinsicFutureInstance, u32>,
    pub display_functions: &'a HashMap<StdlibTypeId, FunctionInstance>,
    pub equality_functions: &'a EqualityFunctions,
    pub records: &'a [RecordDecl],
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub memory: &'a MemoryLayouts,
    pub abi_read: AbiReadScratch,
    pub matches: &'a MatchLayout,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
    pub gc: &'a GcLayout,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub intrinsic_capture: Option<IntrinsicCapture<'a>>,
    /// Concrete type arguments while emitting a generic function template.
    pub function_instance: Option<&'a FunctionInstance>,
    pub loop_control: Option<LoopControl>,
    pub bare_return: BareReturn,
    /// Whether semantic unit values need a physical operand-stack value.
    /// Discarded expressions and unit-returning ABIs erase `None` entirely.
    pub materialize_none: bool,
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
    },
    Async {
        break_state: wasm_ir::AsyncStateId,
        continue_state: wasm_ir::AsyncStateId,
        dispatcher_depth: u32,
    },
}

impl LoopControl {
    pub(super) fn nested(self, depth: u32) -> Self {
        match self {
            Self::Branch {
                break_depth,
                continue_depth,
            } => Self::Branch {
                break_depth: break_depth + depth,
                continue_depth: continue_depth + depth,
            },
            Self::Async {
                break_state,
                continue_state,
                dispatcher_depth,
            } => Self::Async {
                break_state,
                continue_state,
                dispatcher_depth: dispatcher_depth + depth,
            },
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
            wasm_ir::Statement::Store {
                target,
                operation,
                value,
                ..
            } => {
                compile_assignment(function, *target, operation.as_ref(), *value, context);
            }
            wasm_ir::Statement::StoreTemporary { target, value } => {
                compile_temporary_set(function, *target, *value, context);
            }
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
            wasm_ir::Statement::While { condition, body } => {
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
                iterable,
                body,
            } => {
                compile_for_init(function, *iterable_value, *index_value, *iterable, context);
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_for_has_next(function, *iterable_value, *index_value, context);
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_for_bind_and_advance(
                    function,
                    *binding,
                    *iterable_value,
                    *index_value,
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
                iterable,
                ..
            } => compile_for_init(function, *iterable_value, *index_value, *iterable, context),
            wasm_ir::Statement::Evaluate {
                expression,
                discard_result,
            } => {
                let ty = context.expression_type(*expression);
                let expression_context = if *discard_result && ty == Type::None {
                    context.erasing_none()
                } else {
                    *context
                };
                compile_expr(function, *expression, &expression_context);
                if *discard_result && ty != Type::None {
                    function.instruction(&Instruction::Drop);
                }
            }
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Fallthrough => {}
        wasm_ir::Terminator::Break => {
            loop_control
                .expect("checked break statements belong to loops")
                .emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue statements belong to loops")
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
        wasm_ir::Terminator::Throw { error, target } => {
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
        wasm_ir::Terminator::Suspend { .. } => {
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
    operation: Option<&wasm_ir::AssignmentOperation>,
    value: ExprId,
    context: &ExprContext<'_>,
) {
    match context.locals {
        LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&target) => {
            let (field, ty) = frame_values[&target];
            if ty == Type::None {
                compile_expr(function, value, &context.erasing_none());
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
            if ty == Type::None {
                compile_expr(function, value, &context.erasing_none());
                return;
            }
            compile_assignment_value(function, operation, value, ty, context, |function| {
                function.instruction(&Instruction::LocalGet(local));
            });
            function.instruction(&Instruction::LocalSet(local));
        }
        LocalStorage::Wasm { values, .. } if values.contains_key(&target) => {
            let (local, ty) = values[&target];
            if ty == Type::None {
                compile_expr(function, value, &context.erasing_none());
                return;
            }
            compile_assignment_value(function, operation, value, ty, context, |function| {
                function.instruction(&Instruction::LocalGet(local));
            });
            function.instruction(&Instruction::LocalSet(local));
        }
        _ => {
            let ty = context.global_types[&target];
            if ty == Type::None {
                compile_expr(function, value, &context.erasing_none());
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
            let (field, _) = frame_temporaries[&target];
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
            compile_expr(function, value, context);
            function.instruction(&Instruction::LocalSet(wasm_temporaries[&target].0));
        }
        LocalStorage::Wasm { temporaries, .. } => {
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
            let frame = context.locals.frame();
            frame.emit(function);
            emit_typed_struct_get(function, frame.struct_type, field, ty);
        }
        LocalStorage::Hybrid {
            wasm_temporaries, ..
        } => {
            function.instruction(&Instruction::LocalGet(wasm_temporaries[&temporary].0));
        }
        LocalStorage::Wasm { temporaries, .. } => {
            function.instruction(&Instruction::LocalGet(temporaries[&temporary].0));
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
            if matches!(
                intrinsic,
                IntrinsicId::NumericAdd | IntrinsicId::NumericSubtract
            ) =>
        {
            let receiver = compile_receiver(function, target, context);
            compile_expr(function, argument, context);
            emit_binary_instruction(
                function,
                if *intrinsic == IntrinsicId::NumericAdd {
                    BinaryOp::Add
                } else {
                    BinaryOp::Sub
                },
                receiver,
            );
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
    let value_type = match value {
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
        ResolvedValue::CurrentSnapshot | ResolvedValue::OldSnapshot => {
            let snapshot = u32::from(matches!(value, ResolvedValue::OldSnapshot));
            function.instruction(&Instruction::LocalGet(snapshot));
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
            let snapshot = u32::from(matches!(value, ResolvedValue::OldState(_)));
            function.instruction(&Instruction::LocalGet(snapshot));
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
        ResolvedValue::Variable(value) => match context.locals {
            LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&value) => {
                let (field, ty) = frame_values[&value];
                if ty == Type::None {
                    if context.materialize_none {
                        emit_default(function, Type::None, context.gc);
                    }
                } else {
                    let frame = context.locals.frame();
                    frame.emit(function);
                    emit_typed_struct_get(function, frame.struct_type, field, ty);
                }
                ty
            }
            LocalStorage::Hybrid { wasm_values, .. } if wasm_values.contains_key(&value) => {
                let (local, ty) = wasm_values[&value];
                if ty == Type::None {
                    if context.materialize_none {
                        emit_default(function, Type::None, context.gc);
                    }
                } else {
                    function.instruction(&Instruction::LocalGet(local));
                }
                ty
            }
            LocalStorage::Wasm { values, .. } if values.contains_key(&value) => {
                let (local, ty) = values[&value];
                if ty == Type::None {
                    if context.materialize_none {
                        emit_default(function, Type::None, context.gc);
                    }
                } else {
                    function.instruction(&Instruction::LocalGet(local));
                }
                ty
            }
            _ => {
                let ty = context.global_types[&value];
                if ty != Type::None {
                    function.instruction(&Instruction::GlobalGet(context.globals[&value]));
                } else if context.materialize_none {
                    emit_default(function, Type::None, context.gc);
                }
                ty
            }
        },
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

fn compile_value_set(
    function: &mut Function,
    value: ValueId,
    context: &ExprContext<'_>,
    emit_value: impl FnOnce(&mut Function),
) {
    match context.locals {
        LocalStorage::Hybrid { frame_values, .. } if frame_values.contains_key(&value) => {
            let (field, ty) = frame_values[&value];
            if ty == Type::None {
                emit_value(function);
                function.instruction(&Instruction::Drop);
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
            if wasm_values[&value].1 == Type::None {
                emit_value(function);
                function.instruction(&Instruction::Drop);
                return;
            }
            emit_value(function);
            function.instruction(&Instruction::LocalSet(wasm_values[&value].0));
        }
        LocalStorage::Wasm { values, .. } if values.contains_key(&value) => {
            if values[&value].1 == Type::None {
                emit_value(function);
                function.instruction(&Instruction::Drop);
                return;
            }
            emit_value(function);
            function.instruction(&Instruction::LocalSet(values[&value].0));
        }
        _ => unreachable!("compiler-owned for-loop values are local"),
    }
}

fn for_array_type(
    iterable_value: ValueId,
    context: &ExprContext<'_>,
) -> (crate::ast::ArrayTypeId, Type) {
    let ty = context
        .semantics
        .value_type(iterable_value)
        .expect("checked for-loop iterable storage has a type");
    let Type::Array(array) = context.ty(ty) else {
        unreachable!("checked for-loop iterables are arrays")
    };
    (array, array_element_type(array, context.semantics))
}

pub(super) fn compile_for_init(
    function: &mut Function,
    iterable_value: ValueId,
    index_value: ValueId,
    iterable: ExprId,
    context: &ExprContext<'_>,
) {
    compile_value_set(function, iterable_value, context, |function| {
        compile_expr(function, iterable, context);
    });
    compile_value_set(function, index_value, context, |function| {
        function.instruction(&Instruction::I32Const(0));
    });
}

/// Leaves whether another element exists on the stack.
pub(super) fn compile_for_has_next(
    function: &mut Function,
    iterable_value: ValueId,
    index_value: ValueId,
    context: &ExprContext<'_>,
) {
    compile_value_get(function, index_value, context);
    compile_value_get(function, iterable_value, context);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32LtU);
}

/// Stores the current element in the source binding and advances before the
/// body, so a `continue` cannot accidentally repeat the same element.
pub(super) fn compile_for_bind_and_advance(
    function: &mut Function,
    binding: ValueId,
    iterable_value: ValueId,
    index_value: ValueId,
    context: &ExprContext<'_>,
) {
    let (array, element) = for_array_type(iterable_value, context);
    compile_value_set(function, binding, context, |function| {
        compile_value_get(function, iterable_value, context);
        function.instruction(&Instruction::RefAsNonNull);
        compile_value_get(function, index_value, context);
        emit_array_get(function, context.gc.index(Type::Array(array)), element);
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
                let owner = library.type_decl(declaration.owner);
                let RuntimeRepresentation::GcStruct { .. } = owner.representation else {
                    unreachable!("resolved standard field belongs to a GC struct")
                };
                let field_index = library
                    .fields_of(owner.id)
                    .position(|candidate| candidate.id == *field)
                    .expect("declared standard field has a runtime slot")
                    as u32;
                let owner_type = Type::from_standard(declaration.owner);
                debug_assert_eq!(current_type, owner_type);
                (
                    context.gc.index(owner_type),
                    field_index,
                    standard_field_type(declaration.id, context.semantics),
                )
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

pub(super) fn compile_expr(function: &mut Function, expression: ExprId, context: &ExprContext<'_>) {
    if let Some(capture) = context.intrinsic_capture
        && let Some(&(field, ty)) = capture.layout.arguments.get(&expression)
    {
        if ty != Type::None || context.materialize_none {
            capture.frame.emit(function);
            emit_typed_struct_get(function, capture.frame.struct_type, field, ty);
        }
        return;
    }
    let expression_ir = context
        .wasm_ir
        .expression(expression)
        .expect("compiled expression belongs to Wasm IR");
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
}

fn compile_expr_unconverted(
    function: &mut Function,
    expression_ir: &wasm_ir::Expression,
    ty: Type,
    context: &ExprContext<'_>,
) {
    let expression = expression_ir.id;
    match &expression_ir.kind {
        wasm_ir::ExpressionKind::Suspend { destination, .. } => {
            if ty != Type::None || context.materialize_none {
                compile_value_get(function, *destination, context);
            }
        }
        wasm_ir::ExpressionKind::Temporary(temporary) => {
            if ty != Type::None || context.materialize_none {
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
        wasm_ir::ExpressionKind::Bool(value) => {
            function.instruction(&Instruction::I32Const(*value as i32));
        }
        wasm_ir::ExpressionKind::Int(value) => emit_int(function, *value, ty),
        wasm_ir::ExpressionKind::Float(value) => {
            if ty == Type::F32 {
                function.instruction(&Instruction::F32Const((*value as f32).into()));
            } else {
                function.instruction(&Instruction::F64Const((*value).into()));
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
            function.instruction(&Instruction::ArrayNewFixed {
                array_type_index: context.gc.index(Type::Array(strings.id)),
                array_size: parts.len() as u32,
            });
            function.instruction(&Instruction::Call(
                context
                    .runtime_helpers
                    .function(RuntimeHelperId::ConcatStrings),
            ));
        }
        wasm_ir::ExpressionKind::Signature(_) => {
            unreachable!("signature literals are lowered by signature-consuming builtins")
        }
        wasm_ir::ExpressionKind::Array(elements) => {
            let Type::Array(array_id) = ty else {
                unreachable!();
            };
            for element in elements {
                compile_expr(function, *element, context);
            }
            function.instruction(&Instruction::ArrayNewFixed {
                array_type_index: context.gc.index(Type::Array(array_id)),
                array_size: elements.len() as u32,
            });
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
            compile_expr(function, *receiver, context);
            let receiver_type = context.expression_type(*receiver);
            let lowered_type = emit_path_fields(function, members, receiver_type, context);
            debug_assert_eq!(lowered_type, ty);
        }
        wasm_ir::ExpressionKind::Index { receiver, index } => {
            compile_expr(function, *receiver, context);
            let Type::Array(array_id) = context.expression_type(*receiver) else {
                unreachable!("checked index receivers are arrays")
            };
            function.instruction(&Instruction::RefAsNonNull);
            compile_expr(function, *index, context);
            let element = array_element_type(array_id, context.semantics);
            emit_array_get(function, context.gc.index(Type::Array(array_id)), element);
        }
        wasm_ir::ExpressionKind::Unary { op, operand } => match op {
            UnaryOp::Not => {
                compile_expr(function, *operand, context);
                function.instruction(&Instruction::I32Eqz);
            }
            UnaryOp::Neg => match ty {
                Type::I8 | Type::I16 | Type::I32 => {
                    function.instruction(&Instruction::I32Const(0));
                    compile_expr(function, *operand, context);
                    function.instruction(&Instruction::I32Sub);
                }
                Type::I64 => {
                    function.instruction(&Instruction::I64Const(0));
                    compile_expr(function, *operand, context);
                    function.instruction(&Instruction::I64Sub);
                }
                Type::F32 => {
                    compile_expr(function, *operand, context);
                    function.instruction(&Instruction::F32Neg);
                }
                Type::F64 => {
                    compile_expr(function, *operand, context);
                    function.instruction(&Instruction::F64Neg);
                }
                _ => unreachable!(),
            },
        },
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
                    compile_fallback_branch(function, *fallback, &nested_context);
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
                    compile_fallback_branch(function, *fallback, &nested_context);
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
        wasm_ir::ExpressionKind::Propagate { value, target } => {
            let input_local = context.matches.fallback_values[&expression];
            let Type::Result(input_result) = context.expression_type(*value) else {
                unreachable!("typed propagation inputs are result values")
            };
            let Type::Result(target_result) = context.ty(*target) else {
                unreachable!("propagation targets are result values")
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
        wasm_ir::ExpressionKind::Call { .. } => {}
    }
    let wasm_ir::ExpressionKind::Call {
        target,
        arguments: args,
    } = &expression_ir.kind
    else {
        return;
    };
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
            wasm_ir::CallTarget::Intrinsic { .. } => {
                unreachable!("standard-library implementations have intrinsic IDs")
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
            IntrinsicId::Print => {
                compile_as_string(function, args[0], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::PrintString),
                ));
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
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ConcatStrings),
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
            IntrinsicId::SettingsEnabled => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::SettingsEnabled),
                ));
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
                compile_expr(function, args[0], context);
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
                let names = &context.state.processes;
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
            IntrinsicId::NextTick
            | IntrinsicId::ProcessClosed
            | IntrinsicId::ProcessMainModule
            | IntrinsicId::ProcessModule => {
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
            IntrinsicId::ProcessScan => {
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
            IntrinsicId::UnityIl2Cpp => {
                unreachable!("Unity.il2cpp is lowered as an await")
            }
            IntrinsicId::GbaAttach => {
                function
                    .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
                    .instruction(&Instruction::Call(
                        context.runtime_helpers.function(RuntimeHelperId::GbaAttach),
                    ));
                emit_sentinel_result(
                    function,
                    expression,
                    Type::Standard(StdlibTypeId::GbaEmulator),
                    Instruction::RefIsNull,
                    "GBA emulator memory is not available",
                    context,
                );
            }
            IntrinsicId::GbaEmulatorRead => {
                let read_type = match target {
                    wasm_ir::CallTarget::Intrinsic { type_arguments, .. } => {
                        context.type_id(type_arguments[0])
                    }
                    _ => unreachable!("GBA reads resolve to their standard-library method"),
                };
                let Type::Result(result_type) = context.expression_type(expression) else {
                    unreachable!("GBA reads produce Result values")
                };
                let address = context.matches.intrinsic_temps[&expression][0];
                function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                let size = context
                    .memory
                    .layout(read_type, context.semantics)
                    .expect("checked GBA reads are MemoryReadable")
                    .size();
                function
                    .instruction(&Instruction::I32Const(size as i32))
                    .instruction(&Instruction::Call(
                        context
                            .runtime_helpers
                            .function(RuntimeHelperId::GbaTranslateAddress),
                    ))
                    .instruction(&Instruction::LocalTee(address))
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Result(
                        context.gc.val_type(Type::Result(result_type)),
                    )));
                emit_result_error(
                    function,
                    result_type,
                    context.ty(read_type),
                    "invalid or unavailable GBA memory address",
                    context.gc,
                );
                function
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::GlobalGet(context.runtime_globals.process))
                    .instruction(&Instruction::LocalGet(address));
                emit_process_read_from_stack(
                    function,
                    read_type,
                    result_type,
                    "GBA memory read failed",
                    context,
                );
                function.instruction(&Instruction::End);
            }
            IntrinsicId::NumericMin | IntrinsicId::NumericMax => {
                unreachable!("numeric intrinsics are lowered before ordinary calls")
            }
            IntrinsicId::NumericAdd | IntrinsicId::NumericSubtract => {
                let receiver = compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                emit_binary_instruction(
                    function,
                    if builtin == IntrinsicId::NumericAdd {
                        BinaryOp::Add
                    } else {
                        BinaryOp::Sub
                    },
                    receiver,
                );
            }
            IntrinsicId::FloatAbs
            | IntrinsicId::FloatFloor
            | IntrinsicId::FloatCeil
            | IntrinsicId::FloatRound => {
                let receiver = compile_receiver(function, target, context);
                function.instruction(&match (receiver, builtin) {
                    (Type::F32, IntrinsicId::FloatAbs) => Instruction::F32Abs,
                    (Type::F32, IntrinsicId::FloatFloor) => Instruction::F32Floor,
                    (Type::F32, IntrinsicId::FloatCeil) => Instruction::F32Ceil,
                    (Type::F32, IntrinsicId::FloatRound) => Instruction::F32Nearest,
                    (Type::F64, IntrinsicId::FloatAbs) => Instruction::F64Abs,
                    (Type::F64, IntrinsicId::FloatFloor) => Instruction::F64Floor,
                    (Type::F64, IntrinsicId::FloatCeil) => Instruction::F64Ceil,
                    (Type::F64, IntrinsicId::FloatRound) => Instruction::F64Nearest,
                    _ => unreachable!("float intrinsics require an f32 or f64 receiver"),
                });
            }
            IntrinsicId::AddressAdd => {
                compile_receiver(function, target, context);
                compile_expr(function, args[0], context);
                function.instruction(&Instruction::I64Add);
            }
            IntrinsicId::ArrayLength | IntrinsicId::ArraySet => {
                let receiver_type = compile_receiver(function, target, context);
                let Type::Array(array_id) = receiver_type else {
                    unreachable!();
                };
                function.instruction(&Instruction::RefAsNonNull);
                for argument in args {
                    compile_expr(function, *argument, context);
                }
                match builtin {
                    IntrinsicId::ArrayLength => {
                        function.instruction(&Instruction::ArrayLen);
                    }
                    IntrinsicId::ArraySet => {
                        function.instruction(&Instruction::ArraySet(
                            context.gc.index(Type::Array(array_id)),
                        ));
                    }
                    _ => unreachable!(),
                }
            }
            IntrinsicId::ModuleScan
            | IntrinsicId::UnityModuleImage
            | IntrinsicId::UnityImageClass
            | IntrinsicId::UnityClassField
            | IntrinsicId::UnityClassFieldAny
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

fn compile_fallback_branch(
    function: &mut Function,
    fallback: wasm_ir::FallbackBranch,
    context: &ExprContext<'_>,
) {
    match fallback {
        wasm_ir::FallbackBranch::Value(value) => compile_expr(function, value, context),
        wasm_ir::FallbackBranch::Return(value) => {
            if let BareReturn::AsyncFuture { frame, completion } = context.bare_return {
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
            } else if let Some(value) = value {
                compile_expr(function, value, context);
            } else {
                match context.bare_return {
                    BareReturn::None => {}
                    BareReturn::Action(action) => {
                        emit_action_default(function, action, context.gc);
                    }
                    BareReturn::AsyncAttach => {
                        function.instruction(&Instruction::I32Const(1));
                    }
                    BareReturn::AsyncFuture { .. } => unreachable!(),
                }
            }
            function.instruction(&Instruction::Return);
        }
        wasm_ir::FallbackBranch::Break => {
            context
                .loop_control
                .expect("checked `else break` belongs to a loop")
                .emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::FallbackBranch::Continue => {
            context
                .loop_control
                .expect("checked `else continue` belongs to a loop")
                .emit_continue(function, context.locals.continuation_frame());
        }
    }
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

fn emit_process_read_from_stack(
    function: &mut Function,
    ty: TypeId,
    result_type: ResultTypeId,
    error: &str,
    context: &ExprContext<'_>,
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

fn emit_cast(function: &mut Function, expression: ExprId, target: Type, context: &ExprContext<'_>) {
    let source = context.expression_type(expression);
    compile_expr(function, expression, context);

    if target == Type::Standard(StdlibTypeId::String) {
        if source == target {
            return;
        }
        if let Type::Standard(standard) = source
            && let Some(display) = context.display_functions.get(&standard)
        {
            let display = context.called_instance(display);
            function.instruction(&Instruction::Call(context.functions[&display].call));
            return;
        }
        function
            .instruction(&if matches!(source, Type::I8 | Type::I16 | Type::I32) {
                Instruction::I64ExtendI32S
            } else if matches!(source, Type::U8 | Type::U16 | Type::U32) {
                Instruction::I64ExtendI32U
            } else {
                Instruction::Nop
            })
            .instruction(&Instruction::I32Const(source.is_signed() as i32))
            .instruction(&Instruction::Call(
                context.runtime_helpers.function(RuntimeHelperId::FormatI64),
            ));
        return;
    }

    let source_i32 = matches!(
        source,
        Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32
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

fn emit_binary(
    function: &mut Function,
    op: BinaryOp,
    left: ExprId,
    right: ExprId,
    context: &ExprContext<'_>,
) {
    let operand_type = context.expression_type(left);
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && operand_type == Type::None {
        let operand_context = context.erasing_none();
        compile_expr(function, left, &operand_context);
        compile_expr(function, right, &operand_context);
        function.instruction(&Instruction::I32Const(i32::from(op == BinaryOp::Eq)));
        return;
    }
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
        && matches!(operand_type, Type::Standard(_))
        && operand_type.is_enum(context.standard_library)
    {
        for expression in [left, right] {
            compile_expr(function, expression, context);
            function.instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, context.gc.index(operand_type), 0, Type::I32);
        }
        function.instruction(&Instruction::I32Eq);
        if op == BinaryOp::Ne {
            function.instruction(&Instruction::I32Eqz);
        }
        return;
    }
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
        && matches!(
            operand_type,
            Type::Standard(_) | Type::Record(_) | Type::Enum(_) | Type::Option(_) | Type::Result(_)
        )
    {
        compile_expr(function, left, context);
        compile_expr(function, right, context);
        emit_value_equality(
            function,
            operand_type,
            context.equality_functions,
            context
                .runtime_helpers
                .optional_function(RuntimeHelperId::StringEquality)
                .unwrap_or(0),
        );
        if op == BinaryOp::Ne {
            function.instruction(&Instruction::I32Eqz);
        }
        return;
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
