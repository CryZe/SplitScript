//! Immutable context shared by Wasm body emitters after all plans are fixed.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, RecordDecl, ValueId},
    memory::MemoryLayouts,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{StandardLibrary, StdlibTypeId},
    types::ResolvedArrayType,
    wasm_ir,
};

use super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, SettingStorage, Type,
    data_plan::{SignaturePool, StringPool},
    global_plan::RuntimeGlobals,
    imports::Abi,
    memory_plan::AbiReadScratch,
};

/// Shared immutable inputs needed while emitting script-owned bodies.
///
/// This is intentionally constructed only after type, function, global,
/// memory, helper, and GC plans are complete. Individual emitters should take
/// a narrower input when they do not need this complete view.
pub(super) struct EmissionContext<'a> {
    pub standard_library: &'a StandardLibrary,
    pub abi: &'a Abi,
    pub state: &'a crate::ast::StateDecl,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub settings: &'a HashMap<ValueId, SettingStorage>,
    pub runtime_globals: RuntimeGlobals,
    pub runtime_helpers: &'a RuntimeHelperPlan,
    pub functions: &'a HashMap<FunctionInstance, super::function_plan::UserFunctionPlan>,
    pub intrinsic_futures: &'a HashMap<super::async_frame::IntrinsicFutureInstance, u32>,
    pub display_functions: &'a HashMap<StdlibTypeId, FunctionInstance>,
    pub equality_functions: &'a EqualityFunctions,
    pub records: &'a [RecordDecl],
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub memory: &'a MemoryLayouts,
    pub abi_read: AbiReadScratch,
    pub signatures: &'a SignaturePool,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
    pub gc: &'a GcLayout,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
}

/// Extra pools required only by the async attachment state machine.
pub(super) struct AttachContext<'a> {
    pub abi: &'a Abi,
    pub strings: &'a StringPool,
    pub lowering: &'a EmissionContext<'a>,
}
