use codespan_reporting::{
    diagnostic::{Diagnostic as CodespanDiagnostic, Label, Severity},
    files::SimpleFiles,
    term::{self, Config},
};

use splitscript::{
    Diagnostic, DiagnosticLabelStyle, DiagnosticSeverity, FixApplicability, compiler::ast::Span,
};

/// Emits compiler diagnostics with source snippets to a terminal writer.
///
/// The compiler and editor integrations retain SplitScript's richer structured
/// diagnostic model. Only the native CLI boundary converts that model into
/// `codespan-reporting` values.
pub(crate) fn emit(
    writer: &mut dyn term::termcolor::WriteColor,
    source_name: &str,
    source: &str,
    diagnostics: &[Diagnostic],
) -> Result<(), codespan_reporting::files::Error> {
    let mut files = SimpleFiles::new();
    let file_id = files.add(source_name, source);
    let config = Config::default();

    for diagnostic in diagnostics {
        let rendered = convert(file_id, source, diagnostic);
        term::emit_to_write_style(writer, &config, &files, &rendered)?;
    }

    Ok(())
}

fn convert(file_id: usize, source: &str, diagnostic: &Diagnostic) -> CodespanDiagnostic<usize> {
    let labels = diagnostic
        .labels
        .iter()
        .map(|label| {
            let range = source_range(source, label.span);
            let rendered = match label.style {
                DiagnosticLabelStyle::Primary => Label::primary(file_id, range),
                DiagnosticLabelStyle::Secondary => Label::secondary(file_id, range),
            };
            match &label.message {
                Some(message) => rendered.with_message(message),
                None => rendered,
            }
        })
        .collect();

    let mut notes = diagnostic.notes.clone();
    if let Some(uri) = diagnostic.documentation_uri() {
        notes.push(format!("help: SplitScript documentation `{uri}`"));
    }
    if let Some(topic) = diagnostic.migration_topic() {
        notes.push(format!("help: SplitScript documentation topic `{topic}`"));
    }
    notes.extend(diagnostic.fixes.iter().map(|fix| match fix.applicability {
        FixApplicability::MachineApplicable => format!("help: {}", fix.title),
        applicability => format!("help ({}): {}", applicability, fix.title),
    }));

    CodespanDiagnostic::new(match diagnostic.severity {
        DiagnosticSeverity::Error => Severity::Error,
        DiagnosticSeverity::Warning => Severity::Warning,
        DiagnosticSeverity::Information => Severity::Note,
        DiagnosticSeverity::Hint => Severity::Help,
    })
    .with_code(diagnostic.code.as_str())
    .with_message(&diagnostic.message)
    .with_labels(labels)
    .with_notes(notes)
}

fn source_range(source: &str, span: Span) -> std::ops::Range<usize> {
    let start = char_boundary_at_or_before(source, span.start);
    let end = char_boundary_at_or_before(source, span.end).max(start);
    start..end
}

fn char_boundary_at_or_before(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use codespan_reporting::term::termcolor::Buffer;
    use splitscript::{DiagnosticCode, DiagnosticFix, TextEdit};

    #[test]
    fn renders_multiple_locations_notes_and_fixes_as_source_annotations() {
        let source = "struct First {}\nrecord First {}\n";
        let diagnostic =
            Diagnostic::type_error("duplicate declaration `First`", Span { start: 23, end: 28 })
                .with_primary_label("`First` is declared again here")
                .with_secondary_label(Span { start: 7, end: 12 }, "the first declaration is here")
                .with_note("declaration names must be unique")
                .with_fix(DiagnosticFix {
                    title: "rename the second declaration".to_owned(),
                    applicability: FixApplicability::MachineApplicable,
                    edits: vec![TextEdit {
                        span: Span { start: 23, end: 28 },
                        replacement: "Second".to_owned(),
                    }],
                });
        let output = render("game.split", source, &[diagnostic]);

        assert!(output.contains("error[SS0003]: duplicate declaration `First`"));
        assert!(output.contains("game.split:2:8"));
        assert!(output.contains("`First` is declared again here"));
        assert!(output.contains("the first declaration is here"));
        assert!(output.contains("declaration names must be unique"));
        assert!(output.contains("help: rename the second declaration"));
        assert!(output.contains("struct First {}"));
        assert!(!output.contains("\u{1b}["));
    }

    #[test]
    fn renders_warnings_and_accepts_unicode_and_empty_eof_spans() {
        let source = "print(`🦊`)\n";
        let diagnostic = Diagnostic::warning(
            DiagnosticCode::UnusedBinding,
            "unused value",
            Span {
                start: source.len(),
                end: source.len(),
            },
        )
        .with_primary_label("the value is unused");
        let output = render("unicode.split", source, &[diagnostic]);

        assert!(output.contains("warning[SS1002]: unused value"));
        assert!(output.contains("unicode.split:2:1"));
        assert!(output.contains("the value is unused"));
    }

    #[test]
    fn renders_secondary_locations_from_real_compiler_diagnostics() {
        let source = "state \"first.exe\" {}\nstate \"second.exe\" {}\n";
        let diagnostics = splitscript::parse(source).expect_err("the state is declared twice");
        let duplicate = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.labels.len() > 1)
            .expect("the duplicate-state diagnostic points to both declarations");
        let output = render("duplicate.split", source, std::slice::from_ref(duplicate));

        assert!(output.contains("state \"first.exe\" {}"));
        assert!(output.contains("state \"second.exe\" {}"));
        assert!(output.contains("the first state declaration is here"));
    }

    #[test]
    fn renders_stable_documentation_topics_for_native_workflows() {
        let diagnostic = Diagnostic::new("legacy lifecycle", Span { start: 0, end: 6 })
            .with_migration_topic("asl.lifecycle.update");
        let output = render("legacy.split", "update", &[diagnostic]);

        assert!(output.contains("SplitScript documentation topic `asl.lifecycle.update`"));
    }

    #[test]
    fn renders_direct_compiler_documentation_links() {
        let diagnostic = Diagnostic::new("unknown provider", Span { start: 0, end: 5 })
            .with_documentation_uri("/stdlib/state-providers/index.md");
        let output = render("game.split", "Untiy", &[diagnostic]);

        assert!(output.contains("SplitScript documentation `/stdlib/state-providers/index.md`"));
    }

    fn render(source_name: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
        let mut buffer = Buffer::no_color();
        emit(&mut buffer, source_name, source, diagnostics).unwrap();
        String::from_utf8(buffer.into_inner()).unwrap()
    }
}
