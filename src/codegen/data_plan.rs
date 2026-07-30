use std::collections::HashMap;

use wasm_encoder::{ConstExpr, DataSection};

use crate::{
    ast::{Program, SettingFileFilter, SettingKind, StateSource},
    signature::parse_signature,
    wasm_ir,
};

use super::dependencies::BackendDependencies;
use super::reachability::Reachability;

pub(super) const IL2CPP_ASSEMBLIES_SIGNATURE: &str = "75 ?? 48 8B 1D ?? ?? ?? ?? 48 3B 1D";
pub(super) const IL2CPP_METADATA_SIGNATURE: &str =
    "67 6C 6F 62 61 6C 2D 6D 65 74 61 64 61 74 61 2E 64 61 74 00";
pub(super) const IL2CPP_LEA_SIGNATURE: &str = "48 8D 0D";
pub(super) const IL2CPP_SHR_SIGNATURE: &str = "48 C1 E9";
pub(super) const IL2CPP_RAX_SIGNATURE: &str = "48 89 05";

pub(super) struct StaticData {
    pub strings: StringPool,
    pub signatures: SignaturePool,
}

#[derive(Default)]
pub(super) struct StringPool {
    bytes: Vec<u8>,
    entries: HashMap<String, (u32, u32)>,
}

#[derive(Clone, Copy)]
pub(super) struct SignatureEntry {
    pub needle: u32,
    pub mask: u32,
    pub len: u32,
}

#[derive(Default)]
pub(super) struct SignaturePool {
    base: u32,
    bytes: Vec<u8>,
    entries: HashMap<String, SignatureEntry>,
}

impl StaticData {
    pub fn collect(
        program: &Program,
        wasm_ir: &wasm_ir::Program,
        reachability: &Reachability,
        dependencies: &BackendDependencies,
    ) -> Self {
        let state = program.state.as_ref().expect("checked programs have state");
        let mut strings = StringPool::default();
        for process in &state.processes {
            strings.intern(process);
        }
        for field in &state.fields {
            if let StateSource::Pointer(path) = &field.source
                && let Some(module) = &path.module
            {
                strings.intern(module);
            }
        }
        for setting in &program.settings {
            strings.intern(&setting.name);
            strings.intern(&setting.description);
            if let Some(tooltip) = &setting.tooltip {
                strings.intern(tooltip);
            }
            match &setting.kind {
                SettingKind::Choice { options, .. } => {
                    for option in options {
                        strings.intern(&option.variant);
                        strings.intern(&option.description);
                    }
                }
                SettingKind::File { filters } => {
                    for filter in filters {
                        match filter {
                            SettingFileFilter::Name {
                                description,
                                pattern,
                            } => {
                                if let Some(description) = description {
                                    strings.intern(description);
                                }
                                strings.intern(pattern);
                            }
                            SettingFileFilter::Mime(mime) => {
                                strings.intern(mime);
                            }
                        }
                    }
                }
                SettingKind::Bool { .. } | SettingKind::Title { .. } => {}
            }
        }
        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::String(value) => {
                    strings.intern(value);
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    for part in parts {
                        if let wasm_ir::InterpolatedPart::Text(value) = part {
                            strings.intern(value);
                        }
                    }
                }
                _ => {}
            }
        }
        if dependencies.uses_unity_metadata() {
            strings.intern("GameAssembly.dll");
        }

        let mut signatures = SignaturePool::new(64 + strings.bytes.len() as u32);
        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            if let wasm_ir::ExpressionKind::Signature(signature) = &expression.kind {
                signatures.intern(signature);
            }
        }
        if dependencies.uses_unity_metadata() {
            for signature in [
                IL2CPP_ASSEMBLIES_SIGNATURE,
                IL2CPP_METADATA_SIGNATURE,
                IL2CPP_LEA_SIGNATURE,
                IL2CPP_SHR_SIGNATURE,
                IL2CPP_RAX_SIGNATURE,
            ] {
                signatures.intern(signature);
            }
        }

        Self {
            strings,
            signatures,
        }
    }

    pub fn encode(&self) -> DataSection {
        let mut section = DataSection::new();
        if !self.strings.bytes.is_empty() {
            section.active(
                0,
                &ConstExpr::i32_const(64),
                self.strings.bytes.iter().copied(),
            );
        }
        if !self.signatures.bytes.is_empty() {
            section.active(
                0,
                &ConstExpr::i32_const(self.signatures.base as i32),
                self.signatures.bytes.iter().copied(),
            );
        }
        section
    }
}

impl SignaturePool {
    fn new(base: u32) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    fn intern(&mut self, signature: &str) {
        if self.entries.contains_key(signature) {
            return;
        }
        let (needle, mask) = parse_signature(signature).expect("signatures were type checked");
        let needle_ptr = self.base + self.bytes.len() as u32;
        self.bytes.extend_from_slice(&needle);
        let mask_ptr = self.base + self.bytes.len() as u32;
        self.bytes.extend_from_slice(&mask);
        self.entries.insert(
            signature.to_owned(),
            SignatureEntry {
                needle: needle_ptr,
                mask: mask_ptr,
                len: needle.len() as u32,
            },
        );
    }

    pub fn get(&self, signature: &str) -> SignatureEntry {
        self.entries[signature]
    }
}

impl StringPool {
    fn intern(&mut self, value: &str) -> (u32, u32) {
        if let Some(entry) = self.entries.get(value) {
            return *entry;
        }
        let offset = 64 + self.bytes.len() as u32;
        let len = value.len() as u32;
        self.bytes.extend_from_slice(value.as_bytes());
        self.entries.insert(value.to_owned(), (offset, len));
        (offset, len)
    }

    pub fn get(&self, value: &str) -> (u32, u32) {
        self.entries[value]
    }
}
