//! Attachment-wide shape selection derived from runtime schema evidence.
//!
//! Managed metadata contributes presence observations for conditional fields;
//! this module turns those observations into a bounded, backend-independent
//! decision plan shared by semantic validation and Wasm emission.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{ActionKind, EnumId, EnumVariantId, Expr, ExprKind, ManagedFieldId, Program},
    semantic::{ResolvedShapeDimension, SemanticModel},
    types::TypeKind,
    visit::{self, Visitor},
};

/// Shape products above this size require an explicit selector. This is a
/// compiler-complexity bound, not a runtime language limit: explicit
/// `onAttach` code can still select any declared combination.
pub(crate) const MAX_ENUMERATED_SHAPE_COMBINATIONS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutomaticShapeSelection {
    NotDeclared,
    Available(ShapeSelectionPlan),
    RequiresExplicit(ShapeSelectionReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShapeSelectionReason {
    PayloadVariants,
    CandidateLimit { combinations: Option<usize> },
    IndistinguishableEvidence,
    EvidenceUnavailable,
}

impl ShapeSelectionReason {
    pub(crate) fn note(&self) -> String {
        match self {
            Self::PayloadVariants => "automatic attachment-shape selection currently requires unit-only enum globals; assign those globals explicitly in `onAttach` when a variant carries a payload".to_owned(),
            Self::CandidateLimit { combinations } => combinations.map_or_else(
                || "the attachment-shape product is too large to derive a bounded metadata selector; assign the shape globals explicitly in `onAttach`".to_owned(),
                |count| format!("the attachment shape has {count} possible combinations, above the automatic-selection limit of {MAX_ENUMERATED_SHAPE_COMBINATIONS}; assign the shape globals explicitly in `onAttach`"),
            ),
            Self::IndistinguishableEvidence => "the declared managed fields do not distinguish every shape combination; assign the shape globals explicitly in `onAttach` after checking the remaining build facts".to_owned(),
            Self::EvidenceUnavailable => "this state provider cannot probe the conditional managed fields used as shape evidence; assign the shape globals explicitly in `onAttach`".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionPlan {
    pub dimensions: Vec<ShapeSelectionDimension>,
    /// Every probed conditional field, in stable source identity order.
    pub evidence_fields: Vec<ManagedFieldId>,
    /// Every possible assignment and its exact expected presence pattern.
    pub candidates: Vec<ShapeSelectionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionDimension {
    pub dimension: ResolvedShapeDimension,
    pub enumeration: EnumId,
    pub variants: Vec<EnumVariantId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionCandidate {
    /// One variant per [`ShapeSelectionPlan::dimensions`] entry.
    pub variants: Vec<EnumVariantId>,
    /// Conditional fields that must be present for this exact assignment.
    pub present_fields: Vec<ManagedFieldId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionFailureReport {
    pub header: String,
    pub observed_present: String,
    pub observed_absent: String,
    pub expected_present: String,
    pub expected_absent: String,
    pub evidence: Vec<ShapeSelectionEvidenceReport>,
    pub candidates: Vec<ShapeSelectionCandidateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionEvidenceReport {
    pub field: ManagedFieldId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShapeSelectionCandidateReport {
    pub label: String,
    pub present_fields: Vec<ManagedFieldId>,
}

impl ShapeSelectionFailureReport {
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

impl ShapeSelectionPlan {
    /// Builds the source-facing report used when the runtime metadata presence
    /// vector does not equal any statically valid shape pattern.
    ///
    /// The selector itself remains a compact bit-vector comparison. Keeping
    /// human-readable labels here gives static-data planning and failure
    /// emission one canonical description without making diagnostics part of
    /// either managed backend.
    pub(crate) fn failure_report(&self, program: &Program) -> ShapeSelectionFailureReport {
        let evidence = self
            .evidence_fields
            .iter()
            .map(|field| {
                let label = managed_field_label(program, *field);
                ShapeSelectionEvidenceReport {
                    field: *field,
                    label,
                }
            })
            .collect();
        let candidates = self
            .candidates
            .iter()
            .map(|candidate| {
                let shape = self
                    .dimensions
                    .iter()
                    .zip(&candidate.variants)
                    .map(|(dimension, variant)| {
                        let enumeration = program
                            .enum_declaration(dimension.enumeration)
                            .expect("shape dimensions use source enums");
                        let variant = enumeration
                            .variants
                            .iter()
                            .find(|declaration| declaration.id == *variant)
                            .expect("shape candidates use declared variants");
                        format!(
                            "{} = {}.{}",
                            shape_dimension_name(program, dimension.dimension),
                            enumeration.name,
                            variant.name
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let label = format!("Expected attachment shape `{shape}`");
                ShapeSelectionCandidateReport {
                    label,
                    present_fields: candidate.present_fields.clone(),
                }
            })
            .collect();
        ShapeSelectionFailureReport {
            header: "Could not select the attachment shape: managed metadata did not match any declared shape".to_owned(),
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
    unreachable!("shape evidence belongs to a managed source field")
}

fn shape_dimension_name(program: &Program, dimension: ResolvedShapeDimension) -> &str {
    match dimension {
        ResolvedShapeDimension::Global(target) => program
            .globals
            .iter()
            .filter_map(|global| global.binding.simple_binding())
            .find(|binding| binding.id == target)
            .map(|binding| binding.name.as_str())
            .expect("shape dimensions refer to declared globals"),
        ResolvedShapeDimension::StateField(target) => program
            .state
            .iter()
            .flat_map(|state| state.all_fields())
            .find(|field| field.id == target)
            .map(|field| field.name.as_str())
            .expect("shape dimensions refer to declared state fields"),
    }
}

struct ManagedEvidenceGroup {
    alternatives: Vec<Vec<(ResolvedShapeDimension, EnumVariantId)>>,
    fields: Vec<ManagedFieldId>,
}

pub(crate) fn automatic_shape_selection(
    program: &Program,
    semantics: &SemanticModel,
) -> AutomaticShapeSelection {
    automatic_shape_selection_with(
        program,
        |dimension| {
            let ty = match dimension {
                ResolvedShapeDimension::Global(value) => semantics.value_type(value)?,
                ResolvedShapeDimension::StateField(value) => semantics.value_type(value)?,
            };
            let TypeKind::Enum(enumeration) = semantics.types().kind(ty) else {
                return None;
            };
            Some(*enumeration)
        },
        |field| {
            semantics
                .managed_field_shape_predicate(field)
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

pub(crate) fn automatic_shape_selection_with(
    program: &Program,
    enum_for_dimension: impl Fn(ResolvedShapeDimension) -> Option<EnumId>,
    predicates_for_field: impl Fn(ManagedFieldId) -> Vec<Vec<(ResolvedShapeDimension, EnumVariantId)>>,
) -> AutomaticShapeSelection {
    let mut source_dimensions = Vec::new();
    for field in program
        .managed_class_declarations()
        .into_iter()
        .flat_map(|class| class.all_fields())
    {
        for alternative in predicates_for_field(field.id) {
            for (dimension, _) in alternative {
                if matches!(dimension, ResolvedShapeDimension::StateField(_)) {
                    continue;
                }
                if !source_dimensions.contains(&dimension) {
                    source_dimensions.push(dimension);
                }
            }
        }
    }
    if source_dimensions.is_empty() {
        return AutomaticShapeSelection::NotDeclared;
    }
    source_dimensions.sort_by_key(|dimension| match dimension {
        ResolvedShapeDimension::Global(value) => (0, value.index()),
        ResolvedShapeDimension::StateField(value) => (1, value.index()),
    });
    let mut dimensions = Vec::with_capacity(source_dimensions.len());
    let mut combination_count = 1usize;
    for dimension in source_dimensions {
        let Some(enumeration) = enum_for_dimension(dimension) else {
            return AutomaticShapeSelection::RequiresExplicit(
                ShapeSelectionReason::IndistinguishableEvidence,
            );
        };
        let declaration = program
            .enum_declaration(enumeration)
            .expect("checked shape dimensions use source enums");
        if declaration
            .variants
            .iter()
            .any(|variant| variant.payload.is_some())
        {
            return AutomaticShapeSelection::RequiresExplicit(
                ShapeSelectionReason::PayloadVariants,
            );
        }
        combination_count = match combination_count.checked_mul(declaration.variants.len()) {
            Some(count) if count <= MAX_ENUMERATED_SHAPE_COMBINATIONS => count,
            count => {
                return AutomaticShapeSelection::RequiresExplicit(
                    ShapeSelectionReason::CandidateLimit {
                        combinations: count,
                    },
                );
            }
        };
        dimensions.push(ShapeSelectionDimension {
            dimension,
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
            return AutomaticShapeSelection::RequiresExplicit(
                ShapeSelectionReason::IndistinguishableEvidence,
            );
        }
    }

    AutomaticShapeSelection::Available(ShapeSelectionPlan {
        dimensions,
        evidence_fields,
        candidates,
    })
}

fn enumerate_candidates(
    dimensions: &[ShapeSelectionDimension],
    groups: &[ManagedEvidenceGroup],
    dimension_index: usize,
    variants: &mut Vec<EnumVariantId>,
    output: &mut Vec<ShapeSelectionCandidate>,
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
        .map(|(dimension, variant)| (dimension.dimension, variant))
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
    output.push(ShapeSelectionCandidate {
        variants: variants.clone(),
        present_fields,
    });
}

/// Whether user `onAttach` code explicitly owns shape selection. Returns
/// inside closures belong to those closures and do not count.
pub(crate) fn has_explicit_shape_selection(program: &Program) -> bool {
    let Some(action) = program
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OnAttach)
    else {
        return false;
    };
    shape_global_assignments(program, action)
        .is_some_and(|(dimensions, assigned)| !assigned.is_empty() && assigned == dimensions)
}

/// Returns the attachment-shape globals assigned directly by `onAttach`, but
/// only when that action owns every global dimension. Mixing user selection
/// with metadata selection would make the managed schema and source value
/// disagree, so it is intentionally not treated as explicit selection.
fn shape_global_assignments(
    program: &Program,
    action: &crate::ast::Action,
) -> Option<(HashSet<String>, HashSet<String>)> {
    struct DimensionCollector<'a> {
        globals: &'a HashSet<&'a str>,
        dimensions: HashSet<String>,
    }
    impl<'ast> Visitor<'ast> for DimensionCollector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Path(path) = &expression.kind
                && let [name] = path.as_slice()
                && self.globals.contains(name.as_str())
            {
                self.dimensions.insert(name.clone());
            }
            visit::walk_expr(self, expression);
        }
    }
    struct AssignmentCollector<'a> {
        dimensions: &'a HashSet<String>,
        assigned: HashSet<String>,
    }
    impl<'ast> Visitor<'ast> for AssignmentCollector<'_> {
        fn visit_stmt(&mut self, statement: &'ast crate::ast::Stmt) {
            if let crate::ast::Stmt::Assign { name, op: None, .. } = statement
                && self.dimensions.contains(name)
            {
                self.assigned.insert(name.clone());
            }
            visit::walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if !matches!(expression.kind, ExprKind::Closure { .. }) {
                visit::walk_expr(self, expression);
            }
        }
    }

    let globals = program
        .globals
        .iter()
        .filter_map(|global| global.binding.simple_binding())
        .map(|binding| binding.name.as_str())
        .collect::<HashSet<_>>();
    let mut collector = DimensionCollector {
        globals: &globals,
        dimensions: HashSet::new(),
    };
    for condition in program
        .state
        .iter()
        .flat_map(|state| &state.conditional_fields)
        .filter_map(|group| group.condition.as_ref())
        .chain(
            program
                .managed_class_declarations()
                .into_iter()
                .flat_map(|class| &class.conditional_fields)
                .filter_map(|group| group.condition.as_ref()),
        )
    {
        collector.visit_expr(condition);
    }
    if collector.dimensions.is_empty() {
        return None;
    }
    let assigned = {
        let mut assignments = AssignmentCollector {
            dimensions: &collector.dimensions,
            assigned: HashSet::new(),
        };
        assignments.visit_block(&action.body);
        assignments.assigned
    };
    Some((collector.dimensions, assigned))
}

pub(crate) fn partial_shape_selection(program: &Program) -> Option<(Vec<String>, Vec<String>)> {
    let action = program
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OnAttach)?;
    let (dimensions, assigned) = shape_global_assignments(program, action)?;
    if assigned.is_empty() || assigned == dimensions {
        return None;
    }
    let mut assigned = assigned.into_iter().collect::<Vec<_>>();
    let mut missing = dimensions
        .into_iter()
        .filter(|dimension| !assigned.contains(dimension))
        .collect::<Vec<_>>();
    assigned.sort();
    missing.sort();
    Some((assigned, missing))
}
