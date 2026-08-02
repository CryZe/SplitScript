//! Planned GC-frame storage for values that survive async suspension.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, ValueId},
    semantic::SemanticModel,
    wasm_ir::{self, BodyOwner, LocalPurpose, TemporaryId},
};

use super::{Type, semantic_type};

#[derive(Default)]
pub(super) struct AsyncFrameLayout {
    pub fields: HashMap<ValueId, (u32, Type)>,
    pub temporaries: HashMap<TemporaryId, (u32, Type)>,
    pub types: Vec<Type>,
}

impl AsyncFrameLayout {
    pub(super) fn for_action(
        action: Option<ActionKind>,
        wasm_ir: &wasm_ir::Program,
        semantics: &SemanticModel,
    ) -> Option<Self> {
        let action = action?;
        let body = wasm_ir
            .body(BodyOwner::Action(action))
            .expect("checked actions have Wasm IR bodies");
        let mut layout = Self::default();
        for local in &body.locals {
            let destination = match local.purpose {
                LocalPurpose::Value(value) if body.frame_values.contains(&value) => Some(Ok(value)),
                LocalPurpose::Temporary(temporary)
                    if body.frame_temporaries.contains(&temporary) =>
                {
                    Some(Err(temporary))
                }
                _ => None,
            };
            if let Some(destination) = destination {
                let ty = semantic_type(local.ty, semantics);
                let field = 1 + layout.types.len() as u32;
                match destination {
                    Ok(value) => {
                        layout.fields.insert(value, (field, ty));
                    }
                    Err(temporary) => {
                        layout.temporaries.insert(temporary, (field, ty));
                    }
                }
                layout.types.push(ty);
            }
        }
        Some(layout)
    }

    pub(super) fn field(&self, destination: wasm_ir::SuspensionDestination) -> Option<(u32, Type)> {
        match destination {
            wasm_ir::SuspensionDestination::SourceValue(value) => self.fields.get(&value).copied(),
            wasm_ir::SuspensionDestination::Temporary(temporary) => {
                self.temporaries.get(&temporary).copied()
            }
            wasm_ir::SuspensionDestination::Discard
            | wasm_ir::SuspensionDestination::BodyResult => None,
        }
    }
}
