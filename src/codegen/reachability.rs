use std::collections::BTreeSet;

use crate::{
    ast::{
        ArrayTypeId, BinaryOp, EnumDecl, EnumId, ExprId, FunctionId, OptionTypeId, Program,
        RecordId, ResultTypeId,
    },
    semantic::{ResolvedCall, SemanticModel},
    stdlib::StdlibTypeId,
    types::{TypeId, TypeKind},
    wasm_ir::{self, BodyOwner, FallbackBranch, Terminator},
};

#[derive(Debug, Default)]
pub(super) struct Reachability {
    functions: BTreeSet<FunctionId>,
    expressions: BTreeSet<ExprId>,
    equality_records: BTreeSet<RecordId>,
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
    ) -> Self {
        let mut pending = Vec::new();
        for body in wasm_ir.bodies() {
            if matches!(body.owner, BodyOwner::Action(_)) {
                collect_block_expressions(&body.entry, &mut pending);
            }
        }
        pending.extend(
            wasm_ir
                .state_expressions()
                .map(|expression| expression.expression),
        );
        pending.extend(
            wasm_ir
                .global_initializers()
                .map(|(_, expression)| expression),
        );

        let mut reachable = Self::default();
        while let Some(id) = pending.pop() {
            if !reachable.expressions.insert(id) {
                continue;
            }
            let expression = wasm_ir
                .expression(id)
                .expect("reachable expressions belong to Wasm IR");
            collect_expression_children(&expression.kind, &mut pending);
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
                reachable.require_equality(ty, program, enums, semantics);
            }
            if let wasm_ir::ExpressionKind::Call { target, .. } = &expression.kind {
                let function = match target {
                    ResolvedCall::UserFunction { function }
                    | ResolvedCall::UserMethod { function, .. } => Some(*function),
                    ResolvedCall::StandardLibrary { .. }
                    | ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
                if let Some(function) = function
                    && reachable.functions.insert(function)
                {
                    let body = wasm_ir
                        .body(BodyOwner::Function(function))
                        .expect("resolved user functions have Wasm IR bodies");
                    collect_block_expressions(&body.entry, &mut pending);
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

        for body in wasm_ir.bodies().filter(|body| match body.owner {
            BodyOwner::Action(_) => true,
            BodyOwner::Function(function) => reachable.functions.contains(&function),
        }) {
            type_roots.extend(body.locals.iter().map(|local| local.ty));
        }
        for expression in wasm_ir.state_expressions() {
            type_roots.extend(expression.locals.iter().map(|local| local.ty));
        }
        for function in program
            .functions
            .iter()
            .filter(|function| reachable.functions.contains(&function.id))
        {
            type_roots.extend(function.params.iter().map(|parameter| {
                semantics
                    .value_type(parameter.id)
                    .expect("checked function parameters have types")
            }));
            type_roots.push(
                semantics
                    .function_result(function.id)
                    .expect("checked functions have result types"),
            );
        }
        // Interpolation's concatenation helper receives a compiler-generated
        // Array<String>; that layout is not the type of any source expression.
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
        for expression in reachable.expressions.iter().map(|id| {
            wasm_ir
                .expression(*id)
                .expect("reachable expressions exist")
        }) {
            type_roots.push(expression.ty);
            if let Some(conversion) = expression.conversion {
                type_roots.extend([conversion.source, conversion.target]);
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::InterpolatedString(_) => type_roots.push(
                    string_array.expect("interpolation has a compiler-generated String array"),
                ),
                wasm_ir::ExpressionKind::Call { target, .. } => match target {
                    ResolvedCall::UserMethod { receiver_type, .. } => {
                        type_roots.push(*receiver_type);
                    }
                    ResolvedCall::StandardLibrary {
                        type_arguments,
                        receiver_type,
                        ..
                    } => {
                        type_roots.extend(type_arguments.iter().copied());
                        type_roots.extend(*receiver_type);
                    }
                    ResolvedCall::UserFunction { .. }
                    | ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => {}
                },
                wasm_ir::ExpressionKind::Propagate { target, .. } => type_roots.push(*target),
                _ => {}
            }
        }
        reachable.require_types(type_roots, program, enums, semantics);
        reachable
    }

    pub fn contains_function(&self, function: FunctionId) -> bool {
        self.functions.contains(&function)
    }

    pub fn contains_expression(&self, expression: ExprId) -> bool {
        self.expressions.contains(&expression)
    }

    pub fn requires_record_equality(&self, record: RecordId) -> bool {
        self.equality_records.contains(&record)
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
    ) {
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Builtin(_) | TypeKind::Standard(_) => {}
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
                TypeKind::Array { layout, element } => {
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
    ) {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Standard(StdlibTypeId::String) => self.string_equality = true,
                TypeKind::Builtin(_) | TypeKind::Standard(_) => {}
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

fn collect_block_expressions(block: &wasm_ir::Block, output: &mut Vec<ExprId>) {
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::Store { value, .. }
            | wasm_ir::Statement::Evaluate {
                expression: value, ..
            } => output.push(*value),
            wasm_ir::Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                output.push(*condition);
                collect_block_expressions(then_block, output);
                collect_block_expressions(else_block, output);
            }
            wasm_ir::Statement::While { condition, body } => {
                output.push(*condition);
                collect_block_expressions(body, output);
            }
        }
    }
    match &block.terminator {
        Terminator::Fallthrough | Terminator::Break | Terminator::Continue => {}
        Terminator::AsyncWhile {
            condition,
            body,
            continuation,
            ..
        } => {
            output.push(*condition);
            collect_block_expressions(body, output);
            collect_block_expressions(continuation, output);
        }
        Terminator::Return(value) => output.extend(value),
        Terminator::Throw { error, .. } => output.push(*error),
        Terminator::Suspend {
            value,
            continuation,
            ..
        } => {
            output.push(*value);
            collect_block_expressions(continuation, output);
        }
    }
}

fn collect_expression_children(kind: &wasm_ir::ExpressionKind, output: &mut Vec<ExprId>) {
    match kind {
        wasm_ir::ExpressionKind::None
        | wasm_ir::ExpressionKind::Bool(_)
        | wasm_ir::ExpressionKind::Int(_)
        | wasm_ir::ExpressionKind::Float(_)
        | wasm_ir::ExpressionKind::String(_)
        | wasm_ir::ExpressionKind::Signature(_)
        | wasm_ir::ExpressionKind::Path { .. } => {}
        wasm_ir::ExpressionKind::Member { receiver, .. } => output.push(*receiver),
        wasm_ir::ExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                if let wasm_ir::InterpolatedPart::Expression { expression, .. } = part {
                    output.push(*expression);
                }
            }
        }
        wasm_ir::ExpressionKind::Array(elements) => output.extend(elements),
        wasm_ir::ExpressionKind::Record { fields, .. } => {
            output.extend(fields.iter().map(|(_, value)| value));
        }
        wasm_ir::ExpressionKind::Enum { payload, .. } => output.extend(payload),
        wasm_ir::ExpressionKind::Unary { operand, .. } => output.push(*operand),
        wasm_ir::ExpressionKind::Cast { value }
        | wasm_ir::ExpressionKind::Propagate { value, .. } => output.push(*value),
        wasm_ir::ExpressionKind::Binary { left, right, .. } => {
            output.push(*left);
            output.push(*right);
        }
        wasm_ir::ExpressionKind::Call { arguments, .. } => output.extend(arguments),
        wasm_ir::ExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            output.push(*condition);
            output.push(*then_expr);
            output.push(*else_expr);
        }
        wasm_ir::ExpressionKind::Fallback { value, fallback } => {
            output.push(*value);
            match fallback {
                FallbackBranch::Value(value) => output.push(*value),
                FallbackBranch::Return(value) => output.extend(value),
                FallbackBranch::Break | FallbackBranch::Continue => {}
            }
        }
        wasm_ir::ExpressionKind::Match { value, arms } => {
            output.push(*value);
            for arm in arms {
                output.extend(arm.guard);
                output.push(arm.value);
            }
        }
    }
}
