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
    validation_source: &'static str,
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
            validation_source,
        }
    }

    pub const fn validation_source(self) -> &'static str {
        self.validation_source
    }
}
