//! Backend-independent managed metadata binding plans.
//!
//! Source schemas remain the canonical declaration model. This module derives
//! the runtime-facing lookup plan from those stable syntax identities and the
//! semantic field types established by type checking. Mono and IL2CPP adapters
//! therefore consume one model without becoming additional symbol registries.

use crate::{
    ast::{
        ManagedBindingNameKind, ManagedClassDecl, ManagedClassId, ManagedFieldDecl, ManagedFieldId,
        ManagedImageId, ManagedItemDecl, Program, Span,
    },
    semantic::SemanticModel,
    types::TypeId,
};

/// All managed metadata declarations needed to bind one checked program.
///
/// Building this value does not make any declarations reachable and does not
/// generate readers. Backend reachability chooses which entries are actually
/// materialized for an attachment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ManagedBindingPlan {
    pub classes: Vec<ManagedClassBinding>,
    pub automatic_layout: Option<crate::layout_selection::LayoutSelectionPlan>,
}

/// One nominal managed class together with its metadata ownership path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedClassBinding {
    pub id: ManagedClassId,
    pub image: ManagedImageId,
    pub image_name: String,
    pub namespace: Vec<String>,
    pub metadata_names: Vec<ManagedMetadataCandidate>,
    pub fields: Vec<ManagedFieldBinding>,
    pub conditional_fields: Vec<ManagedConditionalBinding>,
}

impl ManagedClassBinding {
    pub fn all_fields(&self) -> impl Iterator<Item = &ManagedFieldBinding> {
        self.fields.iter().chain(
            self.conditional_fields
                .iter()
                .flat_map(|group| &group.fields),
        )
    }
}

/// Managed fields guarded by attachment-wide layout facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedConditionalBinding {
    pub predicate: crate::semantic::ResolvedLayoutPredicate,
    pub fields: Vec<ManagedFieldBinding>,
}

