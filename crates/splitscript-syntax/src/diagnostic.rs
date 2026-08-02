use std::fmt;
use std::str::FromStr;

use crate::Span;

/// Stable identifier for a class of compiler diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    Lexical,
    Syntax,
    Type,
    Semantic,
    MustUse,
    UnusedBinding,
    UnusedDeclaration,
    UnusedMember,
}

impl DiagnosticCode {
    pub const WARNINGS: [Self; 4] = [
        Self::MustUse,
        Self::UnusedBinding,
        Self::UnusedDeclaration,
        Self::UnusedMember,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "SS0001",
            Self::Syntax => "SS0002",
            Self::Type => "SS0003",
            Self::Semantic => "SS0004",
            Self::MustUse => "SS1001",
            Self::UnusedBinding => "SS1002",
            Self::UnusedDeclaration => "SS1003",
            Self::UnusedMember => "SS1004",
        }
    }

    pub const fn is_warning(self) -> bool {
        matches!(
            self,
            Self::MustUse | Self::UnusedBinding | Self::UnusedDeclaration | Self::UnusedMember
        )
    }
}

impl FromStr for DiagnosticCode {
    type Err = ();

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        match code.to_ascii_uppercase().as_str() {
            "SS0001" => Ok(Self::Lexical),
            "SS0002" => Ok(Self::Syntax),
            "SS0003" => Ok(Self::Type),
            "SS0004" => Ok(Self::Semantic),
            "SS1001" => Ok(Self::MustUse),
            "SS1002" => Ok(Self::UnusedBinding),
            "SS1003" => Ok(Self::UnusedDeclaration),
            "SS1004" => Ok(Self::UnusedMember),
            _ => Err(()),
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticLabelStyle {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub style: DiagnosticLabelStyle,
    pub span: Span,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixApplicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

impl fmt::Display for FixApplicability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MachineApplicable => "machine-applicable",
            Self::MaybeIncorrect => "maybe-incorrect",
            Self::HasPlaceholders => "has-placeholders",
            Self::Unspecified => "unspecified",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub span: Span,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFix {
    pub title: String,
    pub applicability: FixApplicability,
    pub edits: Vec<TextEdit>,
}

impl fmt::Display for DiagnosticSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    /// The primary source span. Kept directly accessible for parser recovery
    /// and simple clients; `labels[0]` describes the same location.
    pub span: Span,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
    pub fixes: Vec<DiagnosticFix>,
}

impl Diagnostic {
    /// Constructs a syntax error. Parser call sites use this concise default;
    /// other compiler stages use their category-specific constructors.
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self::error(DiagnosticCode::Syntax, message, span)
    }

    pub fn lexical(message: impl Into<String>, span: Span) -> Self {
        Self::error(DiagnosticCode::Lexical, message, span)
    }

    pub fn type_error(message: impl Into<String>, span: Span) -> Self {
        Self::error(DiagnosticCode::Type, message, span)
    }

    pub fn semantic(message: impl Into<String>, span: Span) -> Self {
        Self::error(DiagnosticCode::Semantic, message, span)
    }

    pub fn warning(code: DiagnosticCode, message: impl Into<String>, span: Span) -> Self {
        let mut diagnostic = Self::error(code, message, span);
        diagnostic.severity = DiagnosticSeverity::Warning;
        diagnostic
    }

    pub fn error(code: DiagnosticCode, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            span,
            labels: vec![DiagnosticLabel {
                style: DiagnosticLabelStyle::Primary,
                span,
                message: None,
            }],
            notes: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub fn with_primary_label(mut self, message: impl Into<String>) -> Self {
        self.labels[0].message = Some(message.into());
        self
    }

    pub fn with_secondary_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(DiagnosticLabel {
            style: DiagnosticLabelStyle::Secondary,
            span,
            message: Some(message.into()),
        });
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_fix(mut self, fix: DiagnosticFix) -> Self {
        self.fixes.push(fix);
        self
    }

    pub fn with_machine_applicable_fix(
        self,
        title: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Self {
        self.with_fix(DiagnosticFix {
            title: title.into(),
            applicability: FixApplicability::MachineApplicable,
            edits: vec![TextEdit {
                span,
                replacement: replacement.into(),
            }],
        })
    }

    pub fn render(&self, source_name: &str, source: &str) -> String {
        let (line, column) = line_column(source, self.span.start);
        let mut rendered = format!(
            "{source_name}:{line}:{column}: {}[{}]: {}",
            self.severity, self.code, self.message
        );
        for label in &self.labels {
            let Some(message) = &label.message else {
                continue;
            };
            match label.style {
                DiagnosticLabelStyle::Primary => {
                    rendered.push_str(&format!("\n  = primary: {message}"));
                }
                DiagnosticLabelStyle::Secondary => {
                    let (line, column) = line_column(source, label.span.start);
                    rendered.push_str(&format!(
                        "\n  = secondary {source_name}:{line}:{column}: {message}"
                    ));
                }
            }
        }
        for note in &self.notes {
            rendered.push_str(&format!("\n  = note: {note}"));
        }
        for fix in &self.fixes {
            rendered.push_str(&format!(
                "\n  = help: {} ({})",
                fix.title, fix.applicability
            ));
        }
        rendered
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset.min(source.len())];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before.len() + 1, |(_, tail)| tail.len() + 1);
    (line, column)
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}
