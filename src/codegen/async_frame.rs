//! Planned GC-frame storage for values that survive async suspension.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, ExprId, Program, ValueId},
    semantic::{FunctionInstance, SemanticModel},
    wasm_ir::{self, BodyOwner, LocalPurpose, TemporaryId},
};

use wasm_encoder::{Function, Instruction};

use super::{Type, semantic_type};

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

#[derive(Default)]
pub(super) struct AsyncFrameLayout {
    pub fields: HashMap<ValueId, (u32, Type)>,
    pub temporaries: HashMap<TemporaryId, (u32, Type)>,
    pub types: Vec<Type>,
    pub completion: Option<(u32, Type)>,
    pub children: HashMap<ExprId, (u32, Type)>,
    pub base_fields: u32,
}

impl AsyncFrameLayout {
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
            body,
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
            wasm_ir::BodyAbi::AsyncFunction(wasm_ir::AsyncFunctionAbi {
                completion: wasm_ir::AsyncCompletion::FrameValue(ty),
            }) => Some(semantic_type(
                semantics.specialize_type(instance, ty),
                semantics,
            )),
            wasm_ir::BodyAbi::AsyncFunction(wasm_ir::AsyncFunctionAbi {
                completion: wasm_ir::AsyncCompletion::Void,
            }) => None,
            wasm_ir::BodyAbi::Direct | wasm_ir::BodyAbi::AttachPoll => {
                unreachable!("only suspending functions receive typed future frames")
            }
        };
        Self::for_body(
            body,
            wasm_ir,
            semantics,
            Some(instance),
            2,
            declaration.params.iter().map(|parameter| parameter.id),
        )
        .with_completion(completion)
    }

    fn for_body(
        body: &wasm_ir::Body,
        program: &wasm_ir::Program,
        semantics: &SemanticModel,
        instance: Option<&FunctionInstance>,
        base_fields: u32,
        parameters: impl IntoIterator<Item = ValueId>,
    ) -> Self {
        let mut layout = Self {
            base_fields,
            ..Self::default()
        };
        for value in parameters {
            let source = semantics
                .value_type(value)
                .expect("checked parameters have semantic types");
            let source = instance.map_or(source, |instance| {
                semantics.specialize_type(instance, source)
            });
            layout.push_value(value, semantic_type(source, semantics));
        }
        for local in &body.locals {
            let destination = match local.purpose {
                LocalPurpose::Value(value) if body.frame_values.contains(&value) => Some(Ok(value)),
                LocalPurpose::Temporary(temporary)
                    if body.frame_temporaries.contains(&temporary) =>
                {
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
                        layout.push_value(value, ty);
                    }
                    Err(temporary) => {
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
                    let is_intrinsic = matches!(
                        expression.kind,
                        wasm_ir::ExpressionKind::Call {
                            target: wasm_ir::CallTarget::Intrinsic { .. },
                            ..
                        }
                    );
                    let ty = self.owner.map_or(expression.ty, |owner| {
                        self.semantics.specialize_type(owner, expression.ty)
                    });
                    let ty = semantic_type(ty, self.semantics);
                    if matches!(ty, Type::Async(_)) && !is_intrinsic {
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
        wasm_ir::Visitor::visit_block(&mut children, &body.entry, program);
        for (expression, ty) in children.values {
            let field = layout.base_fields + layout.types.len() as u32;
            layout.children.insert(expression, (field, ty));
            layout.types.push(ty);
        }
        layout
    }

    fn push_value(&mut self, value: ValueId, ty: Type) {
        if self.fields.contains_key(&value) {
            return;
        }
        let field = self.base_fields + self.types.len() as u32;
        self.fields.insert(value, (field, ty));
        self.types.push(ty);
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
        match destination {
            wasm_ir::SuspensionDestination::SourceValue(value) => self.fields.get(&value).copied(),
            wasm_ir::SuspensionDestination::Temporary(temporary) => {
                self.temporaries.get(&temporary).copied()
            }
            wasm_ir::SuspensionDestination::BodyResult => self.completion,
            wasm_ir::SuspensionDestination::Discard => None,
        }
    }
}

#[derive(Default)]
pub(super) struct AsyncFrameLayouts {
    pub attach: Option<AsyncFrameLayout>,
    functions: HashMap<FunctionInstance, AsyncFrameLayout>,
    ordered_functions: Vec<FunctionInstance>,
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
        Self {
            attach,
            functions,
            ordered_functions,
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
}
