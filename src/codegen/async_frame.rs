//! Planned GC-frame storage for values that survive async suspension.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{ActionKind, ExprId, Program, ValueId},
    semantic::{ClosureInstance, FunctionInstance, SemanticModel},
    wasm_ir::{self, BodyOwner, LocalPurpose, TemporaryId},
};

use wasm_encoder::{Function, Instruction};

use super::{Type, semantic_type};

/// Common fields shared by every first-class future frame.
pub(super) const FUTURE_STATE_FIELD: u32 = 0;
pub(super) const FUTURE_TAG_FIELD: u32 = 1;
pub(super) const FUTURE_POLL_EPOCH_FIELD: u32 = 2;
pub(super) const FUTURE_BASE_FIELDS: u32 = 3;

/// How an async body reaches the continuation frame it is currently polling.
///
/// The host-owned `onAttach` frame lives in a global. Source-defined futures
/// pass their typed frame as a poll-function parameter. Keeping that detail in
/// one value lets the state-machine emitter remain independent of ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AsyncFrameRef {
    pub struct_type: u32,
    pub source: AsyncFrameSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AsyncFrameSource {
    Global(u32),
    Local(u32),
}

impl AsyncFrameRef {
    pub(super) fn emit(self, function: &mut Function) {
        match self.source {
            AsyncFrameSource::Global(index) => {
                function.instruction(&Instruction::GlobalGet(index));
            }
            AsyncFrameSource::Local(index) => {
                function.instruction(&Instruction::LocalGet(index));
            }
        }
        function.instruction(&Instruction::RefAsNonNull);
    }
}

#[derive(Default, Debug)]
pub(super) struct AsyncFrameLayout {
    pub fields: HashMap<ValueId, (u32, Type)>,
    pub temporaries: HashMap<TemporaryId, (u32, Type)>,
    pub types: Vec<Type>,
    /// Frame fields whose physical value is a shared mutable-capture cell,
    /// while `fields` and `types` retain the source-level value type.
    pub capture_cell_fields: HashSet<u32>,
    pub completion: Option<(u32, Type)>,
    pub children: HashMap<ExprId, (u32, Type)>,
    pub base_fields: u32,
}

impl AsyncFrameLayout {
    /// Adapts a generated leaf future's completion slot to the shared
    /// suspension emitter. Leaf futures have no source locals or nested child
    /// continuations, but store their ready value through the same destination
    /// contract as source state machines.
    pub(super) fn for_leaf_completion(completion: Option<(u32, Type)>) -> Self {
        Self {
            completion,
            base_fields: FUTURE_BASE_FIELDS,
            ..Self::default()
        }
    }

    pub(super) fn for_action(
        action: Option<ActionKind>,
        wasm_ir: &wasm_ir::Program,
        semantics: &SemanticModel,
    ) -> Option<Self> {
        let action = action?;
        let body = wasm_ir
            .body(BodyOwner::Action(action))
            .expect("checked actions have Wasm IR bodies");
        Some(Self::for_body(
            &body.entry,
            &body.locals,
            &body.frame_values,
            &body.frame_temporaries,
            wasm_ir,
            semantics,
            None,
            1,
            std::iter::empty(),
        ))
    }

    pub(super) fn for_function(
        instance: &FunctionInstance,
        program: &Program,
        wasm_ir: &wasm_ir::Program,
        semantics: &SemanticModel,
    ) -> Self {
        let body = wasm_ir
            .body(BodyOwner::Function(instance.clone()))
            .expect("checked functions have Wasm IR bodies");
        let declaration = program
            .functions
            .iter()
            .find(|function| function.id == instance.function)
            .expect("reachable function instances have declarations");
        let completion = match body.abi {
            wasm_ir::BodyAbi::AsyncFunction(wasm_ir::AsyncFunctionAbi { completion }) => {
                let completion =
                    semantic_type(semantics.specialize_type(instance, completion), semantics);
                completion.has_runtime_value().then_some(completion)
            }
            wasm_ir::BodyAbi::Direct | wasm_ir::BodyAbi::AttachPoll => {
                unreachable!("only suspending functions receive typed future frames")
            }
        };
        Self::for_body(
            &body.entry,
            &body.locals,
            &body.frame_values,
            &body.frame_temporaries,
            wasm_ir,
            semantics,
            Some(instance),
            FUTURE_BASE_FIELDS,
            declaration
                .params
                .iter()
                .map(|parameter| (parameter.id, wasm_ir.is_mutably_captured(parameter.id))),
        )
        .with_completion(completion)
    }

