//! Lazily planned `Display` overrides and compiler-provided `Debug` implementations.

use std::collections::{BTreeMap, HashMap};

use crate::{capabilities::DerivedDebugKind, semantic::FunctionInstance, types::TypeId};

#[derive(Debug, Clone, Copy)]
pub(super) struct DerivedDebugFunction {
    pub function: u32,
    pub kind: DerivedDebugKind,
}

#[derive(Debug, Default)]
pub(super) struct DisplayFunctions {
    /// Source-defined user-facing `Display.toString` overrides, including
    /// privileged standard-library bodies.
    pub custom: HashMap<TypeId, FunctionInstance>,
    /// Source-defined structural `Debug.debugString` overrides.
    pub custom_debug: HashMap<TypeId, FunctionInstance>,
    /// Compiler-derived structural or opaque `Debug` formatters for reachable
    /// concrete types.
    /// Kept in declaration/body emission order so function indices and bodies
    /// cannot diverge through randomized `HashMap` iteration.
    pub derived: BTreeMap<TypeId, DerivedDebugFunction>,
}