/// One static or instance metadata field with its source and value types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedFieldBinding {
    pub id: ManagedFieldId,
    pub kind: ManagedFieldKind,
    /// Type written in the schema. A managed class denotes the remote field's
    /// class, not an eagerly nested local snapshot.
    pub declared_type: TypeId,
    /// Value produced by a field read. Direct managed class fields become the
    /// corresponding `T.Ref`; terminal fields retain their declared type.
    pub value_type: TypeId,
    /// How the remote field storage is decoded after its metadata offset has
    /// been resolved. Keeping this policy in the binding plan prevents Mono,
    /// IL2CPP, snapshots, and live reads from growing separate string rules.
    pub read: ManagedFieldRead,
    pub metadata_names: Vec<ManagedMetadataCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedFieldRead {
    Fixed,
    ManagedString {
        max_utf16_units: u32,
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedFieldKind {
    Static,
    Instance,
}

/// A deterministic runtime metadata spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedMetadataCandidate {
    pub name: String,
    pub origin: ManagedMetadataOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedMetadataOrigin {
    /// An implicit source declaration name or an explicit `from` candidate.
    Declared(Span),
    /// The conventional backing field corresponding to a declared candidate.
    AutomaticPropertyBackingField(Span),
}

impl ManagedBindingPlan {
    pub fn build(program: &Program, semantics: &SemanticModel) -> Self {
        let mut classes = Vec::new();
        for image in &program.managed_images {
            collect_classes(
                &mut classes,
                image.id,
                &image.name,
                &[],
                &image.items,
                semantics,
            );
        }
        let automatic_layout =
            match crate::layout_selection::automatic_layout_selection(program, semantics) {
                crate::layout_selection::AutomaticLayoutSelection::Available(plan) => Some(plan),
                crate::layout_selection::AutomaticLayoutSelection::NotDeclared
                | crate::layout_selection::AutomaticLayoutSelection::RequiresExplicit(_) => None,
            };
        Self {
            classes,
            automatic_layout,
        }
    }
}

fn collect_classes(
    output: &mut Vec<ManagedClassBinding>,
    image: ManagedImageId,
    image_name: &str,
    namespace: &[String],
    items: &[ManagedItemDecl],
    semantics: &SemanticModel,
) {
    for item in items {
        match item {
            ManagedItemDecl::Namespace(item) => {
                let mut qualified = namespace.to_vec();
                qualified.push(item.name.clone());
                collect_classes(
                    output,
                    image,
                    image_name,
                    &qualified,
                    &item.items,
                    semantics,
                );
            }
            ManagedItemDecl::Class(class) => output.push(class_binding(
                image, image_name, namespace, class, semantics,
            )),
        }
    }
}

fn class_binding(
    image: ManagedImageId,
    image_name: &str,
    namespace: &[String],
    class: &ManagedClassDecl,
    semantics: &SemanticModel,
) -> ManagedClassBinding {
    ManagedClassBinding {
        id: class.id,
        image,
        image_name: image_name.to_owned(),
        namespace: namespace.to_vec(),
        metadata_names: class
            .metadata_name_candidates()
            .map(|(name, span)| ManagedMetadataCandidate {
                name: name.to_owned(),
                origin: ManagedMetadataOrigin::Declared(span),
            })
            .collect(),
        fields: class
            .fields
            .iter()
            .map(|field| field_binding(field, semantics))
            .collect(),
        conditional_fields: class
            .conditional_fields
            .iter()
            .map(|group| ManagedConditionalBinding {
                predicate: group
                    .fields
                    .first()
                    .map(|field| {
                        semantics
                            .managed_field_layout_predicate(field.id)
                            .cloned()
                            .unwrap_or_default()
                    })
                    .unwrap_or_default(),
                fields: group
                    .fields
                    .iter()
                    .map(|field| field_binding(field, semantics))
                    .collect(),
            })
            .collect(),
    }
}

fn field_binding(field: &ManagedFieldDecl, semantics: &SemanticModel) -> ManagedFieldBinding {
    let declared_type = semantics
        .managed_field_type(field.id)
        .expect("checked managed fields have declared semantic types");
    let value_type = semantics
        .managed_field_value_type(field.id)
        .expect("checked managed fields have value semantic types");
    let read = field.max_length.map_or(ManagedFieldRead::Fixed, |limit| {
        ManagedFieldRead::ManagedString {
            max_utf16_units: limit.value,
            nullable: matches!(
                semantics.types().kind(value_type),
                crate::types::TypeKind::Option { .. }
            ),
        }
    });
    ManagedFieldBinding {
        id: field.id,
        kind: if field.is_static {
            ManagedFieldKind::Static
        } else {
            ManagedFieldKind::Instance
        },
        declared_type,
        value_type,
        read,
        metadata_names: field_metadata_candidates(field),
    }
}

fn field_metadata_candidates(field: &ManagedFieldDecl) -> Vec<ManagedMetadataCandidate> {
    field
        .binding_name_candidates()
        .into_iter()
        .map(|(name, span, kind)| ManagedMetadataCandidate {
            name,
            origin: match kind {
                ManagedBindingNameKind::Declared => ManagedMetadataOrigin::Declared(span),
                ManagedBindingNameKind::AutomaticPropertyBackingField => {
                    ManagedMetadataOrigin::AutomaticPropertyBackingField(span)
                }
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, parse, types::TypeKind};

    #[test]
    fn binding_plan_preserves_ownership_types_conditions_and_metadata_order() {
        let checked = check(
            parse(
                r#"
enum Edition { Demo }

image "Assembly-CSharp" {
    namespace Game {
        namespace Core {
            class Player {}

            class GameManager from ["Manager", "GameManager"] {
                static GameManager instance from ["Instance", "_instance"];
                Player player;
                i32 points from "<Points>k__BackingField";

                if layout.edition == Edition.Demo {
                    u16 scene;
                }
            }
        }
    }
}

state "game.exe" { layout { edition: Edition } }
onAttach { return Layout { edition: Edition.Demo } }
"#,
            )
            .unwrap(),
        )
        .unwrap();

        let plan = ManagedBindingPlan::build(checked.syntax(), checked.semantics());
        let manager = plan
            .classes
            .iter()
            .find(|class| {
                class
                    .metadata_names
                    .iter()
                    .any(|candidate| candidate.name == "Manager")
            })
            .unwrap();
        assert_eq!(manager.image_name, "Assembly-CSharp");
        assert_eq!(manager.namespace, ["Game", "Core"]);
        assert_eq!(
            manager
                .metadata_names
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            ["Manager", "GameManager"]
        );

        let instance = &manager.fields[0];
        assert_eq!(instance.kind, ManagedFieldKind::Static);
        assert!(matches!(
            checked.semantics().types().kind(instance.declared_type),
            TypeKind::ManagedClass(_)
        ));
        assert!(matches!(
            checked.semantics().types().kind(instance.value_type),
            TypeKind::ManagedReference(_)
        ));
        assert_eq!(
            instance
                .metadata_names
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            ["Instance", "_instance",]
        );

        let points = &manager.fields[2];
        assert_eq!(
            points
                .metadata_names
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            ["<Points>k__BackingField"]
        );

        let conditional = &manager.conditional_fields[0];
        assert_eq!(manager.fields.iter().chain(&conditional.fields).count(), 4);
        assert_eq!(conditional.predicate.alternatives.len(), 1);
        assert_eq!(conditional.predicate.alternatives[0].len(), 1);
    }
}
