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
    ValueBlockSemicolon,
    AmbiguousRetryFallback,
    StaticSettingLookup,
    SuspiciousInterpolation,
}

impl DiagnosticCode {
    pub const WARNINGS: [Self; 8] = [
        Self::MustUse,
        Self::UnusedBinding,
        Self::UnusedDeclaration,
        Self::UnusedMember,
        Self::ValueBlockSemicolon,
        Self::AmbiguousRetryFallback,
        Self::StaticSettingLookup,
        Self::SuspiciousInterpolation,
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
            Self::ValueBlockSemicolon => "SS1005",
            Self::AmbiguousRetryFallback => "SS1006",
            Self::StaticSettingLookup => "SS1007",
            Self::SuspiciousInterpolation => "SS1008",
        }
    }

    pub const fn is_warning(self) -> bool {
        matches!(
            self,
            Self::MustUse
                | Self::UnusedBinding
                | Self::UnusedDeclaration
                | Self::UnusedMember
                | Self::ValueBlockSemicolon
                | Self::AmbiguousRetryFallback
                | Self::StaticSettingLookup
                | Self::SuspiciousInterpolation
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
            "SS1005" => Ok(Self::ValueBlockSemicolon),
            "SS1006" => Ok(Self::AmbiguousRetryFallback),
            "SS1007" => Ok(Self::StaticSettingLookup),
            "SS1008" => Ok(Self::SuspiciousInterpolation),
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

/// Lazily allocated collection of machine-readable diagnostic fixes.
///
/// Diagnostics are the parser's error value, so keeping the empty case to one
/// pointer avoids inflating every small `Result` and does not allocate for the
/// common no-fix case. Slice behavior remains available through dereferencing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticFixes(Option<Box<DiagnosticFixStorage>>);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DiagnosticFixStorage {
    values: Vec<DiagnosticFix>,
}

impl DiagnosticFixes {
    pub fn push(&mut self, fix: DiagnosticFix) {
        self.0.get_or_insert_with(Box::default).values.push(fix);
    }

    pub fn as_slice(&self) -> &[DiagnosticFix] {
        self
    }
}

impl std::ops::Deref for DiagnosticFixes {
    type Target = [DiagnosticFix];

    fn deref(&self) -> &Self::Target {
        self.0
            .as_deref()
            .map_or(&[], |storage| storage.values.as_slice())
    }
}

impl IntoIterator for DiagnosticFixes {
    type Item = DiagnosticFix;
    type IntoIter = std::vec::IntoIter<DiagnosticFix>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .map_or_else(Vec::new, |storage| storage.values)
            .into_iter()
    }
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
    pub fixes: DiagnosticFixes,
    /// Stable compiler-owned migration concept identity. Frontends decide how
    /// to present or open it.
    pub migration_topic: Option<Box<str>>,
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
            fixes: DiagnosticFixes::default(),
            migration_topic: None,
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

    pub fn with_migration_topic(mut self, topic: impl Into<String>) -> Self {
        self.migration_topic = Some(topic.into().into_boxed_str());
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
        for fix in self.fixes.iter() {
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
