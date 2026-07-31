//! Stable editor/tooling-facing API.
//!
//! Queries are editor-neutral. Protocol and editor clients should adapt these
//! products rather than reaching into parser, checker, or backend internals.

pub use crate::{
    Diagnostic, DiagnosticCode, DiagnosticFix, DiagnosticLabel, DiagnosticLabelStyle,
    DiagnosticSeverity, FixApplicability, TextEdit, format_source,
};

/// Revisioned compiler queries and semantic snapshots.
pub mod database {
    pub use crate::database::*;
}

/// Completion candidates and snippets.
pub mod completion {
    pub use crate::completion::*;
}

/// Normalized standard-library documentation views.
pub mod documentation {
    pub use crate::documentation::*;
}

/// Semantic highlighting tokens.
pub mod highlight {
    pub use crate::highlight::*;
}

/// Hover and signature-help products.
pub mod insight {
    pub use crate::insight::*;
}

/// Compiler-owned syntax, built-in, and lifecycle catalog.
pub mod language {
    pub use crate::language::*;
}

/// In-process Language Server Protocol handler.
pub mod lsp {
    pub use crate::lsp::*;
}

/// Document outline products.
pub mod symbols {
    pub use crate::symbols::*;
}
