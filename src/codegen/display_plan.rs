//! Lazily planned implementations of the `Display` capability.

use std::collections::{BTreeMap, HashMap};

use crate::{semantic::FunctionInstance, types::TypeId};

#[derive(Debug, Default)]
pub(super) struct DisplayFunctions {
    /// Source-defined overrides, including privileged standard-library bodies.
    pub custom: HashMap<TypeId, FunctionInstance>,
    /// Compiler-derived structural formatters for reachable concrete types.
    /// Kept in declaration/body emission order so function indices and bodies
    /// cannot diverge through randomized `HashMap` iteration.
    pub derived: BTreeMap<TypeId, u32>,
}
