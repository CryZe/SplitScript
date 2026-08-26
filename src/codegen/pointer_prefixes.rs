//! Common-subexpression planning for ordinary process-backed state paths.
//!
//! A candidate snapshot is assembled by one invocation of the generated
//! `update` function. Its locals therefore form the natural transaction scope:
//! module lookups and pointer dereferences shared by multiple state fields can
//! be evaluated once and reused without persistent cache state.

use std::collections::{HashMap, HashSet};

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{PointerPathBase, Program, StateSource, ValueId},
    semantic::SemanticModel,
    stdlib::{Implementation, IntrinsicId, StandardLibrary},
};

use super::{data_plan::StringPool, memarg, memory_plan::AbiReadScratch};

const PREFIX_UNRESOLVED: i32 = 0;
pub(super) const PREFIX_RESOLVED: i32 = 1;
pub(super) const PREFIX_MODULE_MISSING: i32 = 2;
pub(super) const PREFIX_READ_FAILED: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct PrefixId(usize);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PrefixOperation {
    Absolute(u64),
    Module(String),
    Add { parent: PrefixId, offset: i64 },
    Dereference { parent: PrefixId },
}

impl PrefixOperation {
    fn is_costly(&self) -> bool {
        matches!(self, Self::Module(_) | Self::Dereference { .. })
    }

