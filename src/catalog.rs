//! Shared metadata primitives for compiler-owned public catalogs.
//!
//! Standard-library callables and language syntax have different identities
//! and lookup rules, but documentation and examples should be consumed by the
//! same generated-docs and editor infrastructure.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Documentation<Id: 'static> {
    pub summary: &'static str,
    pub details: &'static str,
    pub examples: &'static [Example],
    pub related: &'static [Id],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    pub title: &'static str,
    /// The concise, user-facing snippet rendered in documentation and editor
    /// hovers.
    pub source: &'static str,
    /// A complete program used only to keep the example compiler-checked.
    /// This may provide declarations and lifecycle context that would distract
    /// from the documented symbol in `source`.
    validation: ExampleValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExampleValidation {
    CompleteProgram(&'static str),
    OnAttachBody,
    ProviderOnAttachBody(&'static str),
}

impl Example {
    pub const fn checked(
        title: &'static str,
        source: &'static str,
        validation_source: &'static str,
    ) -> Self {
        Self {
            title,
            source,
            validation: ExampleValidation::CompleteProgram(validation_source),
        }
    }

    /// Creates an example that is already a complete SplitScript program.
    ///
    /// This is reserved for examples of top-level declarations and lifecycle
    /// blocks. Statement and expression examples should use one of the body
    /// helpers so the rendered snippet stays focused.
    pub const fn complete_program(title: &'static str, source: &'static str) -> Self {
        Self {
            title,
            source,
            validation: ExampleValidation::CompleteProgram(source),
        }
    }

    /// Creates a focused statement snippet whose compiler fixture is generated
    /// automatically. Catalog authors do not need to pollute the visible
    /// example with an otherwise unrelated state declaration and action block.
    pub const fn on_attach_body(title: &'static str, source: &'static str) -> Self {
        Self {
            title,
            source,
            validation: ExampleValidation::OnAttachBody,
        }
    }

    pub const fn provider_on_attach_body(
        title: &'static str,
        source: &'static str,
        provider: &'static str,
    ) -> Self {
        Self {
            title,
            source,
            validation: ExampleValidation::ProviderOnAttachBody(provider),
        }
    }

    pub fn validation_program(self) -> String {
        match self.validation {
            ExampleValidation::CompleteProgram(source) => source.to_owned(),
            ExampleValidation::OnAttachBody | ExampleValidation::ProviderOnAttachBody(_) => {
                let mut program = match self.validation {
                    ExampleValidation::ProviderOnAttachBody(provider) => {
                        format!("state {provider} {{}}\nonAttach {{\n")
                    }
                    ExampleValidation::OnAttachBody => {
                        String::from("state \"example.exe\" {}\nonAttach {\n")
                    }
                    ExampleValidation::CompleteProgram(_) => unreachable!(),
                };
                // Keep the displayed snippet byte-for-byte contiguous inside
                // the generated program. SplitScript blocks do not require
                // indentation, and preserving the exact bytes lets semantic
                // spans map back to every line of a multiline example.
                program.push_str(self.source);
                program.push('\n');
                program.push_str("}\n");
                program
            }
        }
    }

    pub const fn has_validation_source(self) -> bool {
        match self.validation {
            ExampleValidation::CompleteProgram(source) => !source.is_empty(),
            ExampleValidation::OnAttachBody | ExampleValidation::ProviderOnAttachBody(_) => true,
        }
    }

    /// Reports whether validation checks the exact snippet shown to readers.
    ///
    /// Hidden rustdoc-style context may surround the snippet, but it must not
    /// replace it with a different program.
    pub fn validation_includes_source(self) -> bool {
        self.validation_program().contains(self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::Example;

    #[test]
    fn validation_programs_must_include_the_visible_snippet() {
        assert!(Example::complete_program("Complete", "onDetach {}").validation_includes_source());
        assert!(Example::on_attach_body("Body", "let value = 4").validation_includes_source());
        assert!(
            Example::provider_on_attach_body("Provider", "let value = 4", "GBA")
                .validation_includes_source()
        );
        assert!(
            !Example::checked("Unrelated", "let value = 4", "state \"game.exe\" {}")
                .validation_includes_source()
        );
    }
}
