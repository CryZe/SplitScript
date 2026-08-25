//! Immutable context shared by Wasm body emitters after all plans are fixed.

use std::collections::HashMap;

use crate::{
    ast::{EnumDecl, RecordDecl, ValueId},
    managed::ManagedBindingPlan,
    memory::MemoryLayouts,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::StandardLibrary,
    types::ResolvedArrayType,
    wasm_ir,
};

use super::{
    ArrayFunctions, DisplayFunctions, EqualityFunctions, GcLayout, RuntimeHelperPlan, SetFunctions,
    SettingStorage, Type,
    data_plan::{SignaturePool, StringPool},
    debug_artifacts::DebugRecorder,
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
    pub reachability: &'a super::reachability::Reachability,
    pub capabilities: &'a crate::capabilities::CapabilityAnalysis,
    pub abi: &'a Abi,
    pub state: &'a crate::ast::StateDecl,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub settings: &'a HashMap<ValueId, SettingStorage>,
    pub runtime_globals: RuntimeGlobals,
    pub runtime_helpers: &'a RuntimeHelperPlan,
    pub functions: &'a HashMap<FunctionInstance, super::function_plan::UserFunctionPlan>,
    pub closures: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub function_values: &'a HashMap<crate::semantic::FunctionValueInstance, u32>,
    pub closure_polls: &'a HashMap<crate::semantic::ClosureInstance, u32>,
    pub intrinsic_futures: &'a HashMap<super::async_frame::IntrinsicFutureInstance, u32>,
    pub display_functions: &'a DisplayFunctions,
    pub equality_functions: &'a EqualityFunctions,
    pub array_functions: &'a ArrayFunctions,
    pub set_functions: &'a SetFunctions,
    pub records: &'a [RecordDecl],
    pub managed: &'a ManagedBindingPlan,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub memory: &'a MemoryLayouts,
    pub abi_read: AbiReadScratch,
    pub signatures: &'a SignaturePool,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
    pub gc: &'a GcLayout,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub debug: Option<&'a DebugRecorder>,
}

impl<'a> EmissionContext<'a> {
    pub fn debug_emission(
        &self,
        function: u32,
    ) -> Option<super::debug_artifacts::DebugEmission<'a>> {
        self.debug.map(|recorder| recorder.emission(function))
    }
}

/// Extra pools required only by the async attachment state machine.
pub(super) struct AttachContext<'a> {
    pub abi: &'a Abi,
    pub strings: &'a StringPool,
    pub lowering: &'a EmissionContext<'a>,
}
