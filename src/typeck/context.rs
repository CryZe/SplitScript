//! Scoped context for body checking.
//!
//! These enums replace combinations of booleans whose stale values could
//! previously describe impossible checker states.

use crate::ast::ActionKind;
use crate::inference::Type;
use crate::stdlib::StdlibItemId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DebugContext {
    #[default]
    Normal,
    DebugOnly,
}

impl DebugContext {
    pub(super) fn from_declaration(debug_only: bool) -> Self {
        if debug_only {
            Self::DebugOnly
        } else {
            Self::Normal
        }
    }

    pub(super) fn is_debug(self) -> bool {
        self == Self::DebugOnly
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LoopContext {
    depth: usize,
}

impl LoopContext {
    pub(super) fn enter(&mut self) {
        self.depth += 1;
    }

    pub(super) fn exit(&mut self) {
        self.depth = self
            .depth
            .checked_sub(1)
            .expect("loop contexts are balanced");
    }

    pub(super) fn is_inside(self) -> bool {
        self.depth != 0
    }
}

#[derive(Debug, Clone)]
pub(super) enum CallableContext {
    TopLevel,
    Function,
    LibraryFunction(StdlibItemId),
    Action(ActionKind),
}

impl CallableContext {
    pub(super) fn is_function(&self) -> bool {
        matches!(self, Self::Function | Self::LibraryFunction(_))
    }

    pub(super) fn can_suspend(&self) -> bool {
        matches!(
            self,
            Self::Function | Self::LibraryFunction(_) | Self::Action(ActionKind::OnAttach)
        )
    }

    pub(super) fn action(&self) -> Option<ActionKind> {
        match self {
            Self::Action(action) => Some(*action),
            Self::TopLevel | Self::Function | Self::LibraryFunction(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ExpressionMode {
    #[default]
    Normal,
    DirectReturn,
    StateSource,
    SuspensionOperand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum NonePolicy {
    #[default]
    OptionalOnly,
    DomainNullable,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FailureContext {
    None,
    Boundary { result: Type, propagated: bool },
}

impl FailureContext {
    pub(super) fn boundary(result: Type) -> Self {
        Self::Boundary {
            result,
            propagated: false,
        }
    }

    pub(super) fn result(self) -> Option<Type> {
        match self {
            Self::None => None,
            Self::Boundary { result, .. } => Some(result),
        }
    }

    pub(super) fn propagate(&mut self) -> Option<Type> {
        match self {
            Self::None => None,
            Self::Boundary { result, propagated } => {
                *propagated = true;
                Some(*result)
            }
        }
    }

    pub(super) fn propagated(self) -> bool {
        matches!(
            self,
            Self::Boundary {
                propagated: true,
                ..
            }
        )
    }
}
