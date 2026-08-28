//! Attachment-wide layout selection derived from runtime schema evidence.
//!
//! The public model is always the source-defined `Layout` record. Managed
//! metadata contributes only presence observations for conditional fields;
//! this module turns those observations into a bounded, backend-independent
//! decision plan shared by semantic validation and Wasm emission.

use std::collections::HashMap;

use crate::{
    ast::{
        ActionKind, EnumId, EnumVariantId, Expr, ExprKind, ManagedFieldId, Program, RecordFieldId,
    },
    semantic::SemanticModel,
    types::TypeKind,
    visit::{self, Visitor},
};

/// Layout products above this size require an explicit selector. This is a
/// compiler-complexity bound, not a runtime language limit: explicit
/// `onAttach` code can still select any declared combination.
pub(crate) const MAX_ENUMERATED_LAYOUT_COMBINATIONS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutomaticLayoutSelection {
    NotDeclared,
    Available(LayoutSelectionPlan),
    RequiresExplicit(ExplicitSelectionReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExplicitSelectionReason {
    PayloadVariants,
    CandidateLimit { combinations: Option<usize> },
    IndistinguishableEvidence,
    EvidenceUnavailable,
}

impl ExplicitSelectionReason {
    pub(crate) fn note(&self) -> String {
        match self {
            Self::PayloadVariants => "automatic layout selection currently requires unit-only dimension enums; return `Layout { ... }` explicitly when a dimension carries payloads".to_owned(),
            Self::CandidateLimit { combinations } => combinations.map_or_else(
                || "the layout product is too large to derive a bounded metadata selector; return `Layout { ... }` explicitly".to_owned(),
                |count| format!("the layout has {count} possible combinations, above the automatic-selection limit of {MAX_ENUMERATED_LAYOUT_COMBINATIONS}; return `Layout {{ ... }}` explicitly"),
            ),
            Self::IndistinguishableEvidence => "the declared managed fields do not distinguish every layout combination; return `Layout { ... }` explicitly after checking the remaining build facts".to_owned(),
            Self::EvidenceUnavailable => "this state provider cannot probe the conditional managed fields used as layout evidence; return `Layout { ... }` explicitly".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionPlan {
    pub dimensions: Vec<LayoutSelectionDimension>,
    /// Every probed conditional field, in stable source identity order.
    pub evidence_fields: Vec<ManagedFieldId>,
    /// Every possible assignment and its exact expected presence pattern.
    pub candidates: Vec<LayoutSelectionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionDimension {
    pub field: RecordFieldId,
    pub enumeration: EnumId,
    pub variants: Vec<EnumVariantId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionCandidate {
    /// One variant per [`LayoutSelectionPlan::dimensions`] entry.
    pub variants: Vec<EnumVariantId>,
    /// Conditional fields that must be present for this exact assignment.
    pub present_fields: Vec<ManagedFieldId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionFailureReport {
    pub header: String,
    pub observed_present: String,
    pub observed_absent: String,
    pub expected_present: String,
    pub expected_absent: String,
    pub evidence: Vec<LayoutSelectionEvidenceReport>,
    pub candidates: Vec<LayoutSelectionCandidateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionEvidenceReport {
    pub field: ManagedFieldId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutSelectionCandidateReport {
    pub label: String,
    pub present_fields: Vec<ManagedFieldId>,
}

impl LayoutSelectionFailureReport {
    pub(crate) fn messages(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.header.as_str())
            .chain(std::iter::once(self.observed_present.as_str()))
            .chain(std::iter::once(self.observed_absent.as_str()))
            .chain(std::iter::once(self.expected_present.as_str()))
            .chain(std::iter::once(self.expected_absent.as_str()))
            .chain(self.evidence.iter().map(|evidence| evidence.label.as_str()))
            .chain(
                self.candidates
                    .iter()
                    .map(|candidate| candidate.label.as_str()),
            )
    }
}

impl LayoutSelectionPlan {
    /// Builds the source-facing report used when the runtime metadata presence
    /// vector does not equal any statically valid layout pattern.
    ///
    /// The selector itself remains a compact bit-vector comparison. Keeping
    /// human-readable labels here gives static-data planning and failure
    /// emission one canonical description without making diagnostics part of
    /// either managed backend.
    pub(crate) fn failure_report(&self, program: &Program) -> LayoutSelectionFailureReport {
        let evidence = self
            .evidence_fields
            .iter()
            .map(|field| {
                let label = managed_field_label(program, *field);
                LayoutSelectionEvidenceReport {
                    field: *field,
                    label,
                }
            })
            .collect();
        let candidates = self
            .candidates
            .iter()
            .map(|candidate| {
                let layout = self
                    .dimensions
                    .iter()
                    .zip(&candidate.variants)
                    .map(|(dimension, variant)| {
                        let field = program.records[program
                            .state
                            .as_ref()
                            .and_then(|state| state.layout.as_ref())
                            .expect("layout selection plans have a layout record")
                            .record
                            .index()]
                        .fields
                        .iter()
                        .find(|field| field.id == dimension.field)
                        .expect("layout dimensions use fields from the layout record");
                        let enumeration = program
                            .enum_declaration(dimension.enumeration)
                            .expect("layout dimensions use source enums");
                        let variant = enumeration
                            .variants
                            .iter()
                            .find(|declaration| declaration.id == *variant)
                            .expect("layout candidates use declared variants");
                        format!("{}: {}.{}", field.name, enumeration.name, variant.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                LayoutSelectionCandidateReport {
                    label: format!("Expected `Layout {{ {layout} }}`"),
                    present_fields: candidate.present_fields.clone(),
                }
            })
            .collect();
        LayoutSelectionFailureReport {
            header: "Could not select an attachment layout: managed metadata did not match any declared layout".to_owned(),
            observed_present: "Observed present managed fields:".to_owned(),
            observed_absent: "Observed absent managed fields:".to_owned(),
            expected_present: "  Expected present fields:".to_owned(),
            expected_absent: "  Expected absent fields:".to_owned(),
            evidence,
            candidates,
        }
    }
}

fn managed_field_label(program: &Program, target: ManagedFieldId) -> String {
    fn find(
        items: &[crate::ast::ManagedItemDecl],
        namespace: &[&str],
        target: ManagedFieldId,
    ) -> Option<String> {
        for item in items {
            match item {
                crate::ast::ManagedItemDecl::Namespace(declaration) => {
                    let mut nested = namespace.to_vec();
                    nested.push(&declaration.name);
                    if let Some(label) = find(&declaration.items, &nested, target) {
                        return Some(label);
                    }
                }
                crate::ast::ManagedItemDecl::Class(class) => {
                    if let Some(field) = class.all_fields().find(|field| field.id == target) {
                        let owner = namespace
                            .iter()
                            .copied()
                            .chain(std::iter::once(class.name.as_str()))
                            .collect::<Vec<_>>()
                            .join(".");
                        return Some(format!("{owner}.{}", field.name));
                    }
                }
            }
        }
        None
    }

    for image in &program.managed_images {
        if let Some(field) = find(&image.items, &[], target) {
            return format!("{}::{field}", image.name);
        }
    }
    unreachable!("layout evidence belongs to a managed source field")
}

struct ManagedEvidenceGroup {
    alternatives: Vec<Vec<(RecordFieldId, EnumVariantId)>>,
    fields: Vec<ManagedFieldId>,
}

pub(crate) fn automatic_layout_selection(
    program: &Program,
    semantics: &SemanticModel,
) -> AutomaticLayoutSelection {
    automatic_layout_selection_with(
        program,
        |field| {
            let ty = semantics.record_field_type(field)?;
            let TypeKind::Enum(enumeration) = semantics.types().kind(ty) else {
                return None;
            };
            Some(*enumeration)
        },
        |field| {
            semantics
                .managed_field_layout_predicate(field)
                .map(|predicate| {
                    predicate
                        .alternatives
                        .iter()
                        .map(|alternative| {
                            alternative
                                .iter()
                                .map(|constraint| (constraint.dimension, constraint.variant))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default()
        },
    )
}

pub(crate) fn automatic_layout_selection_with(
    program: &Program,
    enum_for_dimension: impl Fn(RecordFieldId) -> Option<EnumId>,
    predicates_for_field: impl Fn(ManagedFieldId) -> Vec<Vec<(RecordFieldId, EnumVariantId)>>,
) -> AutomaticLayoutSelection {
    let Some(layout) = program
        .state
        .as_ref()
        .and_then(|state| state.layout.as_ref())
    else {
        return AutomaticLayoutSelection::NotDeclared;
    };
    let record = &program.records[layout.record.index()];
    let mut dimensions = Vec::with_capacity(record.fields.len());
    let mut combination_count = 1usize;
    for field in &record.fields {
        let Some(enumeration) = enum_for_dimension(field.id) else {
            return AutomaticLayoutSelection::RequiresExplicit(
                ExplicitSelectionReason::IndistinguishableEvidence,
            );
        };
        let declaration = program
            .enum_declaration(enumeration)
            .expect("checked layout dimensions use source enums");
        if declaration
            .variants
            .iter()
            .any(|variant| variant.payload.is_some())
        {
            return AutomaticLayoutSelection::RequiresExplicit(
                ExplicitSelectionReason::PayloadVariants,
            );
        }
        combination_count = match combination_count.checked_mul(declaration.variants.len()) {
            Some(count) if count <= MAX_ENUMERATED_LAYOUT_COMBINATIONS => count,
            count => {
                return AutomaticLayoutSelection::RequiresExplicit(
                    ExplicitSelectionReason::CandidateLimit {
                        combinations: count,
                    },
                );
            }
        };
        dimensions.push(LayoutSelectionDimension {
            field: field.id,
            enumeration,
            variants: declaration
                .variants
                .iter()
                .map(|variant| variant.id)
                .collect(),
        });
    }

    let groups = program
        .managed_class_declarations()
        .into_iter()
        .flat_map(|class| &class.conditional_fields)
        .filter_map(|group| {
            let field = group.fields.first()?;
            Some(ManagedEvidenceGroup {
                alternatives: predicates_for_field(field.id),
                fields: group
                    .fields
                    .iter()
                    .map(|field| field.id)
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let mut evidence_fields = groups
        .iter()
        .flat_map(|group| group.fields.iter().copied())
        .collect::<Vec<_>>();
    evidence_fields.sort_by_key(|field| field.index());
    evidence_fields.dedup();

    let mut candidates = Vec::with_capacity(combination_count);
    enumerate_candidates(
        &dimensions,
        &groups,
        0,
        &mut Vec::with_capacity(dimensions.len()),
        &mut candidates,
    );

    let mut evidence_patterns = HashMap::<Vec<ManagedFieldId>, usize>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if evidence_patterns
            .insert(candidate.present_fields.clone(), index)
            .is_some()
        {
            return AutomaticLayoutSelection::RequiresExplicit(
                ExplicitSelectionReason::IndistinguishableEvidence,
            );
        }
    }

    AutomaticLayoutSelection::Available(LayoutSelectionPlan {
        dimensions,
        evidence_fields,
        candidates,
    })
}

fn enumerate_candidates(
    dimensions: &[LayoutSelectionDimension],
    groups: &[ManagedEvidenceGroup],
    dimension_index: usize,
    variants: &mut Vec<EnumVariantId>,
    output: &mut Vec<LayoutSelectionCandidate>,
) {
    if dimension_index != dimensions.len() {
        for variant in &dimensions[dimension_index].variants {
            variants.push(*variant);
            enumerate_candidates(dimensions, groups, dimension_index + 1, variants, output);
            variants.pop();
        }
        return;
    }

    let assignment = dimensions
        .iter()
        .zip(variants.iter().copied())
        .map(|(dimension, variant)| (dimension.field, variant))
        .collect::<HashMap<_, _>>();
    let mut present_fields = groups
        .iter()
        .filter(|group| {
            group.alternatives.iter().any(|alternative| {
                alternative
                    .iter()
                    .all(|(dimension, variant)| assignment.get(dimension) == Some(variant))
            })
        })
        .flat_map(|group| group.fields.iter().copied())
        .collect::<Vec<_>>();
    present_fields.sort_by_key(|field| field.index());
    present_fields.dedup();
    output.push(LayoutSelectionCandidate {
        variants: variants.clone(),
        present_fields,
    });
}

/// Whether user `onAttach` code explicitly owns layout selection. Returns
/// inside closures belong to those closures and do not count.
pub(crate) fn has_explicit_layout_return(program: &Program) -> bool {
    struct Finder(bool);

    impl<'ast> Visitor<'ast> for Finder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            match &expression.kind {
                ExprKind::Return(Some(_)) => self.0 = true,
                ExprKind::Closure { .. } => {}
                _ => visit::walk_expr(self, expression),
            }
        }
    }

    let Some(action) = program
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OnAttach)
    else {
        return false;
    };
    let mut finder = Finder(false);
    finder.visit_block(&action.body);
    finder.0
}
