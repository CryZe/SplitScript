use wasm_encoder::{CodeSection, Function};

use super::debug_artifacts::DebugRecorder;

/// Builds the code section while preserving the final function-index and
/// code-section-offset invariants needed by DWARF addresses.
pub(super) struct CodeBodies<'a> {
    section: CodeSection,
    imported_functions: u32,
    defined_functions: u32,
    debug: Option<&'a DebugRecorder>,
}

impl<'a> CodeBodies<'a> {
    pub(super) fn new(
        imported_functions: u32,
        defined_functions: usize,
        debug: Option<&'a DebugRecorder>,
    ) -> Self {
        Self {
            section: CodeSection::new(),
            imported_functions,
            defined_functions: u32::try_from(defined_functions)
                .expect("WebAssembly modules support at most u32::MAX functions"),
            debug,
        }
    }

    pub(super) fn push(&mut self, body: &Function) {
        if let Some(debug) = self.debug {
            let body_length =
                u32::try_from(body.byte_len()).expect("debuggable function bodies fit in 4 GiB");
            let raw_body_start = encoded_u32_len(self.defined_functions)
                + u32::try_from(self.section.byte_len())
                    .expect("debuggable code sections fit in 4 GiB")
                + encoded_u32_len(body_length);
            debug.register_body(
                self.imported_functions + self.section.len(),
                raw_body_start,
                body_length,
            );
        }
        self.section.function(body);
    }

    pub(super) fn finish(self) -> CodeSection {
        assert_eq!(
            self.section.len(),
            self.defined_functions,
            "every planned function must have one code body"
        );
        self.section
    }
}

const fn encoded_u32_len(mut value: u32) -> u32 {
    let mut length = 1;
    while value >= 0x80 {
        value >>= 7;
        length += 1;
    }
    length
}
