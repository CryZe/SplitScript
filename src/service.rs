//! Versioned in-memory compiler service shared by embedded hosts.
//!
//! Native command-line binaries deliberately remain responsible for files,
//! watching, terminal rendering, and process lifetime. This module owns only
//! values that can cross an editor or worker boundary without relying on an
//! operating-system path or stderr convention.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BuildProfile, CompilerContext, CompilerIdentity, CompilerOptions, Diagnostic, DiagnosticFix,
    DiagnosticLabel, DiagnosticLabelStyle, DiagnosticSeverity, FixApplicability, TextEdit,
    WarningPolicy, compile_named_with_context_and_options_diagnostics, compiler_identity,
};

pub const COMPILER_SERVICE_PROTOCOL_VERSION: u32 = 1;
pub const MAX_COMPILER_SERVICE_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    pub protocol_version: u32,
    pub uri: String,
    /// Native filesystem path when the host has one. Browser-only or untitled
    /// inputs may omit it and retain their URI as the DWARF source identity.
    #[serde(default)]
    pub source_path: Option<String>,
    pub revision: u64,
    pub source: String,
    pub profile: BuildProfile,
    #[serde(default)]
    pub warnings: WarningPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResponse {
    pub protocol_version: u32,
    pub compiler: CompilerIdentity,
    pub uri: String,
    pub revision: u64,
    pub diagnostics: Vec<ServiceDiagnostic>,
    pub artifact: Option<Vec<u8>>,
}

