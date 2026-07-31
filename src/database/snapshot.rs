//! Editor-safe semantic facts from either strict or recovering checking.

use std::sync::Arc;

use crate::{
    CheckedProgram, CompilerContext, RecoveredCheck,
    ast::{EnumDecl, Program},
    effects::OperationAnalysis,
    hir::TypedProgram,
    semantic::SemanticModel,
    syntax::SourceDocument,
};

#[derive(Debug, Clone)]
pub enum SemanticSnapshot {
    Checked(Arc<CheckedProgram>),
    Recovered(Arc<RecoveredCheck>),
}

impl SemanticSnapshot {
    pub fn context(&self) -> CompilerContext {
        match self {
            Self::Checked(program) => program.context(),
            Self::Recovered(program) => program.context(),
        }
    }

    pub fn semantics(&self) -> &SemanticModel {
        match self {
            Self::Checked(program) => program.semantics(),
            Self::Recovered(program) => program.semantics(),
        }
    }

    pub fn syntax(&self) -> &Program {
        match self {
            Self::Checked(program) => program.syntax(),
            Self::Recovered(program) => program.syntax(),
        }
    }

    pub fn source_document(&self) -> &SourceDocument {
        match self {
            Self::Checked(program) => program.source_document(),
            Self::Recovered(program) => program.source_document(),
        }
    }

    pub fn enum_types(&self) -> &[EnumDecl] {
        match self {
            Self::Checked(program) => program.enum_types(),
            Self::Recovered(program) => program.enum_types(),
        }
    }

    pub fn typed_hir(&self) -> Option<&TypedProgram> {
        match self {
            Self::Checked(program) => Some(program.typed_hir()),
            Self::Recovered(_) => None,
        }
    }

    pub fn effects(&self) -> Option<&OperationAnalysis> {
        match self {
            Self::Checked(program) => Some(program.effects()),
            Self::Recovered(program) => program.effects(),
        }
    }

    pub fn checked(&self) -> Option<&CheckedProgram> {
        match self {
            Self::Checked(program) => Some(program),
            Self::Recovered(_) => None,
        }
    }
}
