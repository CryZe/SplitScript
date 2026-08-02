use std::collections::BTreeSet;

use crate::{
    ast::{
        ArrayTypeId, BinaryOp, EnumDecl, EnumId, ExprId, OptionTypeId, Program, RecordId,
        ResultTypeId,
    },
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibTypeId},
    types::{TypeId, TypeKind},
    wasm_ir::{self, BodyOwner, Visitor},
};

#[derive(Debug, Default)]
pub(super) struct Reachability {
    functions: BTreeSet<FunctionInstance>,
    expressions: BTreeSet<ExprId>,
    expression_instances: BTreeSet<(Option<FunctionInstance>, ExprId)>,
    equality_records: BTreeSet<RecordId>,
    equality_standard_records: BTreeSet<StdlibTypeId>,
    equality_enums: BTreeSet<EnumId>,
    equality_options: BTreeSet<OptionTypeId>,
    equality_results: BTreeSet<ResultTypeId>,
    string_equality: bool,
    gc_records: BTreeSet<RecordId>,
    gc_enums: BTreeSet<EnumId>,
    gc_arrays: BTreeSet<ArrayTypeId>,
    gc_options: BTreeSet<OptionTypeId>,
    gc_results: BTreeSet<ResultTypeId>,
}

impl Reachability {
    pub fn analyze(
        program: &Program,
        semantics: &SemanticModel,
        enums: &[EnumDecl],
        wasm_ir: &wasm_ir::Program,
        standard_library: &StandardLibrary,
    ) -> Self {
        let mut pending = Vec::new();
        for body in wasm_ir.bodies() {
            if matches!(body.owner, BodyOwner::Action(_)) {
                collect_block_expression_roots(&body.entry, wasm_ir, None, &mut pending);
            }
        }
        pending.extend(
            wasm_ir
                .state_expressions()
                .map(|expression| (None, expression.expression)),
        );
        pending.extend(
            wasm_ir
                .global_initializers()
                .map(|(_, expression)| (None, expression)),
        );

        let mut reachable = Self::default();
        while let Some((owner, id)) = pending.pop() {
            if !reachable.expression_instances.insert((owner.clone(), id)) {
                continue;
            }
            reachable.expressions.insert(id);
            let expression = wasm_ir
                .expression(id)
                .expect("reachable expressions belong to Wasm IR");
            wasm_ir::visit_expression_children(&expression.kind, |child| {
                pending.push((owner.clone(), child))
            });
            if let wasm_ir::ExpressionKind::Binary {
                op: BinaryOp::Eq | BinaryOp::Ne,
                left,
                ..
            } = expression.kind
            {
                let ty = wasm_ir
                    .expression(left)
                    .expect("binary operands belong to Wasm IR")
                    .ty;
                let ty = owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty));
                reachable.require_equality(ty, program, enums, semantics, standard_library);
            }
            if let wasm_ir::ExpressionKind::Call { target, .. } = &expression.kind {
                let function = match target {
                    wasm_ir::CallTarget::UserFunction { function }
                    | wasm_ir::CallTarget::UserMethod { function, .. } => Some(function.clone()),
                    wasm_ir::CallTarget::Intrinsic { .. }
                    | wasm_ir::CallTarget::ResultError { .. }
                    | wasm_ir::CallTarget::OptionSome { .. }
                    | wasm_ir::CallTarget::ResultSuccess { .. } => None,
                };
                let function = function.map(|function| {
                    owner.as_ref().map_or(function.clone(), |owner| {
                        semantics.specialize_function_instance(owner, &function)
                    })
                });
                if let Some(function) = function
                    && reachable.functions.insert(function.clone())
                {
                    let body = wasm_ir
                        .body(BodyOwner::Function(function.clone()))
                        .expect("resolved user functions have Wasm IR bodies");
                    collect_block_expression_roots(
                        &body.entry,
                        wasm_ir,
                        Some(function),
                        &mut pending,
                    );
                }
            }
        }

        // Type reachability includes every value shape referenced by emitted
        // storage or signatures, not only the result types of live expressions.
        let mut type_roots = Vec::new();
        if let Some(state) = &program.state {
            for field in &state.fields {
                type_roots.push(
                    semantics
                        .value_type(field.id)
                        .expect("checked state fields have types"),
                );
                type_roots.push(
                    semantics
                        .state_poll_result(field.id)
                        .expect("checked state fields have poll-result types"),
                );
            }
        }
        type_roots.extend(wasm_ir.global_initializers().map(|(global, _)| {
            semantics
                .value_type(global)
                .expect("checked globals have types")
        }));
        type_roots.extend(
            program
                .settings
                .iter()
                .filter_map(|setting| semantics.value_type(setting.id)),
        );

        for body in wasm_ir
            .bodies()
            .filter(|body| matches!(body.owner, BodyOwner::Action(_)))
        {
            type_roots.extend(body.locals.iter().map(|local| local.ty));
        }
        for instance in &reachable.functions {
            let body = wasm_ir
                .body(BodyOwner::Function(instance.clone()))
                .expect("reachable functions have template bodies");
            type_roots.extend(
                body.locals
                    .iter()
                    .map(|local| semantics.specialize_type(instance, local.ty)),
            );
        }
        for expression in wasm_ir.state_expressions() {
            type_roots.extend(expression.locals.iter().map(|local| local.ty));
        }
        for instance in &reachable.functions {
            let function = program
                .functions
                .iter()
                .find(|function| function.id == instance.function)
                .expect("reachable functions have declarations");
            type_roots.extend(function.params.iter().map(|parameter| {
                semantics.specialize_type(
                    instance,
                    semantics
                        .value_type(parameter.id)
                        .expect("checked function parameters have types"),
                )
            }));
            type_roots.push(
                semantics.specialize_type(
                    instance,
                    semantics
                        .function_result(function.id)
                        .expect("checked functions have result types"),
                ),
            );
        }
        // Interpolation's concatenation helper receives a compiler-generated
        // [String]; that layout is not the type of any source expression.
        let string_array = semantics.types().iter().find_map(|(ty, kind)| {
            let TypeKind::Array { element, .. } = kind else {
                return None;
            };
            matches!(
                semantics.types().kind(*element),
                TypeKind::Standard(StdlibTypeId::String)
            )
            .then_some(ty)
        });
        for (owner, id) in &reachable.expression_instances {
            let expression = wasm_ir
                .expression(*id)
                .expect("reachable expressions exist");
            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty))
            };
            type_roots.push(specialize(expression.ty));
            if let Some(conversion) = expression.conversion {
                type_roots.extend([specialize(conversion.source), specialize(conversion.target)]);
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::InterpolatedString(_) => type_roots.push(
                    string_array.expect("interpolation has a compiler-generated String array"),
                ),
                wasm_ir::ExpressionKind::Call { target, .. } => match target {
                    wasm_ir::CallTarget::UserMethod { receiver_type, .. } => {
                        type_roots.push(specialize(*receiver_type));
                    }
                    wasm_ir::CallTarget::Intrinsic {
                        type_arguments,
                        receiver_type,
                        ..
                    } => {
                        type_roots.extend(type_arguments.iter().copied().map(specialize));
                        type_roots.extend(receiver_type.map(specialize));
                    }
                    wasm_ir::CallTarget::UserFunction { .. }
                    | wasm_ir::CallTarget::ResultError { .. }
                    | wasm_ir::CallTarget::OptionSome { .. }
                    | wasm_ir::CallTarget::ResultSuccess { .. } => {}
                },
                wasm_ir::ExpressionKind::Propagate { target, .. } => {
                    type_roots.push(specialize(*target));
                }
                _ => {}
            }
        }
        // Standard GC structs are currently emitted as one recursive catalog
        // group. Their constructed field layouts therefore need matching
        // dynamic GC types even when no user expression reaches the owner.
        type_roots.extend(standard_library.fields().iter().map(|field| {
            semantics
                .standard_field_type(field.id)
                .expect("checked standard fields have semantic types")
        }));
        reachable.require_types(type_roots, program, enums, semantics, standard_library);
        reachable
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionInstance> {
        self.functions.iter()
    }

    pub fn contains_expression(&self, expression: ExprId) -> bool {
        self.expressions.contains(&expression)
    }

    pub fn requires_record_equality(&self, record: RecordId) -> bool {
        self.equality_records.contains(&record)
    }

    pub fn requires_standard_record_equality(&self, record: StdlibTypeId) -> bool {
        self.equality_standard_records.contains(&record)
    }

    pub fn requires_enum_equality(&self, enumeration: EnumId) -> bool {
        self.equality_enums.contains(&enumeration)
    }

    pub fn requires_option_equality(&self, option: OptionTypeId) -> bool {
        self.equality_options.contains(&option)
    }

    pub fn requires_result_equality(&self, result: ResultTypeId) -> bool {
        self.equality_results.contains(&result)
    }

    pub fn requires_string_equality(&self) -> bool {
        self.string_equality
    }

    pub fn contains_record_type(&self, record: RecordId) -> bool {
        self.gc_records.contains(&record)
    }

    pub fn contains_enum_type(&self, enumeration: EnumId) -> bool {
        self.gc_enums.contains(&enumeration)
    }

    pub fn contains_array_type(&self, array: ArrayTypeId) -> bool {
        self.gc_arrays.contains(&array)
    }

    pub fn contains_option_type(&self, option: OptionTypeId) -> bool {
        self.gc_options.contains(&option)
    }

    pub fn contains_result_type(&self, result: ResultTypeId) -> bool {
        self.gc_results.contains(&result)
    }

    fn require_types(
        &mut self,
        roots: impl IntoIterator<Item = TypeId>,
        program: &Program,
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
    ) {
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Builtin(_) | TypeKind::GenericParameter { .. } => {}
                TypeKind::Standard(standard) => {
                    if matches!(
                        standard_library.type_decl(*standard).representation,
                        RuntimeRepresentation::GcStruct { .. }
                    ) {
                        pending.extend(standard_library.fields_of(*standard).map(|field| {
                            semantics
                                .standard_field_type(field.id)
                                .expect("checked standard fields have semantic types")
                        }));
                    }
                }
                TypeKind::Record(record) => {
                    self.gc_records.insert(*record);
                    let declaration = program
                        .records
                        .iter()
                        .find(|declaration| declaration.id == *record)
                        .expect("record IDs refer to declarations");
                    pending.extend(declaration.fields.iter().map(|field| {
                        semantics
                            .record_field_type(field.id)
                            .expect("checked record fields have types")
                    }));
                }
                TypeKind::Enum(enumeration) => {
                    self.gc_enums.insert(*enumeration);
                    let declaration = enums
                        .iter()
                        .find(|declaration| declaration.id == *enumeration)
                        .expect("enum IDs refer to declarations");
                    pending.extend(
                        declaration
                            .variants
                            .iter()
                            .filter_map(|variant| semantics.enum_variant_payload(variant.id)),
                    );
                }
                TypeKind::Array {
                    layout, element, ..
                } => {
                    self.gc_arrays.insert(*layout);
                    pending.push(*element);
                }
                TypeKind::Option { layout, value } => {
                    self.gc_options.insert(*layout);
                    pending.push(*value);
                }
                TypeKind::Result { layout, value } => {
                    self.gc_results.insert(*layout);
                    pending.push(*value);
                }
            }
        }
    }

    fn require_equality(
        &mut self,
        root: TypeId,
        program: &Program,
        enums: &[EnumDecl],
        semantics: &SemanticModel,
        standard_library: &StandardLibrary,
    ) {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Standard(StdlibTypeId::String) => self.string_equality = true,
                TypeKind::Standard(standard) => {
                    let library = standard_library;
                    let declaration = library.type_decl(*standard);
                    if library.type_has_capability(*standard, StdlibCapabilityId::Equatable)
                        && matches!(
                            declaration.representation,
                            RuntimeRepresentation::GcStruct { .. }
                        )
                        && self.equality_standard_records.insert(*standard)
                    {
                        pending.extend(library.fields_of(*standard).map(|field| {
                            semantics
                                .standard_field_type(field.id)
                                .expect("checked standard fields have semantic types")
                        }));
                    }
                }
                TypeKind::Builtin(_) | TypeKind::GenericParameter { .. } => {}
                TypeKind::Record(record) if self.equality_records.insert(*record) => {
                    let declaration = program
                        .records
                        .iter()
                        .find(|declaration| declaration.id == *record)
                        .expect("record IDs refer to declarations");
                    pending.extend(declaration.fields.iter().map(|field| {
                        semantics
                            .record_field_type(field.id)
                            .expect("checked record fields have types")
                    }));
                }
                TypeKind::Enum(enumeration) if self.equality_enums.insert(*enumeration) => {
                    let declaration = enums
                        .iter()
                        .find(|declaration| declaration.id == *enumeration)
                        .expect("enum IDs refer to declarations");
                    pending.extend(
                        declaration
                            .variants
                            .iter()
                            .filter_map(|variant| semantics.enum_variant_payload(variant.id)),
                    );
                }
                TypeKind::Option { layout, value } if self.equality_options.insert(*layout) => {
                    pending.push(*value);
                }
                TypeKind::Result { layout, value } if self.equality_results.insert(*layout) => {
                    self.string_equality = true;
                    pending.push(*value);
                }
                TypeKind::Record(_)
                | TypeKind::Enum(_)
                | TypeKind::Array { .. }
                | TypeKind::Option { .. }
                | TypeKind::Result { .. } => {}
            }
        }
    }
}

fn collect_block_expression_roots(
    block: &wasm_ir::Block,
    program: &wasm_ir::Program,
    owner: Option<FunctionInstance>,
    output: &mut Vec<(Option<FunctionInstance>, ExprId)>,
) {
    struct RootCollector<'a> {
        owner: Option<FunctionInstance>,
        output: &'a mut Vec<(Option<FunctionInstance>, ExprId)>,
    }

    impl Visitor for RootCollector<'_> {
        fn visit_expression(
            &mut self,
            expression: &wasm_ir::Expression,
            _program: &wasm_ir::Program,
        ) {
            self.output.push((self.owner.clone(), expression.id));
        }
    }

    RootCollector { owner, output }.visit_block(block, program);
}