    fn parent(&self) -> Option<PrefixId> {
        match self {
            Self::Add { parent, .. } | Self::Dereference { parent } => Some(*parent),
            Self::Absolute(_) | Self::Module(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
struct PrefixNode {
    operation: PrefixOperation,
    fields: Vec<ValueId>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FieldPrefix {
    pub prefix: PrefixId,
    /// The first source pointer offset after the adjustment below.
    pub offset_start: usize,
    /// Shared nodes retain only costly host results. The source displacement
    /// following a module lookup or raw pointer dereference remains a cheap
    /// field-local addition.
    pub initial_offset: i64,
}

#[derive(Default)]
pub(super) struct PointerPrefixPlan {
    nodes: Vec<PrefixNode>,
    fields: HashMap<ValueId, FieldPrefix>,
    shared: Vec<PrefixId>,
}

impl PointerPrefixPlan {
    pub(super) fn build(
        program: &Program,
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
    ) -> Self {
        let Some(provider) = semantics.state_provider() else {
            return Self::default();
        };
        let Implementation::Intrinsic(read) = standard_library
            .item(standard_library.state_provider(provider).direct_read)
            .implementation
        else {
            return Self::default();
        };
        if read != IntrinsicId::ProcessRead {
            return Self::default();
        }

        let Some(state) = &program.state else {
            return Self::default();
        };
        let mut nodes = Vec::<PrefixNode>::new();
        let mut canonical = HashMap::<PrefixOperation, PrefixId>::new();
        let mut field_paths = Vec::<(ValueId, Vec<(PrefixId, usize, i64)>)>::new();

        let intern = |operation: PrefixOperation,
                      nodes: &mut Vec<PrefixNode>,
                      canonical: &mut HashMap<PrefixOperation, PrefixId>| {
            if let Some(id) = canonical.get(&operation) {
                return *id;
            }
            let id = PrefixId(nodes.len());
            nodes.push(PrefixNode {
                operation: operation.clone(),
                fields: Vec::new(),
            });
            canonical.insert(operation, id);
            id
        };

        for field in state.all_fields() {
            let StateSource::Pointer(path) = &field.source else {
                continue;
            };
            let mut path_nodes = Vec::new();
            let mut current = match &path.base {
                PointerPathBase::Absolute(address) => intern(
                    PrefixOperation::Absolute(*address),
                    &mut nodes,
                    &mut canonical,
                ),
                PointerPathBase::Module { name, offset } => {
                    let module = intern(
                        PrefixOperation::Module(name.clone()),
                        &mut nodes,
                        &mut canonical,
                    );
                    path_nodes.push((module, 0, *offset));
                    intern(
                        PrefixOperation::Add {
                            parent: module,
                            offset: *offset,
                        },
                        &mut nodes,
                        &mut canonical,
                    )
                }
            };
            for (offset_index, offset) in path.offsets.iter().copied().enumerate() {
                let dereference = intern(
                    PrefixOperation::Dereference { parent: current },
                    &mut nodes,
                    &mut canonical,
                );
                path_nodes.push((dereference, offset_index + 1, offset));
                current = intern(
                    PrefixOperation::Add {
                        parent: dereference,
                        offset,
                    },
                    &mut nodes,
                    &mut canonical,
                );
            }
            for (node, _, _) in &path_nodes {
                nodes[node.0].fields.push(field.id);
            }
            field_paths.push((field.id, path_nodes));
        }

        let named_layouts = state
            .layouts
            .iter()
            .enumerate()
            .flat_map(|(layout, declaration)| {
                declaration
                    .fields
                    .iter()
                    .map(move |field| (field.id, layout))
            })
            .collect::<HashMap<_, _>>();
        let can_share = |left: ValueId, right: ValueId| {
            if !state.layouts.is_empty() {
                return named_layouts.get(&left) == named_layouts.get(&right);
            }
            let left = semantics.state_field_layout_constraints(left);
            let right = semantics.state_field_layout_constraints(right);
            !left.iter().any(|left| {
                right
                    .iter()
                    .any(|right| left.dimension == right.dimension && left.variant != right.variant)
            })
        };
        let shared = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.operation.is_costly()
                    && node.fields.iter().enumerate().any(|(index, left)| {
                        node.fields[index + 1..]
                            .iter()
                            .any(|right| can_share(*left, *right))
                    })
            })
            .map(|(index, _)| PrefixId(index))
            .collect::<Vec<_>>();
        let is_shared = shared.iter().copied().collect::<HashSet<_>>();
        let mut fields = HashMap::new();
        for (field, path) in field_paths {
            let Some((prefix, consumed_offsets, initial_offset)) = path
                .into_iter()
                .rev()
                .find(|(prefix, _, _)| is_shared.contains(prefix))
            else {
                continue;
            };
            fields.insert(
                field,
                FieldPrefix {
                    prefix,
                    offset_start: consumed_offsets,
                    initial_offset,
                },
            );
        }

        Self {
            nodes,
            fields,
            shared,
        }
    }

    pub(super) fn field(&self, field: ValueId) -> Option<FieldPrefix> {
        self.fields.get(&field).copied()
    }

    pub(super) fn allocate_locals(&self, first: u32) -> (Vec<(u32, ValType)>, PrefixLocals) {
        let mut declarations = Vec::with_capacity(self.shared.len() * 2);
        let mut storage = HashMap::with_capacity(self.shared.len());
        let mut next = first;
        for prefix in &self.shared {
            let address = next;
            let status = next + 1;
            next += 2;
            declarations.push((1, ValType::I64));
            declarations.push((1, ValType::I32));
            storage.insert(*prefix, PrefixStorage { address, status });
        }
        (declarations, PrefixLocals { storage })
    }
}

#[derive(Clone, Copy)]
struct PrefixStorage {
    address: u32,
    status: u32,
}

pub(super) struct PrefixLocals {
    storage: HashMap<PrefixId, PrefixStorage>,
}

#[derive(Default)]
pub(super) struct PrefixEmissionState {
    guaranteed: HashSet<PrefixId>,
    possibly_initialized: HashSet<PrefixId>,
}

pub(super) struct PrefixEmissionContext<'a> {
    pub plan: &'a PointerPrefixPlan,
    pub strings: &'a StringPool,
    pub abi: &'a super::imports::Abi,
    pub process_global: u32,
    pub abi_read: AbiReadScratch,
}

impl PrefixLocals {
    pub(super) fn emit_field_prefix(
        &self,
        function: &mut Function,
        field: FieldPrefix,
        context: &PrefixEmissionContext<'_>,
        emission: &mut PrefixEmissionState,
        conditional: bool,
    ) {
        self.emit_ensure(function, field.prefix, context, emission, conditional);
        let storage = self.storage[&field.prefix];
        function
            .instruction(&Instruction::LocalGet(storage.address))
            .instruction(&Instruction::LocalGet(storage.status));
    }

    fn emit_ensure(
        &self,
        function: &mut Function,
        prefix: PrefixId,
        context: &PrefixEmissionContext<'_>,
        emission: &mut PrefixEmissionState,
        conditional: bool,
    ) {
        if emission.guaranteed.contains(&prefix) {
            return;
        }
        let parent_prefix = match context.plan.nodes[prefix.0].operation {
            PrefixOperation::Dereference { parent } => self.shared_ancestor(parent, context.plan),
            PrefixOperation::Module(_) => None,
            PrefixOperation::Absolute(_) | PrefixOperation::Add { .. } => {
                unreachable!("only costly prefixes receive update locals")
            }
        };
        if let Some(parent_prefix) = parent_prefix {
            self.emit_ensure(function, parent_prefix, context, emission, conditional);
        }

        let storage = self.storage[&prefix];
        let guarded = conditional || emission.possibly_initialized.contains(&prefix);
        if guarded {
            function
                .instruction(&Instruction::LocalGet(storage.status))
                .instruction(&Instruction::I32Const(PREFIX_UNRESOLVED))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty));
        }
        match &context.plan.nodes[prefix.0].operation {
            PrefixOperation::Module(name) => {
                let (pointer, length) = context.strings.get(name);
                function
                    .instruction(&Instruction::GlobalGet(context.process_global))
                    .instruction(&Instruction::I32Const(pointer as i32))
                    .instruction(&Instruction::I32Const(length as i32))
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::ProcessGetModuleAddress),
                    ))
                    .instruction(&Instruction::LocalTee(storage.address))
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(PREFIX_MODULE_MISSING))
                    .instruction(&Instruction::LocalSet(storage.status))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::I32Const(PREFIX_RESOLVED))
                    .instruction(&Instruction::LocalSet(storage.status))
                    .instruction(&Instruction::End);
            }
            PrefixOperation::Dereference { parent } => {
                if let Some(parent_prefix) = parent_prefix {
                    function
                        .instruction(&Instruction::LocalGet(self.storage[&parent_prefix].status))
                        .instruction(&Instruction::LocalSet(storage.status));
                } else {
                    function
                        .instruction(&Instruction::I32Const(PREFIX_RESOLVED))
                        .instruction(&Instruction::LocalSet(storage.status));
                }
                function
                    .instruction(&Instruction::LocalGet(storage.status))
                    .instruction(&Instruction::I32Const(PREFIX_RESOLVED))
                    .instruction(&Instruction::I32Eq)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::GlobalGet(context.process_global));
                self.emit_address(function, *parent, context.plan);
                function
                    .instruction(&Instruction::I32Const(context.abi_read.destination(8)))
                    .instruction(&Instruction::I32Const(8))
                    .instruction(&Instruction::Call(
                        context.abi.function(AbiImportId::ProcessRead),
                    ))
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(context.abi_read.start()))
                    .instruction(&Instruction::I64Load(memarg()))
                    .instruction(&Instruction::LocalSet(storage.address))
                    .instruction(&Instruction::Else)
                    .instruction(&Instruction::I32Const(PREFIX_READ_FAILED))
                    .instruction(&Instruction::LocalSet(storage.status))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            PrefixOperation::Absolute(_) | PrefixOperation::Add { .. } => {
                unreachable!("only costly prefixes receive update locals")
            }
        }
        if guarded {
            function.instruction(&Instruction::End);
        }
        emission.possibly_initialized.insert(prefix);
        if !conditional {
            emission.guaranteed.insert(prefix);
        }
    }

    fn shared_ancestor(&self, mut prefix: PrefixId, plan: &PointerPrefixPlan) -> Option<PrefixId> {
        loop {
            if self.storage.contains_key(&prefix) {
                return Some(prefix);
            }
            prefix = plan.nodes[prefix.0].operation.parent()?;
        }
    }

    fn emit_address(&self, function: &mut Function, prefix: PrefixId, plan: &PointerPrefixPlan) {
        if let Some(storage) = self.storage.get(&prefix) {
            function.instruction(&Instruction::LocalGet(storage.address));
            return;
        }
        match plan.nodes[prefix.0].operation {
            PrefixOperation::Absolute(address) => {
                function.instruction(&Instruction::I64Const(address as i64));
            }
            PrefixOperation::Add { parent, offset } => {
                self.emit_address(function, parent, plan);
                function
                    .instruction(&Instruction::I64Const(offset))
                    .instruction(&Instruction::I64Add);
            }
            PrefixOperation::Module(_) | PrefixOperation::Dereference { .. } => {
                unreachable!("a costly ancestor of a shared prefix is also shared")
            }
        }
    }
}
