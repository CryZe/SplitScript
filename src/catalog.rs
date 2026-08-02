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
                for line in self.source.lines() {
                    program.push_str("    ");
                    program.push_str(line);
                    program.push('\n');
                }
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
}
