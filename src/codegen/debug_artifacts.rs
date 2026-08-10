//! Profile-aware debugger metadata assembled only after Wasm indices are final.

use wasm_encoder::{NameMap, NameSection};

use super::imports::Abi;
use crate::{
    ast::{EnumDecl, Program, TypeApplicationId},
    semantic::{FunctionInstance, SemanticModel},
    stdlib::StandardLibrary,
    types::{TypeId, TypeKind},
};

pub(super) struct DebugArtifactPlan {
    names: NameSection,
}

pub(super) fn set_function_name(set: TypeApplicationId, operation: &str) -> String {
    format!("__splitscript::set#{}::{operation}", set.index())
}

pub(super) fn user_function_name(
    name: &str,
    instance: &FunctionInstance,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    enums: &[EnumDecl],
) -> String {
    if instance.type_arguments.is_empty() {
        return name.to_owned();
    }
    let arguments = instance
        .type_arguments
        .iter()
        .map(|ty| type_name(*ty, program, semantics, standard_library, enums))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}<{arguments}>")
}

fn type_name(
    ty: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    enums: &[EnumDecl],
) -> String {
    let nested = |ty| type_name(ty, program, semantics, standard_library, enums);
    match semantics.types().kind(ty) {
        TypeKind::Error => "<unknown>".to_owned(),
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Standard(standard) => standard_library.type_decl(*standard).name.to_owned(),
        TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
        TypeKind::SettingsView => "SettingsView".to_owned(),
        TypeKind::Record(id) => program
            .records
            .iter()
            .find(|record| record.id == *id)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| format!("record#{}", id.index())),
        TypeKind::Enum(id) => enums
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{}", id.index())),
        TypeKind::GenericParameter { index, .. } => crate::types::generic_parameter_name(*index),
        TypeKind::Array {
            element, length, ..
        } => match length {
            Some(length) => format!("[{}; {length}]", nested(*element)),
            None => format!("[{}]", nested(*element)),
        },
        TypeKind::Option { value, .. } => format!("{}?", nested(*value)),
        TypeKind::Result { value, .. } => format!("{}!", nested(*value)),
        TypeKind::Async { value, .. } => format!("async {}", nested(*value)),
        TypeKind::Set { element, .. } => format!("Set<{}>", nested(*element)),
    }
}

impl DebugArtifactPlan {
    pub(super) fn new(abi: &Abi, defined_functions: &[(u32, String)]) -> Self {
        let mut entries = abi
            .debug_names()
            .map(|(index, name)| (index, format!("env::{name}")))
            .chain(defined_functions.iter().cloned())
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|(index, _)| *index);
        assert!(
            entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "final function indices must have exactly one debug name"
        );
        assert!(
            entries
                .iter()
                .enumerate()
                .all(|(expected, (actual, _))| *actual == expected as u32),
            "every imported and defined function must receive a debug name"
        );

        let mut function_names = NameMap::new();
        for (index, name) in entries {
            function_names.append(index, &name);
        }
        let mut names = NameSection::new();
        names.module("SplitScript autosplitter");
        names.functions(&function_names);
        Self { names }
    }

    pub(super) fn names(&self) -> &NameSection {
        &self.names
    }
}