    pub(super) fn for_closure(
        instance: &ClosureInstance,
        closure: &wasm_ir::ClosureBody,
        program: &wasm_ir::Program,
        semantics: &SemanticModel,
    ) -> Self {
        let completion = closure
            .completion
            .map(|completion| {
                instance.owner.as_ref().map_or(completion, |owner| {
                    semantics.specialize_type(owner, completion)
                })
            })
            .map(|completion| semantic_type(completion, semantics))
            .filter(|completion| completion.has_runtime_value());
        let captures = closure
            .captures
            .iter()
            .map(|capture| (capture.value, capture.mutable));
        let parameters = closure
            .parameters
            .iter()
            .copied()
            .map(|value| (value, program.is_mutably_captured(value)));
        Self::for_body(
            &closure.entry,
            &closure.locals,
            &closure.frame_values,
            &closure.frame_temporaries,
            program,
            semantics,
            instance.owner.as_ref(),
            FUTURE_BASE_FIELDS,
            captures.chain(parameters),
        )
        .with_completion(completion)
    }

    #[allow(clippy::too_many_arguments)]
    fn for_body(
        entry: &wasm_ir::Block,
        locals: &[wasm_ir::Local],
        frame_values: &[ValueId],
        frame_temporaries: &[TemporaryId],
        program: &wasm_ir::Program,
        semantics: &SemanticModel,
        instance: Option<&FunctionInstance>,
        base_fields: u32,
        initial_values: impl IntoIterator<Item = (ValueId, bool)>,
    ) -> Self {
        let mut layout = Self {
            base_fields,
            ..Self::default()
        };
        for (value, capture_cell) in initial_values {
            let source = semantics
                .value_type(value)
                .expect("checked parameters have semantic types");
            let source = instance.map_or(source, |instance| {
                semantics.specialize_type(instance, source)
            });
            layout.push_value(value, semantic_type(source, semantics), capture_cell);
        }
        for local in locals {
            let destination = match local.purpose {
                LocalPurpose::Value(value) if frame_values.contains(&value) => Some(Ok(value)),
                LocalPurpose::Temporary(temporary) if frame_temporaries.contains(&temporary) => {
                    Some(Err(temporary))
                }
                _ => None,
            };
            if let Some(destination) = destination {
                let source = instance.map_or(local.ty, |instance| {
                    semantics.specialize_type(instance, local.ty)
                });
                let ty = semantic_type(source, semantics);
                match destination {
                    Ok(value) => {
                        layout.push_value(value, ty, program.is_mutably_captured(value));
                    }
                    Err(temporary) => {
                        if ty == Type::Never {
                            layout.temporaries.insert(temporary, (u32::MAX, ty));
                            continue;
                        }
                        let field = layout.base_fields + layout.types.len() as u32;
                        layout.temporaries.insert(temporary, (field, ty));
                        layout.types.push(ty);
                    }
                }
            }
        }
        struct Children<'a> {
            owner: Option<&'a FunctionInstance>,
            semantics: &'a SemanticModel,
            values: Vec<(ExprId, Type)>,
        }
        impl wasm_ir::Visitor for Children<'_> {
            fn visit_terminator(
                &mut self,
                terminator: &wasm_ir::Terminator,
                program: &wasm_ir::Program,
            ) {
                if let wasm_ir::Terminator::Suspend { value, .. } = terminator {
                    let expression = program
                        .expression(*value)
                        .expect("suspension operands belong to Wasm IR");
                    let directly_polled_intrinsic = matches!(
                        expression.kind,
                        wasm_ir::ExpressionKind::Call {
                            target: wasm_ir::CallTarget::Intrinsic { intrinsic, .. },
                            ..
                        } if crate::intrinsic_registry::contract(intrinsic).async_state.is_empty()
                    );
                    let ty = self.owner.map_or(expression.ty, |owner| {
                        self.semantics.specialize_type(owner, expression.ty)
                    });
                    let ty = semantic_type(ty, self.semantics);
                    if matches!(ty, Type::Async(_)) && !directly_polled_intrinsic {
                        self.values.push((*value, ty));
                    }
                }
                wasm_ir::walk_terminator(self, terminator, program);
            }
        }
        let mut children = Children {
            owner: instance,
            semantics,
            values: Vec::new(),
        };
        wasm_ir::Visitor::visit_block(&mut children, entry, program);
        for (expression, ty) in children.values {
            if layout.children.contains_key(&expression) {
                continue;
            }
            let field = layout.base_fields + layout.types.len() as u32;
            layout.children.insert(expression, (field, ty));
            layout.types.push(ty);
        }
        layout
    }

    fn push_value(&mut self, value: ValueId, ty: Type, capture_cell: bool) {
        if self.fields.contains_key(&value) {
            return;
        }
        if !ty.has_runtime_value() {
            self.fields.insert(value, (u32::MAX, ty));
            return;
        }
        let field = self.base_fields + self.types.len() as u32;
        self.fields.insert(value, (field, ty));
        self.types.push(ty);
        if capture_cell {
            self.capture_cell_fields.insert(field);
        }
    }

    fn with_completion(mut self, completion: Option<Type>) -> Self {
        if let Some(ty) = completion {
            let field = self.base_fields + self.types.len() as u32;
            self.types.push(ty);
            self.completion = Some((field, ty));
        }
        self
    }

    pub(super) fn field(&self, destination: wasm_ir::SuspensionDestination) -> Option<(u32, Type)> {
        let field = match destination {
            wasm_ir::SuspensionDestination::SourceValue(value) => self.fields.get(&value).copied(),
            wasm_ir::SuspensionDestination::Temporary(temporary) => {
                self.temporaries.get(&temporary).copied()
            }
            wasm_ir::SuspensionDestination::BodyResult => self.completion,
            wasm_ir::SuspensionDestination::Discard => None,
        };
        field.filter(|(field, _)| *field != u32::MAX)
    }
}