impl CompileResponse {
    pub fn succeeded(&self) -> bool {
        self.artifact.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompileServiceErrorCode {
    UnsupportedProtocol,
    SourceTooLarge,
    InvalidRequest,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileServiceError {
    pub code: CompileServiceErrorCode,
    pub message: String,
}

impl CompileServiceError {
    pub fn new(code: CompileServiceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CompileServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CompileServiceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceDiagnosticLabelStyle {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceFixApplicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDiagnosticLabel {
    pub style: ServiceDiagnosticLabelStyle,
    pub span: ServiceSpan,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTextEdit {
    pub span: ServiceSpan,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDiagnosticFix {
    pub title: String,
    pub applicability: ServiceFixApplicability,
    pub edits: Vec<ServiceTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDiagnostic {
    pub code: String,
    pub severity: ServiceDiagnosticSeverity,
    pub message: String,
    pub span: ServiceSpan,
    pub labels: Vec<ServiceDiagnosticLabel>,
    pub notes: Vec<String>,
    pub fixes: Vec<ServiceDiagnosticFix>,
}

#[derive(Debug, Clone)]
pub struct CompilerService {
    context: CompilerContext,
}

impl Default for CompilerService {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilerService {
    pub fn new() -> Self {
        Self {
            context: CompilerContext::default(),
        }
    }

    pub fn with_context(context: CompilerContext) -> Self {
        Self { context }
    }

    pub fn compile(&self, request: CompileRequest) -> Result<CompileResponse, CompileServiceError> {
        if request.protocol_version != COMPILER_SERVICE_PROTOCOL_VERSION {
            return Err(CompileServiceError {
                code: CompileServiceErrorCode::UnsupportedProtocol,
                message: format!(
                    "unsupported compiler-service protocol {}; expected {}",
                    request.protocol_version, COMPILER_SERVICE_PROTOCOL_VERSION
                ),
            });
        }
        if request.source.len() > MAX_COMPILER_SERVICE_SOURCE_BYTES {
            return Err(CompileServiceError {
                code: CompileServiceErrorCode::SourceTooLarge,
                message: format!(
                    "source is {} bytes; the embedded compiler limit is {} bytes",
                    request.source.len(),
                    MAX_COMPILER_SERVICE_SOURCE_BYTES
                ),
            });
        }

        let CompileRequest {
            protocol_version,
            uri,
            source_path,
            revision,
            source,
            profile,
            warnings,
        } = request;
        let source_name = source_path.as_deref().unwrap_or(&uri);
        let (diagnostics, artifact) = match compile_named_with_context_and_options_diagnostics(
            self.context.clone(),
            source_name,
            &source,
            CompilerOptions { profile, warnings },
        ) {
            Ok((artifact, diagnostics)) => (
                diagnostics
                    .into_iter()
                    .map(ServiceDiagnostic::from)
                    .collect(),
                Some(artifact),
            ),
            Err(diagnostics) => (
                diagnostics
                    .into_iter()
                    .map(ServiceDiagnostic::from)
                    .collect(),
                None,
            ),
        };
        Ok(CompileResponse {
            protocol_version,
            compiler: compiler_identity(),
            uri,
            revision,
            diagnostics,
            artifact,
        })
    }
}

impl From<Diagnostic> for ServiceDiagnostic {
    fn from(diagnostic: Diagnostic) -> Self {
        Self {
            code: diagnostic.code.as_str().to_owned(),
            severity: diagnostic.severity.into(),
            message: diagnostic.message,
            span: ServiceSpan {
                start: diagnostic.span.start,
                end: diagnostic.span.end,
            },
            labels: diagnostic
                .labels
                .into_iter()
                .map(ServiceDiagnosticLabel::from)
                .collect(),
            notes: diagnostic.notes,
            fixes: diagnostic
                .fixes
                .into_iter()
                .map(ServiceDiagnosticFix::from)
                .collect(),
        }
    }
}

impl From<DiagnosticSeverity> for ServiceDiagnosticSeverity {
    fn from(severity: DiagnosticSeverity) -> Self {
        match severity {
            DiagnosticSeverity::Error => Self::Error,
            DiagnosticSeverity::Warning => Self::Warning,
            DiagnosticSeverity::Information => Self::Information,
            DiagnosticSeverity::Hint => Self::Hint,
        }
    }
}

impl From<DiagnosticLabel> for ServiceDiagnosticLabel {
    fn from(label: DiagnosticLabel) -> Self {
        Self {
            style: match label.style {
                DiagnosticLabelStyle::Primary => ServiceDiagnosticLabelStyle::Primary,
                DiagnosticLabelStyle::Secondary => ServiceDiagnosticLabelStyle::Secondary,
            },
            span: ServiceSpan {
                start: label.span.start,
                end: label.span.end,
            },
            message: label.message,
        }
    }
}

impl From<DiagnosticFix> for ServiceDiagnosticFix {
    fn from(fix: DiagnosticFix) -> Self {
        Self {
            title: fix.title,
            applicability: match fix.applicability {
                FixApplicability::MachineApplicable => ServiceFixApplicability::MachineApplicable,
                FixApplicability::MaybeIncorrect => ServiceFixApplicability::MaybeIncorrect,
                FixApplicability::HasPlaceholders => ServiceFixApplicability::HasPlaceholders,
                FixApplicability::Unspecified => ServiceFixApplicability::Unspecified,
            },
            edits: fix.edits.into_iter().map(ServiceTextEdit::from).collect(),
        }
    }
}

impl From<TextEdit> for ServiceTextEdit {
    fn from(edit: TextEdit) -> Self {
        Self {
            span: ServiceSpan {
                start: edit.span.start,
                end: edit.span.end,
            },
            replacement: edit.replacement,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(source: &str) -> CompileRequest {
        CompileRequest {
            protocol_version: COMPILER_SERVICE_PROTOCOL_VERSION,
            uri: "file:///example.split".to_owned(),
            source_path: None,
            revision: 42,
            source: source.to_owned(),
            profile: BuildProfile::Release,
            warnings: WarningPolicy::default(),
        }
    }

    #[test]
    fn returns_artifact_for_the_exact_source_revision() {
        let response = CompilerService::new()
            .compile(request("state \"game.exe\" {}"))
            .expect("the protocol request should be accepted");
        assert!(response.succeeded());
        assert_eq!(response.uri, "file:///example.split");
        assert_eq!(response.revision, 42);
        assert!(response.diagnostics.is_empty());
        assert!(response.artifact.unwrap().starts_with(b"\0asm"));
    }

    #[test]
    fn embeds_the_host_source_path_in_debug_artifacts() {
        let source_path = "P:/Games/Example/autosplitter.split";
        let mut request = request(
            "state \"game.exe\" {} fn value() { return 42 } whileAttached { print(value()) }",
        );
        request.profile = BuildProfile::Debug;
        request.source_path = Some(source_path.to_owned());
        let response = CompilerService::new()
            .compile(request)
            .expect("the compiler service request should succeed");
        let artifact = response.artifact.expect("debug compilation should succeed");
        assert!(
            artifact
                .windows(source_path.len())
                .any(|window| window == source_path.as_bytes()),
            "the native source path should be present in embedded DWARF"
        );
    }

    #[test]
    fn compilation_failure_is_a_revisioned_response() {
        let response = CompilerService::new()
            .compile(request("fn broken( {"))
            .expect("compiler diagnostics are not protocol failures");
        assert!(!response.succeeded());
        assert_eq!(response.revision, 42);
        assert_eq!(
            response.diagnostics[0].severity,
            ServiceDiagnosticSeverity::Error
        );
        assert!(response.diagnostics[0].code.starts_with("SS"));
    }

    #[test]
    fn successful_compilation_preserves_warnings() {
        let response = CompilerService::new()
            .compile(request(
                r#"state "game.exe" {} whileAttached { "abc".replaceAll("a", "b") }"#,
            ))
            .expect("warnings are not protocol failures");
        assert!(response.succeeded());
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(
            response.diagnostics[0].severity,
            ServiceDiagnosticSeverity::Warning
        );
        assert!(response.diagnostics[0].message.contains("replaceAll"));
    }

    #[test]
    fn warning_policy_crosses_the_embedded_service_boundary() {
        let mut request =
            request(r#"state "game.exe" {} whileAttached { "abc".replaceAll("a", "b") }"#);
        assert!(
            request
                .warnings
                .set(crate::DiagnosticCode::MustUse, crate::WarningLevel::Deny)
        );
        let response = CompilerService::new()
            .compile(request)
            .expect("policy denial is a compiler result, not a protocol error");
        assert!(!response.succeeded());
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(response.diagnostics[0].code, "SS1001");
        assert_eq!(
            response.diagnostics[0].severity,
            ServiceDiagnosticSeverity::Error
        );
    }

    #[test]
    fn rejects_an_unknown_protocol_before_compiling() {
        let mut request = request("state \"game.exe\" {}");
        request.protocol_version += 1;
        let error = CompilerService::new().compile(request).unwrap_err();
        assert_eq!(error.code, CompileServiceErrorCode::UnsupportedProtocol);
    }
}
