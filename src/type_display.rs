//! Canonical source spelling for resolved semantic types used by editor tooling.

use crate::{
    database::SemanticSnapshot,
    types::{TypeId, TypeKind},
};

pub(crate) fn display_type(ty: TypeId, snapshot: &SemanticSnapshot) -> String {
    let types = snapshot.semantics().types();
    match types.kind(ty) {
        TypeKind::Error => "<unknown>".to_owned(),
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Standard(standard) => snapshot
            .context()
            .standard_library()
            .type_decl(*standard)
            .name
            .to_owned(),
        TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
        TypeKind::SettingsView => "SettingsView".to_owned(),
        TypeKind::Record(id) => snapshot
            .syntax()
            .records
            .iter()
            .find(|record| record.id == *id)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| format!("record#{}", id.index())),
        TypeKind::Enum(id) => snapshot
            .enum_types()
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{}", id.index())),
        TypeKind::GenericParameter { index, .. } => crate::types::generic_parameter_name(*index),
        TypeKind::Array {
            element, length, ..
        } => match length {
            Some(length) => format!("[{}; {length}]", display_type(*element, snapshot)),
            None => format!("[{}]", display_type(*element, snapshot)),
        },
        TypeKind::Option { value, .. } => format!("{}?", display_type(*value, snapshot)),
        TypeKind::Result { value, .. } => format!("{}!", display_type(*value, snapshot)),
        TypeKind::Async { value, .. } => format!("async {}", display_type(*value, snapshot)),
        TypeKind::Set { element, .. } => format!("Set<{}>", display_type(*element, snapshot)),
        TypeKind::Application {
            constructor,
            arguments,
            ..
        } => {
            let name = snapshot
                .context()
                .standard_library()
                .type_constructor(*constructor)
                .name;
            let arguments = arguments
                .iter()
                .map(|argument| display_type(*argument, snapshot))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}<{arguments}>")
        }
        TypeKind::Range { bound, kind, .. } => {
            let bound = display_type(*bound, snapshot);
            format!("{bound}{}{bound}", kind.operator())
        }
    }
}
