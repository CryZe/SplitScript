use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A stable boundary at which a compilation may stop without publishing a
/// partial product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationPhase {
    Analysis,
    WasmLowering,
    WasmEncoding,
    Publication,
}

/// Shared cancellation state for one compiler request.
///
/// Hosts may clone this value and cancel it from another thread. Compiler
/// passes only observe it at explicit phase boundaries, so cancellation never
/// leaks partially initialized products into the revisioned query database.
#[derive(Debug, Clone, Default)]
pub struct CompilationCancellation {
    cancelled: Arc<AtomicBool>,
}

impl CompilationCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn checkpoint(&self, phase: CompilationPhase) -> Result<(), CompilationCancelled> {
        if self.is_cancelled() {
            Err(CompilationCancelled { phase })
        } else {
            Ok(())
        }
    }
}

/// Typed non-diagnostic outcome for a superseded compilation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompilationCancelled {
    pub phase: CompilationPhase,
}

/// A rejected build is either an ordinary source diagnostic set or an
/// explicitly cancelled request. Cancellation is not a source error.
#[derive(Debug)]
pub enum CompilationFailure {
    Diagnostics(Vec<crate::Diagnostic>),
    Cancelled(CompilationCancelled),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_tokens_share_release_acquire_cancellation_state() {
        let token = CompilationCancellation::new();
        let host = token.clone();
        assert!(!token.is_cancelled());
        host.cancel();
        assert!(token.is_cancelled());
        assert_eq!(
            token.checkpoint(CompilationPhase::Analysis),
            Err(CompilationCancelled {
                phase: CompilationPhase::Analysis,
            })
        );
    }
}
