//! Per-revision query storage. One reset invalidates the complete dependency set.

use std::sync::Arc;

use crate::{
    CheckedProgram, Diagnostic, LoweredProgram, ParsedProgram, RecoveredCheck, RecoveredParse,
    highlight::SemanticHighlightIndex,
};

use super::{DefinitionIndex, QueryResult, ReferenceIndex, SemanticSnapshot};

#[derive(Debug, Default)]
pub(super) struct QueryCache {
    pub recovered: Option<QueryResult<RecoveredParse>>,
    pub recovering_lowered: Option<QueryResult<LoweredProgram>>,
    pub parsed: Option<QueryResult<ParsedProgram>>,
    pub lowered: Option<QueryResult<LoweredProgram>>,
    pub checked: Option<QueryResult<CheckedProgram>>,
    pub recovering_checked: Option<QueryResult<RecoveredCheck>>,
    pub semantic_snapshot: Option<QueryResult<SemanticSnapshot>>,
    pub references: Option<QueryResult<ReferenceIndex>>,
    pub definitions: Option<QueryResult<DefinitionIndex>>,
    pub highlights: Option<QueryResult<SemanticHighlightIndex>>,
    pub document_symbols: Option<QueryResult<Vec<crate::symbols::DocumentSymbol>>>,
    pub formatted: Option<QueryResult<String>>,
    pub diagnostics: Option<Arc<[Diagnostic]>>,
}
