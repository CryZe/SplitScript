//! Function indices reserved for structural equality implementations.

use std::collections::HashMap;

use crate::{
    ast::{EnumId, OptionTypeId, RecordId, ResultTypeId},
    stdlib::{StandardLibrary, StdlibTypeId},
};

#[derive(Default)]
pub(super) struct EqualityFunctions {
    pub standard_library: StandardLibrary,
    pub standard_records: HashMap<StdlibTypeId, u32>,
    pub records: HashMap<RecordId, u32>,
    pub enums: HashMap<EnumId, u32>,
    pub options: HashMap<OptionTypeId, u32>,
    pub results: HashMap<ResultTypeId, u32>,
}
