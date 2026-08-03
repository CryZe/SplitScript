use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ast::{
        ArrayTypeId, AsyncTypeId, BinaryOp, EnumDecl, EnumId, ExprId, OptionTypeId, Program,
        RecordId, ResultTypeId,
    },
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{
        IntrinsicId, RuntimeRepresentation, StandardLibrary, StdlibCapabilityId, StdlibTypeId,
    },
    types::{ResolvedArrayType, TypeId, TypeKind},
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
    gc_asyncs: BTreeSet<AsyncTypeId>,
    display_functions: BTreeMap<StdlibTypeId, FunctionInstance>,
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
        let mut pending_functions = Vec::new();
        for body in wasm_ir.bodies() {
            if matches!(body.owner, BodyOwner::Action(_)) {
                collect_block_expression_roots(&body.entry, wasm_ir, None, &mut pending);
                collect_assignment_function_roots(
                    &body.entry,
                    wasm_ir,
                    None,
                    &mut pending_functions,
                );
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
        loop {
            while let Some((owner, function)) = pending_functions.pop() {
                let function = owner.as_ref().map_or(function.clone(), |owner| {
                    semantics.specialize_function_instance(owner, &function)
                });
                if reachable.functions.insert(function.clone()) {
                    let body = wasm_ir
                        .body(BodyOwner::Function(function.clone()))
                        .expect("resolved user functions have Wasm IR bodies");
                    collect_block_expression_roots(
                        &body.entry,
                        wasm_ir,
                        Some(function.clone()),
                        &mut pending,
                    );
                    collect_assignment_function_roots(
                        &body.entry,
                        wasm_ir,
                        Some(function),
                        &mut pending_functions,
                    );
                }
            }
            let Some((owner, id)) = pending.pop() else {
                break;
            };
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
                if let Some(function) = function {
                    pending_functions.push((None, function));
                }
            }

            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |owner| semantics.specialize_type(owner, ty))
            };
            let mut display_sources = Vec::new();
            match &expression.kind {
                wasm_ir::ExpressionKind::Cast { value }
                    if matches!(
                        semantics.types().kind(specialize(expression.ty)),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    display_sources.push(
                        wasm_ir
                            .expression(*value)
                            .expect("cast operands belong to Wasm IR")
                            .ty,
                    );
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    display_sources.extend(parts.iter().filter_map(|part| match part {
                        wasm_ir::InterpolatedPart::Expression {
                            string_conversion_source,
                            ..
                        } => *string_conversion_source,
                        wasm_ir::InterpolatedPart::Text(_) => None,
                    }));
                }
                wasm_ir::ExpressionKind::Call { target, arguments } => {
                    let converted = match target {
                        wasm_ir::CallTarget::Intrinsic {
                            intrinsic: IntrinsicId::Print,
                            ..
                        } => arguments.first(),
                        wasm_ir::CallTarget::Intrinsic {
                            intrinsic: IntrinsicId::TimerSetVariable,
                            ..
                        } => arguments.get(1),
                        _ => None,
                    };
                    if let Some(argument) = converted {
                        display_sources.push(
                            wasm_ir
                                .expression(*argument)
                                .expect("call arguments belong to Wasm IR")
                                .ty,
                        );
                    }
                }
                _ => {}
            }
            for source in display_sources.into_iter().map(specialize) {
                let Some((standard, function)) =
                    super::standard_display_function(source, program, semantics, standard_library)
                else {
                    continue;
                };
                reachable
                    .display_functions
                    .insert(standard, function.clone());
                pending_functions.push((None, function));
            }
        }

        // Type reachability includes every value shape referenced by emitted
        // storage or signatures, not only the result types of live expressions.
        let mut type_roots = Vec::new();
        if let Some(state) = &program.state {
            for field in state.all_fields() {
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
        for transform in wasm_ir.state_transforms() {
            type_roots.extend(transform.locals.iter().map(|local| local.ty));
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

    /// Retains constructed GC layouts referenced by the signatures of the
    /// runtime helpers selected after expression reachability is known.
    pub fn require_runtime_helper_types(
        &mut self,
        dependencies: &super::dependencies::BackendDependencies,
        arrays: &[ResolvedArrayType],
        semantics: &SemanticModel,
    ) {
        self.gc_arrays
            .extend(super::runtime_helper_registry::required_array_layouts(
                dependencies.helpers(),
                arrays,
                semantics,
            ));
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionInstance> {
        self.functions.iter()
    }

    pub fn expression_instances(
        &self,
    ) -> impl Iterator<Item = (Option<FunctionInstance>, ExprId)> + '_ {
        self.expression_instances.iter().cloned()
    }

    pub fn display_functions(&self) -> impl Iterator<Item = (StdlibTypeId, &FunctionInstance)> {
        self.display_functions
            .iter()
            .map(|(ty, function)| (*ty, function))
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

    pub fn contains_async_type(&self, future: AsyncTypeId) -> bool {
        self.gc_asyncs.contains(&future)
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
                TypeKind::StateSnapshot => {
                    pending.extend(
                        program
                            .state
                            .as_ref()
                            .expect("checked programs have state declarations")
                            .all_fields()
                            .map(|field| {
                                semantics
                                    .value_type(field.id)
                                    .expect("checked state fields have semantic types")
                            }),
                    );
                }
                TypeKind::SettingsView => {
                    pending.extend(
                        program
                            .settings
                            .iter()
                            .filter_map(|setting| semantics.value_type(setting.id)),
                    );
                }
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
                TypeKind::Async { layout, value } => {
                    self.gc_asyncs.insert(*layout);
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
                TypeKind::Builtin(_)
                | TypeKind::StateSnapshot
                | TypeKind::SettingsView
                | TypeKind::GenericParameter { .. } => {}
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
                | TypeKind::Result { .. }
                | TypeKind::Async { .. } => {}
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

fn collect_assignment_function_roots(
    block: &wasm_ir::Block,
    program: &wasm_ir::Program,
    owner: Option<FunctionInstance>,
    output: &mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
) {
    struct AssignmentCollector<'a> {
        owner: Option<FunctionInstance>,
        output: &'a mut Vec<(Option<FunctionInstance>, FunctionInstance)>,
    }

    impl Visitor for AssignmentCollector<'_> {
        fn visit_statement(&mut self, statement: &wasm_ir::Statement, program: &wasm_ir::Program) {
            if let wasm_ir::Statement::Store {
                operation:
                    Some(wasm_ir::AssignmentOperation::Call(wasm_ir::CallTarget::UserMethod {
                        function,
                        ..
                    })),
                ..
            } = statement
            {
                self.output.push((self.owner.clone(), function.clone()));
            }
            wasm_ir::walk_statement(self, statement, program);
        }
    }

    AssignmentCollector { owner, output }.visit_block(block, program);
}
