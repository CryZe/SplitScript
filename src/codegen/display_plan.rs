//! Lazily planned `Display` overrides and structural `Debug` implementations.

use std::collections::{BTreeMap, HashMap};

use crate::{semantic::FunctionInstance, types::TypeId};

#[derive(Debug, Default)]
pub(super) struct DisplayFunctions {
    /// Source-defined user-facing `Display.toString` overrides, including
    /// privileged standard-library bodies.
    pub custom: HashMap<TypeId, FunctionInstance>,
    /// Source-defined structural `Debug.debugString` overrides.
    pub custom_debug: HashMap<TypeId, FunctionInstance>,
    /// Compiler-derived `Debug` formatters for reachable concrete types.
    /// Kept in declaration/body emission order so function indices and bodies
    /// cannot diverge through randomized `HashMap` iteration.
    pub derived: BTreeMap<TypeId, u32>,
}