#[derive(Default)]
pub(super) struct AsyncFrameLayouts {
    pub attach: Option<AsyncFrameLayout>,
    functions: HashMap<FunctionInstance, AsyncFrameLayout>,
    ordered_functions: Vec<FunctionInstance>,
    closures: HashMap<ClosureInstance, AsyncFrameLayout>,
    ordered_closures: Vec<ClosureInstance>,
    leaves: HashMap<LeafFutureInstance, LeafFutureLayout>,
    ordered_leaves: Vec<LeafFutureInstance>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct LeafFutureInstance {
    pub owner: Option<FunctionInstance>,
    pub expression: ExprId,
}

pub(super) struct LeafFutureLayout {
    pub future: Type,
    pub receiver: Option<(u32, Type)>,
    pub arguments: HashMap<ExprId, (u32, Type)>,
    pub state: Vec<(u32, Type)>,
    pub completion: Option<(u32, Type)>,
    pub types: Vec<Type>,
}

impl AsyncFrameLayouts {
    pub(super) fn plan(
        on_attach: Option<ActionKind>,
        program: &Program,
        wasm_ir: &wasm_ir::Program,
        semantics: &SemanticModel,
        reachability: &super::reachability::Reachability,
    ) -> Self {
        let attach = AsyncFrameLayout::for_action(on_attach, wasm_ir, semantics);
        let mut functions = HashMap::new();
        let mut ordered_functions = Vec::new();
        for instance in reachability.functions() {
            let body = wasm_ir
                .body(BodyOwner::Function(instance.clone()))
                .expect("reachable functions have Wasm IR bodies");
            if !matches!(body.abi, wasm_ir::BodyAbi::AsyncFunction(_)) {
                continue;
            }
            ordered_functions.push(instance.clone());
            functions.insert(
                instance.clone(),
                AsyncFrameLayout::for_function(instance, program, wasm_ir, semantics),
            );
        }
        let mut closures = HashMap::new();
        let mut ordered_closures = Vec::new();
        for instance in reachability.closure_instances() {
            let closure = wasm_ir
                .closure(instance.expression)
                .expect("reachable closures have Wasm IR bodies");
            if closure.completion.is_none() {
                continue;
            }
            ordered_closures.push(instance.clone());
            closures.insert(
                instance.clone(),
                AsyncFrameLayout::for_closure(instance, closure, wasm_ir, semantics),
            );
        }
        let mut leaves = HashMap::new();
        let mut ordered_leaves = Vec::new();
        let mut directly_polled = HashSet::new();
        struct DirectIntrinsicPolls<'a> {
            owner: Option<&'a FunctionInstance>,
            values: &'a mut HashSet<LeafFutureInstance>,
        }
        impl wasm_ir::Visitor for DirectIntrinsicPolls<'_> {
            fn visit_terminator(
                &mut self,
                terminator: &wasm_ir::Terminator,
                program: &wasm_ir::Program,
            ) {
                if let wasm_ir::Terminator::Suspend { value, .. } = terminator
                    && matches!(
                        program
                            .expression(*value)
                            .expect("suspension operands belong to Wasm IR")
                            .kind,
                        wasm_ir::ExpressionKind::Call {
                            target: wasm_ir::CallTarget::Intrinsic { intrinsic, .. },
                            ..
                        } if crate::intrinsic_registry::contract(intrinsic).async_state.is_empty()
                    )
                {
                    self.values.insert(LeafFutureInstance {
                        owner: self.owner.cloned(),
                        expression: *value,
                    });
                }
                wasm_ir::walk_terminator(self, terminator, program);
            }
        }
        for body in wasm_ir
            .bodies()
            .filter(|body| matches!(body.owner, BodyOwner::Action(_)))
        {
            wasm_ir::Visitor::visit_block(
                &mut DirectIntrinsicPolls {
                    owner: None,
                    values: &mut directly_polled,
                },
                &body.entry,
                wasm_ir,
            );
        }
        for owner in reachability.functions() {
            let body = wasm_ir
                .body(BodyOwner::Function(owner.clone()))
                .expect("reachable functions have Wasm IR bodies");
            wasm_ir::Visitor::visit_block(
                &mut DirectIntrinsicPolls {
                    owner: Some(owner),
                    values: &mut directly_polled,
                },
                &body.entry,
                wasm_ir,
            );
        }
        for instance in reachability.closure_instances() {
            let closure = wasm_ir
                .closure(instance.expression)
                .expect("reachable closures have Wasm IR bodies");
            wasm_ir::Visitor::visit_block(
                &mut DirectIntrinsicPolls {
                    owner: instance.owner.as_ref(),
                    values: &mut directly_polled,
                },
                &closure.entry,
                wasm_ir,
            );
        }
        for (owner, expression) in reachability.expression_instances() {
            let lowered = wasm_ir
                .expression(expression)
                .expect("reachable expressions belong to Wasm IR");
            let wasm_ir::ExpressionKind::Call { target, arguments } = &lowered.kind else {
                continue;
            };
            if !matches!(
                target,
                wasm_ir::CallTarget::Intrinsic { .. }
                    | wasm_ir::CallTarget::ManagedInstances { .. }
            ) {
                continue;
            }
            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty))
            };
            let future_id = specialize(lowered.ty);
            let Type::Async(_) = semantic_type(future_id, semantics) else {
                continue;
            };
            let crate::types::TypeKind::Async { value, .. } = semantics.types().kind(future_id)
            else {
                unreachable!()
            };
            let mut types = Vec::new();
            let receiver = match target {
                wasm_ir::CallTarget::Intrinsic {
                    receiver,
                    receiver_type,
                    ..
                } => receiver.as_ref().map(|_| {
                    let ty = semantic_type(
                        specialize(receiver_type.expect("method receivers have semantic types")),
                        semantics,
                    );
                    let field = FUTURE_BASE_FIELDS + types.len() as u32;
                    types.push(ty);
                    (field, ty)
                }),
                wasm_ir::CallTarget::ManagedInstances { .. } => None,
                _ => unreachable!(),
            };
            let mut captured_arguments = HashMap::new();
            if matches!(target, wasm_ir::CallTarget::Intrinsic { .. }) {
                for argument in arguments {
                    let argument_expression = wasm_ir
                        .expression(*argument)
                        .expect("leaf-future arguments belong to Wasm IR");
                    if matches!(
                        argument_expression.kind,
                        wasm_ir::ExpressionKind::String(_) | wasm_ir::ExpressionKind::Signature(_)
                    ) {
                        continue;
                    }
                    let ty = semantic_type(specialize(argument_expression.ty), semantics);
                    let field = FUTURE_BASE_FIELDS + types.len() as u32;
                    types.push(ty);
                    captured_arguments.insert(*argument, (field, ty));
                }
            }
            let mut state = Vec::new();
            match target {
                wasm_ir::CallTarget::Intrinsic { intrinsic, .. } => {
                    for policy in crate::intrinsic_registry::contract(*intrinsic).async_state {
                        let ty = match policy.ty {
                            crate::intrinsic_registry::ScratchType::Core(core) => {
                                semantic_type(semantics.types().id_for_core(core), semantics)
                            }
                            crate::intrinsic_registry::ScratchType::Standard(standard) => {
                                semantic_type(
                                    semantics.types().id_for_standard(standard),
                                    semantics,
                                )
                            }
                            crate::intrinsic_registry::ScratchType::Expression
                            | crate::intrinsic_registry::ScratchType::ResultValue
                            | crate::intrinsic_registry::ScratchType::AsyncArgumentValue(_)
                            | crate::intrinsic_registry::ScratchType::Receiver => {
                                unreachable!("intrinsic future state currently uses concrete types")
                            }
                        };
                        for _ in 0..policy.slots {
                            let field = FUTURE_BASE_FIELDS + types.len() as u32;
                            types.push(ty);
                            state.push((field, ty));
                        }
                    }
                }
                wasm_ir::CallTarget::ManagedInstances { .. } => {
                    let cursor = semantic_type(
                        semantics
                            .types()
                            .id_for_core(crate::stdlib::CoreTypeId::U64),
                        semantics,
                    );
                    let result = semantic_type(*value, semantics);
                    for ty in [cursor, cursor, result] {
                        let field = FUTURE_BASE_FIELDS + types.len() as u32;
                        types.push(ty);
                        state.push((field, ty));
                    }
                }
                _ => unreachable!(),
            }
            let completion_type = semantic_type(specialize(*value), semantics);
            let completion = completion_type.has_runtime_value().then(|| {
                let field = FUTURE_BASE_FIELDS + types.len() as u32;
                types.push(completion_type);
                (field, completion_type)
            });
            let instance = LeafFutureInstance {
                owner: owner.clone(),
                expression,
            };
            if directly_polled.contains(&instance) {
                continue;
            }
            if leaves.contains_key(&instance) {
                continue;
            }
            ordered_leaves.push(instance.clone());
            leaves.insert(
                instance,
                LeafFutureLayout {
                    future: semantic_type(future_id, semantics),
                    receiver,
                    arguments: captured_arguments,
                    state,
                    completion,
                    types,
                },
            );
        }
        Self {
            attach,
            functions,
            ordered_functions,
            closures,
            ordered_closures,
            leaves,
            ordered_leaves,
        }
    }

    pub(super) fn function(&self, instance: &FunctionInstance) -> Option<&AsyncFrameLayout> {
        self.functions.get(instance)
    }

    pub(super) fn functions(
        &self,
    ) -> impl ExactSizeIterator<Item = (&FunctionInstance, &AsyncFrameLayout)> {
        self.ordered_functions
            .iter()
            .map(|instance| (instance, &self.functions[instance]))
    }

    pub(super) fn closure(&self, instance: &ClosureInstance) -> Option<&AsyncFrameLayout> {
        self.closures.get(instance)
    }

    pub(super) fn closures(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ClosureInstance, &AsyncFrameLayout)> {
        self.ordered_closures
            .iter()
            .map(|instance| (instance, &self.closures[instance]))
    }

    pub(super) fn leaf(&self, instance: &LeafFutureInstance) -> Option<&LeafFutureLayout> {
        self.leaves.get(instance)
    }

    pub(super) fn leaves(
        &self,
    ) -> impl ExactSizeIterator<Item = (&LeafFutureInstance, &LeafFutureLayout)> {
        self.ordered_leaves
            .iter()
            .map(|instance| (instance, &self.leaves[instance]))
    }
}
