//! Planned GC-frame storage for values that survive async suspension.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, ValueId},
    semantic::SemanticModel,
    wasm_ir::{self, BodyOwner, LocalPurpose},
};

use super::{Type, semantic_type};

#[derive(Default)]
pub(super) struct AsyncFrameLayout {
    pub fields: HashMap<ValueId, (u32, Type)>,
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
            if let LocalPurpose::Value(value) = local.purpose
                && body.frame_values.contains(&value)
            {
                let ty = semantic_type(local.ty, semantics);
                let field = 1 + layout.types.len() as u32;
                layout.fields.insert(value, (field, ty));
                layout.types.push(ty);
            }
        }
        Some(layout)
    }

    pub(super) fn field(&self, binding: Option<ValueId>) -> Option<(u32, Type)> {
        binding.and_then(|binding| self.fields.get(&binding).copied())
    }
}
