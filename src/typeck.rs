use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic,
    ast::{
        ActionKind, ArrayTypeDecl, ArrayTypeId, BinaryOp, Block, EnumDecl, EnumTypeId, Expr,
        ExprId, ExprKind, FallbackBranch, FunctionId, InterpolatedPart, MatchPattern,
        OptionTypeDecl, Program, RecordDecl, ResultTypeDecl, SettingKind, Span, StateSource, Stmt,
        SuspensionMode, TypeNameId, TypeRef, UnaryOp, ValueId, VariableDecl,
    },
    inference::{
        ArrayLayout, InferenceContext, InferenceError, OptionLayout, Requirements, ResultLayout,
        Type, fits_unsigned_literal, type_may_have_capability,
    },
    semantic::{
        PendingResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue,
        ResolvedWrapperPattern, SemanticBuilder, SemanticModel, ValueConversionKind,
    },
    signature::parse_signature,
    stdlib::{
        Availability, CallCandidate, DeclaredTypeRef, ItemKind, ParameterRule, StandardLibrary,
        StdlibItem, StdlibItemId, StdlibTypeId, TypeConstraint, TypeRef as CatalogTypeRef,
    },
    types::{TypeKind, TypeStore},
    visit::{self, Visitor},
};

#[derive(Clone, Copy)]
struct Binding {
    id: Option<ValueId>,
    ty: Type,
    mutable: bool,
    debug_only: bool,
}

struct PathResolution {
    ty: Type,
    value: Option<ResolvedValue>,
    members: Option<Vec<ResolvedMember>>,
}

#[derive(Clone)]
struct DeferredMemberPath {
    expression: ExprId,
    receiver: Type,
    fields: Vec<String>,
    result: Type,
    span: Span,
}

struct MethodReceiver {
    ty: Type,
    value: ResolvedValue,
    members: Vec<ResolvedMember>,
}

#[derive(Clone)]
struct FunctionSignature {
    id: FunctionId,
    params: Vec<Type>,
    result: Type,
}

const REQUIRE_EQUATABLE: Requirements = Requirements::EQUATABLE;
const REQUIRE_NUMERIC: Requirements = Requirements::NUMERIC;
const REQUIRE_INTEGER: Requirements = Requirements::INTEGER;
const REQUIRE_SIGNED: Requirements = Requirements::SIGNED;
const REQUIRE_FLOAT: Requirements = Requirements::FLOAT;
const REQUIRE_STRING_CAST: Requirements = Requirements::STRING_CAST;
const REQUIRE_MEMORY_READABLE: Requirements = Requirements::MEMORY_READABLE;
const REQUIRE_INTERPOLATABLE: Requirements = Requirements::INTERPOLATABLE;

pub struct CheckOutput {
    pub semantics: SemanticModel,
    pub enum_types: Vec<EnumDecl>,
    pub array_types: Vec<ArrayTypeDecl>,
    pub option_types: Vec<OptionTypeDecl>,
    pub result_types: Vec<ResultTypeDecl>,
}

pub struct RecoveringCheckOutput {
    pub output: CheckOutput,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn check(program: &Program) -> Result<CheckOutput, Vec<Diagnostic>> {
    let recovered = check_recovering(program);
    if recovered.diagnostics.is_empty() {
        Ok(recovered.output)
    } else {
        Err(recovered.diagnostics)
    }
}

pub fn check_recovering(program: &Program) -> RecoveringCheckOutput {
    let records = program.records.clone();
    let enums = program.enums.clone();
    let semantic_types = TypeStore::with_source_types(&records, &enums);
    let named_types = program
        .type_names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let id = TypeNameId::from_index(index as u32);
            let ty = if let Some(standard) = StandardLibrary::new().type_by_name(name) {
                Type::Known(semantic_types.id_for_standard(standard.id))
            } else if let Some(record) = records.iter().find(|record| record.name == *name) {
                Type::Known(semantic_types.id_for_record(record.id))
            } else if let Some(enumeration) = enums.iter().find(|item| item.name == *name) {
                Type::Known(semantic_types.id_for_enum(enumeration.id))
            } else {
                unreachable!("the parser only interns known nominal type names")
            };
            (id, ty)
        })
        .collect::<HashMap<_, _>>();
    let array_types = program
        .array_types
        .iter()
        .map(|array| ArrayLayout {
            id: array.id,
            element: syntax_type(array.element, &named_types, &semantic_types),
        })
        .collect::<Vec<_>>();
    let option_types = program
        .option_types
        .iter()
        .map(|option| OptionLayout {
            id: option.id,
            value: syntax_type(option.value, &named_types, &semantic_types),
        })
        .collect::<Vec<_>>();
    let result_types = program
        .result_types
        .iter()
        .map(|result| ResultLayout {
            id: result.id,
            value: syntax_type(result.value, &named_types, &semantic_types),
        })
        .collect::<Vec<_>>();
    let inference = InferenceContext::new(
        semantic_types,
        records.len() as u32 + enums.len() as u32,
        array_types,
        option_types,
        result_types,
    );
    let mut checker = Checker {
        errors: Vec::new(),
        state_fields: HashMap::new(),
        settings: HashMap::new(),
        globals: HashMap::new(),
        functions: HashMap::new(),
        methods: HashMap::new(),
        function_signatures: HashMap::new(),
        debug_functions: program
            .functions
            .iter()
            .filter(|function| function.debug_only)
            .map(|function| function.id)
            .collect(),
        records,
        enums,
        named_types,
        inference,
        scopes: Vec::new(),
        return_ty: Type::Void,
        current_action: ActionKind::WhileAttached,
        current_callable: "top level".to_owned(),
        in_function: false,
        checking_suspension: false,
        debug_context: false,
        loop_depth: 0,
        checking_state_source: false,
        failure_boundary: None,
        used_propagation: false,
        inferred_process_reads: Vec::new(),
        deferred_member_paths: Vec::new(),
        allowing_null: false,
        semantics: SemanticBuilder::default(),
    };

    {
        let state = program.state.as_ref().unwrap();
        for field in &state.fields {
            let ty = if let Some(annotation) = field.annotation {
                checker.syntax_type(annotation)
            } else {
                checker.fresh_inference(Requirements::NONE, None)
            };
            if let Some(standard) = checker.standard_type_id(ty) {
                let declaration = StandardLibrary::new().type_decl(standard);
                if !declaration.value_usage.state_field {
                    checker.error(
                        format!("{} cannot be stored in a state field", declaration.name),
                        field.span,
                    );
                }
            }
            checker.semantics.resolve_value_type(field.id, ty);
            if checker
                .state_fields
                .insert(field.name.clone(), (field.id, ty))
                .is_some()
            {
                checker.error(
                    format!("duplicate state field `{}`", field.name),
                    field.span,
                );
            }
            if let StateSource::Pointer(path) = &field.source {
                if path.offsets.is_empty() {
                    checker.error("a pointer path needs at least one offset", field.span);
                }
                checker.require(ty, REQUIRE_MEMORY_READABLE, field.span);
            }
        }
    }
    for setting in &program.settings {
        if let Some(ty) = setting.value_type() {
            let ty = checker.syntax_type(ty);
            checker.semantics.resolve_value_type(setting.id, ty);
            if checker
                .settings
                .insert(setting.name.clone(), (setting.id, ty))
                .is_some()
            {
                checker.error(
                    format!("duplicate setting `{}`", setting.name),
                    setting.span,
                );
            }
        }
        if let SettingKind::Choice {
            enumeration,
            default_variant,
            options,
        } = &setting.kind
        {
            let declaration = checker
                .enums
                .iter()
                .find(|item| item.id == *enumeration)
                .cloned();
            let Some(declaration) = declaration else {
                checker.error("unknown enum used by choice setting", setting.span);
                continue;
            };
            let mut seen = HashSet::new();
            for option in options {
                let Some(variant) = declaration
                    .variants
                    .iter()
                    .find(|variant| variant.name == option.variant)
                else {
                    checker.error(
                        format!(
                            "enum `{}` has no variant `{}`",
                            declaration.name, option.variant
                        ),
                        option.span,
                    );
                    continue;
                };
                if variant.payload.is_some() {
                    checker.error("choice variants cannot have payloads", option.span);
                }
                checker
                    .semantics
                    .resolve_setting_choice_option(option.id, variant.id);
                if !seen.insert(option.variant.clone()) {
                    checker.error(
                        format!("duplicate choice option `{}`", option.variant),
                        option.span,
                    );
                }
            }
            if !seen.contains(default_variant) {
                checker.error(
                    "the default choice must be one of its options",
                    setting.span,
                );
            } else if let Some(variant) = declaration
                .variants
                .iter()
                .find(|variant| variant.name == *default_variant)
            {
                checker
                    .semantics
                    .resolve_setting_choice_default(setting.id, variant.id);
            }
        }
    }

    let mut record_names = HashSet::new();
    for record in &program.records {
        if !record_names.insert(record.name.clone()) {
            checker.error(format!("duplicate record `{}`", record.name), record.span);
        }
        let mut fields = HashSet::new();
        for field in &record.fields {
            let field_ty = checker.syntax_type(field.ty);
            checker
                .semantics
                .resolve_record_field_type(field.id, field_ty);
            if !fields.insert(field.name.clone()) {
                checker.error(
                    format!(
                        "duplicate field `{}` in record `{}`",
                        field.name, record.name
                    ),
                    field.span,
                );
            }
            if let Some(standard) = checker.standard_type_id(field_ty) {
                let declaration = StandardLibrary::new().type_decl(standard);
                if !declaration.value_usage.record_field {
                    checker.error(
                        format!("{} cannot be stored in a record field", declaration.name),
                        field.span,
                    );
                }
            }
        }
    }

    let mut enum_names = HashSet::new();
    let enum_declarations = checker.enums.clone();
    for enumeration in &enum_declarations {
        if !enum_names.insert(enumeration.name.clone()) || record_names.contains(&enumeration.name)
        {
            checker.error(
                format!("duplicate named type `{}`", enumeration.name),
                enumeration.span,
            );
        }
        let mut variants = HashSet::new();
        for variant in &enumeration.variants {
            let payload = variant.payload.map(|ty| checker.syntax_type(ty));
            checker
                .semantics
                .resolve_enum_variant_payload(variant.id, payload);
            if !variants.insert(variant.name.clone()) {
                checker.error(
                    format!(
                        "duplicate variant `{}` in enum `{}`",
                        variant.name, enumeration.name
                    ),
                    variant.span,
                );
            }
            if let Some(standard) = payload.and_then(|ty| checker.standard_type_id(ty))
                && !StandardLibrary::new()
                    .type_decl(standard)
                    .value_usage
                    .enum_payload
            {
                checker.error(
                    "enum payloads cannot store this standard-library type",
                    variant.span,
                );
            }
        }
        if enumeration.variants.is_empty() {
            checker.error("an enum needs at least one variant", enumeration.span);
        }
    }

    for function in &program.functions {
        let params = function
            .params
            .iter()
            .map(|parameter| {
                let ty = if let Some(annotation) = parameter.annotation {
                    checker.syntax_type(annotation)
                } else {
                    checker.fresh_inference(Requirements::NONE, None)
                };
                checker.semantics.resolve_value_type(parameter.id, ty);
                ty
            })
            .collect::<Vec<_>>();
        let result = if let Some(annotation) = function.return_annotation {
            checker.syntax_type(annotation)
        } else if contains_value_return(&function.body) {
            checker.fresh_inference(Requirements::NONE, None)
        } else {
            Type::Void
        };
        checker
            .semantics
            .resolve_function_result(function.id, result);
        let signature = FunctionSignature {
            id: function.id,
            params,
            result,
        };
        checker
            .function_signatures
            .insert(function.id, signature.clone());
        if let Some(receiver) = function.method_of {
            let key = (checker.syntax_type(receiver), function.name.clone());
            if checker.methods.insert(key, signature).is_some() {
                checker.error(
                    format!("duplicate method `{}` for `{receiver}`", function.name),
                    function.span,
                );
            }
            continue;
        }
        if function.name == "Err"
            || !StandardLibrary::new()
                .function_candidates(std::slice::from_ref(&function.name))
                .is_empty()
            || checker.functions.contains_key(&function.name)
        {
            checker.error(
                format!("duplicate or reserved function name `{}`", function.name),
                function.span,
            );
            continue;
        }
        checker.functions.insert(function.name.clone(), signature);
    }

    for global in &program.globals {
        if checker.globals.contains_key(&global.name) {
            checker.error(
                format!("duplicate global variable `{}`", global.name),
                global.span,
            );
            continue;
        }
        if !is_constant(&global.value) {
            checker.error(
                "global initializers must be numeric, boolean, or payload-free enum constants",
                global.value.span,
            );
        }
        let previous_debug_context = checker.debug_context;
        checker.debug_context = global.debug_only;
        let expected = global.annotation.map(|ty| checker.syntax_type(ty));
        let inferred = checker.expr(&global.value, expected);
        checker.debug_context = previous_debug_context;
        if let Some(ty) = inferred {
            let unsupported_standard = checker.standard_type_id(ty).is_some_and(|standard| {
                !StandardLibrary::new()
                    .type_decl(standard)
                    .value_usage
                    .global_variable
            });
            if unsupported_standard
                || matches!(
                    ty,
                    Type::Void | Type::Array(_) | Type::Option(_) | Type::Result(_)
                )
                || checker.source_record_id(ty).is_some()
            {
                let ty = checker.type_name(ty);
                checker.error(
                    format!("global variables cannot currently store `{ty}`"),
                    global.span,
                );
            }
            checker.semantics.resolve_value_type(global.id, ty);
            checker.globals.insert(
                global.name.clone(),
                Binding {
                    id: Some(global.id),
                    ty,
                    mutable: global.mutable,
                    debug_only: global.debug_only,
                },
            );
        }
    }

    checker.checking_state_source = true;
    for field in &program.state.as_ref().unwrap().fields {
        if let StateSource::Expression(expression) = &field.source {
            let field_type = checker.state_fields[&field.name].1;
            let boundary = contains_propagation(expression)
                .then(|| Type::Result(checker.inference.result_type(field_type)));
            checker.failure_boundary = boundary;
            checker.used_propagation = false;
            let actual = checker.expr(expression, None);
            let used_propagation = checker.used_propagation;
            checker.failure_boundary = None;
            let Some(actual) = actual else {
                continue;
            };
            let actual = checker.shallow_type(actual);
            let poll_result = if used_propagation {
                let boundary = boundary.expect("propagation syntax creates a failure boundary");
                if matches!(actual, Type::Result(_)) {
                    checker.error(
                        "a state expression using `?` must produce the field value, not another result",
                        expression.span,
                    );
                } else {
                    checker.unify(actual, field_type, expression.span);
                    checker.expect_expression(
                        expression.id,
                        actual,
                        Some(boundary),
                        expression.span,
                    );
                }
                boundary
            } else if let Type::Result(result) = actual {
                let value = checker.inference.result_value(result);
                checker.unify(value, field_type, expression.span);
                actual
            } else {
                checker.unify(actual, field_type, expression.span);
                let result = Type::Result(checker.inference.result_type(actual));
                checker.expect_expression(expression.id, actual, Some(result), expression.span);
                result
            };
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    }
    checker.checking_state_source = false;

    checker.in_function = true;
    for function in &program.functions {
        checker.debug_context = function.debug_only;
        let signature = checker.function_signatures[&function.id].clone();
        checker.return_ty = signature.result;
        checker.failure_boundary = match checker.shallow_type(signature.result) {
            Type::Result(_) => Some(checker.shallow_type(signature.result)),
            _ => None,
        };
        checker.current_callable = function.method_of.map_or_else(
            || format!("function `{}`", function.name),
            |receiver| format!("method `{receiver}.{}`", function.name),
        );
        checker.scopes.clear();
        checker.scopes.push(HashMap::new());
        for (parameter, ty) in function.params.iter().zip(signature.params.iter().copied()) {
            let duplicate = checker.scopes[0]
                .insert(
                    parameter.name.clone(),
                    Binding {
                        id: Some(parameter.id),
                        ty,
                        mutable: true,
                        debug_only: checker.debug_context,
                    },
                )
                .is_some();
            if duplicate {
                checker.error(
                    format!("duplicate parameter `{}`", parameter.name),
                    parameter.span,
                );
            }
        }
        checker.block(&function.body, false);
        if signature.result != Type::Void && !definitely_returns(&function.body) {
            checker.error(
                format!(
                    "function `{}` must return `{}` on every path",
                    function.name, signature.result
                ),
                function.body.span,
            );
        }
    }
    checker.in_function = false;
    checker.debug_context = false;
    checker.failure_boundary = None;

    let mut actions = HashSet::new();
    for action in &program.actions {
        if !actions.insert(action.kind) {
            checker.error(
                format!("duplicate `{}` action", action.kind.name()),
                action.span,
            );
            continue;
        }
        checker.return_ty = checker.syntax_type(action.kind.return_type());
        checker.current_action = action.kind;
        checker.current_callable = format!("`{}` action", action.kind.name());
        checker.scopes.clear();
        checker.scopes.push(HashMap::new());
        checker.block(&action.body, false);
    }

    checker.resolve_deferred_member_paths();
    checker.diagnose_ambiguous_process_reads();
    if checker.errors.is_empty() {
        checker.default_inference_variables();
    }
    if !checker.errors.is_empty() {
        checker.inference.recover_unbound();
    }
    for field in &program.state.as_ref().unwrap().fields {
        if matches!(field.source, StateSource::Pointer(_)) {
            let field_type = checker.state_fields[&field.name].1;
            let poll_result = Type::Result(checker.inference.result_type(field_type));
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    }
    checker.finalize_array_types();
    checker.inference.finalize_wrappers();
    let array_types = checker
        .inference
        .arrays()
        .iter()
        .map(|array| ArrayTypeDecl {
            id: array.id,
            element: array.element.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let option_types = checker
        .inference
        .options()
        .iter()
        .map(|option| OptionTypeDecl {
            id: option.id,
            value: option.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    let result_types = checker
        .inference
        .results()
        .iter()
        .map(|result| ResultTypeDecl {
            id: result.id,
            value: result.value.to_ref(checker.inference.type_store()),
        })
        .collect::<Vec<_>>();
    for array in &array_types {
        let element = checker.syntax_type(array.element);
        checker
            .semantics
            .resolve_array_element_type(array.id, element);
    }
    let semantics = std::mem::take(&mut checker.semantics);
    let semantic_types = checker.inference.type_store().clone();
    let enum_types = checker.enums.clone();
    let diagnostics = std::mem::take(&mut checker.errors);
    RecoveringCheckOutput {
        output: CheckOutput {
            semantics: semantics.finish(
                semantic_types,
                &array_types,
                &option_types,
                &result_types,
                |ty| checker.resolved_type(ty),
            ),
            enum_types,
            array_types,
            option_types,
            result_types,
        },
        diagnostics,
    }
}

#[derive(Clone)]
struct EnumVariantInfo {
    id: ResolvedEnumVariantId,
    name: String,
    payload: Option<Type>,
}

#[derive(Clone)]
struct EnumInfo {
    name: String,
    variants: Vec<EnumVariantInfo>,
}

struct Checker {
    errors: Vec<Diagnostic>,
    state_fields: HashMap<String, (ValueId, Type)>,
    settings: HashMap<String, (ValueId, Type)>,
    globals: HashMap<String, Binding>,
    functions: HashMap<String, FunctionSignature>,
    methods: HashMap<(Type, String), FunctionSignature>,
    function_signatures: HashMap<FunctionId, FunctionSignature>,
    debug_functions: HashSet<FunctionId>,
    records: Vec<RecordDecl>,
    enums: Vec<EnumDecl>,
    named_types: HashMap<TypeNameId, Type>,
    inference: InferenceContext,
    scopes: Vec<HashMap<String, Binding>>,
    return_ty: Type,
    current_action: ActionKind,
    current_callable: String,
    in_function: bool,
    checking_suspension: bool,
    debug_context: bool,
    loop_depth: usize,
    checking_state_source: bool,
    failure_boundary: Option<Type>,
    used_propagation: bool,
    inferred_process_reads: Vec<(Type, Span)>,
    deferred_member_paths: Vec<DeferredMemberPath>,
    allowing_null: bool,
    semantics: SemanticBuilder,
}

impl Checker {
    fn syntax_type(&self, ty: TypeRef) -> Type {
        syntax_type(ty, &self.named_types, self.inference.type_store())
    }

    fn standard_type(&self, standard: StdlibTypeId) -> Type {
        self.inference.known_standard(standard)
    }

    fn standard_type_id(&self, ty: Type) -> Option<StdlibTypeId> {
        self.inference.standard_type(ty)
    }

    fn record_type(&self, record: crate::ast::RecordId) -> Type {
        Type::Known(self.inference.type_store().id_for_record(record))
    }

    fn source_record_id(&self, ty: Type) -> Option<crate::ast::RecordId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.inference.type_store().kind(id) {
            TypeKind::Record(record) => Some(*record),
            _ => None,
        }
    }

    fn source_enum_id(&self, ty: Type) -> Option<crate::ast::EnumId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.inference.type_store().kind(id) {
            TypeKind::Enum(enumeration) => Some(*enumeration),
            _ => None,
        }
    }

    fn declared_type(&self, ty: DeclaredTypeRef) -> Type {
        inference_type_for_declared(self.inference.type_store(), ty)
    }

    fn block(&mut self, block: &Block, nested: bool) {
        if nested {
            self.scopes.push(HashMap::new());
        }
        for statement in &block.statements {
            self.statement(statement);
        }
        if nested {
            self.scopes.pop();
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Debug {
                statement: inner,
                span,
            } => {
                if !matches!(
                    inner.as_ref(),
                    Stmt::Variable(_)
                        | Stmt::Assign { .. }
                        | Stmt::If { .. }
                        | Stmt::While { .. }
                        | Stmt::Expression(_)
                        | Stmt::Suspend { .. }
                ) {
                    self.error(
                        "`debug` currently supports bindings, expression statements, assignments, `if`, `while`, and `await`/`retry` statements",
                        *span,
                    );
                }
                let previous_debug_context = self.debug_context;
                self.debug_context = true;
                self.statement(inner);
                self.debug_context = previous_debug_context;
            }
            Stmt::Variable(variable) => self.variable(variable),
            Stmt::Assign {
                id,
                name,
                op,
                value,
                span,
            } => {
                let binding = self.binding_for_use(name, *span);
                match binding {
                    Some(binding) if !binding.mutable => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                        }
                        self.error(format!("cannot assign to constant `{name}`"), *span)
                    }
                    Some(binding) => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                        }
                        if self.expr(value, Some(binding.ty)).is_some()
                            && let Some(op) = op
                        {
                            self.require_binary_operand(*op, binding.ty, *span);
                        }
                    }
                    None => self.error(format!("unknown variable `{name}`"), *span),
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.expr(condition, Some(Type::Bool));
                self.block(then_block, true);
                if let Some(else_block) = else_block {
                    self.block(else_block, true);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.expr(condition, Some(Type::Bool));
                self.loop_depth += 1;
                self.block(body, true);
                self.loop_depth -= 1;
            }
            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    self.error("`break` is only available inside a loop", *span);
                }
            }
            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    self.error("`continue` is only available inside a loop", *span);
                }
            }
            Stmt::Return { value, span } => self.check_return(value.as_ref(), *span),
            Stmt::Throw { error, span } => {
                if self.failure_boundary.is_none() {
                    self.error(
                        "`throw` needs a function returning `T!` or an explicit catch boundary",
                        *span,
                    );
                }
                self.expr(error, Some(self.standard_type(StdlibTypeId::String)));
            }
            Stmt::Suspend {
                mode,
                binding,
                value,
                span,
            } => {
                if self.in_function || self.current_action != ActionKind::OnAttach {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is only available inside `onAttach`"),
                        *span,
                    );
                }
                self.checking_suspension = true;
                let result = self.expr(value, None);
                self.checking_suspension = false;
                let result = result.and_then(|result| {
                    let result = match mode {
                        SuspensionMode::Await => {
                            let supported = self
                                .semantics
                                .standard_library_item(value.id)
                                .map(|item| StandardLibrary::new().item(item))
                                .is_some_and(|item| {
                                    item.operation_semantics().suspension.is_awaitable()
                                });
                            if !supported {
                                self.error("this operation is not awaitable", value.span);
                                return None;
                            }
                            match self.shallow_type(result) {
                                Type::Result(result) => self.inference.result_value(result),
                                result => result,
                            }
                        }
                        SuspensionMode::Retry => match self.shallow_type(result) {
                            Type::Result(result) => self.inference.result_value(result),
                            _ => {
                                self.error(
                                    "`retry` expects an expression of type `T!`",
                                    value.span,
                                );
                                return None;
                            }
                        },
                    };
                    let expected = binding
                        .as_ref()
                        .and_then(|binding| binding.annotation)
                        .map(|ty| self.syntax_type(ty));
                    expected.map_or(Some(result), |expected| {
                        self.unify(result, expected, value.span)
                    })
                });
                if let (Some(binding), Some(ty)) = (binding, result) {
                    let duplicate = self
                        .scopes
                        .iter()
                        .rev()
                        .any(|scope| scope.contains_key(&binding.name))
                        || self.globals.contains_key(&binding.name);
                    if duplicate {
                        self.error(
                            format!("variable `{}` is already declared", binding.name),
                            binding.span,
                        );
                    }
                    if ty == Type::Void {
                        self.error("a suspended binding needs a value", binding.span);
                    } else {
                        self.semantics.resolve_value_type(binding.id, ty);
                        self.scopes.last_mut().unwrap().insert(
                            binding.name.clone(),
                            Binding {
                                id: Some(binding.id),
                                ty,
                                mutable: true,
                                debug_only: self.debug_context,
                            },
                        );
                    }
                }
            }
            Stmt::Expression(expr) => {
                self.expr(expr, None);
            }
        }
    }

    fn check_return(&mut self, value: Option<&Expr>, span: Span) {
        match (self.return_ty, value) {
            (Type::Void, None) => {}
            (Type::Void, Some(value)) => {
                self.expr(value, None);
                self.error(
                    format!("{} cannot return a value", self.current_callable),
                    span,
                );
            }
            (expected, Some(value)) => {
                self.allowing_null = !self.in_function
                    && matches!(
                        self.current_action,
                        ActionKind::IsLoading | ActionKind::GameTime
                    );
                self.expr(value, Some(expected));
                self.allowing_null = false;
            }
            (_, None)
                if !self.in_function
                    && matches!(
                        self.current_action,
                        ActionKind::Start
                            | ActionKind::Split
                            | ActionKind::Reset
                            | ActionKind::IsLoading
                            | ActionKind::GameTime
                    ) => {}
            (expected, None) => {
                let expected = self.type_name(expected);
                self.error(
                    format!("expected a return value of type `{expected}`"),
                    span,
                );
            }
        }
    }

    fn variable(&mut self, variable: &VariableDecl) {
        let duplicate = self
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(&variable.name));
        if duplicate || self.globals.contains_key(&variable.name) {
            self.error(
                format!("variable `{}` is already declared", variable.name),
                variable.span,
            );
        }
        let expected = variable.annotation.map(|ty| self.syntax_type(ty));
        if let Some(ty) = self.expr(&variable.value, expected) {
            let unsupported_standard = self.standard_type_id(ty).is_some_and(|standard| {
                !StandardLibrary::new()
                    .type_decl(standard)
                    .value_usage
                    .local_variable
            });
            if ty == Type::Void || unsupported_standard {
                let ty = self.type_name(ty);
                self.error(
                    format!("local variables cannot currently store `{ty}`"),
                    variable.span,
                );
                return;
            }
            self.semantics.resolve_value_type(variable.id, ty);
            self.scopes.last_mut().unwrap().insert(
                variable.name.clone(),
                Binding {
                    id: Some(variable.id),
                    ty,
                    mutable: variable.mutable,
                    debug_only: self.debug_context || variable.debug_only,
                },
            );
        }
    }

    fn expr(&mut self, expr: &Expr, expected: Option<Type>) -> Option<Type> {
        let ty = match &expr.kind {
            ExprKind::Error => {
                self.error("cannot type-check a recovered expression", expr.span);
                return None;
            }
            ExprKind::None => match expected.map(|ty| self.shallow_type(ty)) {
                Some(expected @ Type::Option(_)) => expected,
                Some(expected) if self.allowing_null => expected,
                Some(_) => {
                    self.error("`None` can only construct an optional value", expr.span);
                    return None;
                }
                None => {
                    self.error(
                        "cannot infer the value type of `None`; add a `T?` annotation",
                        expr.span,
                    );
                    return None;
                }
            },
            ExprKind::Bool(_) => {
                self.expect_expression(expr.id, Type::Bool, expected, expr.span)?
            }
            ExprKind::Int { value, suffix } => {
                let ty = if let Some(suffix) = suffix {
                    let suffix = Type::from(*suffix);
                    if !fits_unsigned_literal(*value, suffix) {
                        self.error(
                            format!("integer literal does not fit in `{suffix}`"),
                            expr.span,
                        );
                        return None;
                    }
                    suffix
                } else {
                    self.fresh_inference(REQUIRE_INTEGER, Some(*value))
                };
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Float(_) => {
                let ty = self.fresh_inference(REQUIRE_FLOAT, None);
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::String(_) => self.expect_expression(
                expr.id,
                self.standard_type(StdlibTypeId::String),
                expected,
                expr.span,
            )?,
            ExprKind::InterpolatedString(parts) => {
                self.array_type_id(self.standard_type(StdlibTypeId::String));
                for part in parts {
                    if let InterpolatedPart::Expr(value) = part {
                        let value_type = self.expr(value, None)?;
                        self.require(value_type, REQUIRE_INTERPOLATABLE, value.span);
                    }
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::String),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Signature(signature) => {
                if let Err(message) = parse_signature(signature) {
                    self.error(message, expr.span);
                }
                self.expect_expression(
                    expr.id,
                    self.standard_type(StdlibTypeId::Signature),
                    expected,
                    expr.span,
                )?
            }
            ExprKind::Array(elements) => {
                let value_expected = expected.map(|ty| self.expected_value_type(ty));
                let hinted = value_expected.and_then(|ty| match ty {
                    Type::Array(id) => self
                        .inference
                        .arrays()
                        .iter()
                        .find(|array| array.id == id)
                        .map(|array| (id, array.element)),
                    _ => None,
                });
                let (id, element_type) = if let Some((id, element)) = hinted {
                    (id, element)
                } else if !elements.is_empty() {
                    let element = self.fresh_inference(Requirements::NONE, None);
                    let id = self.array_type_id(element);
                    (id, element)
                } else {
                    self.error(
                        "an empty array needs an `Array<T>` type annotation",
                        expr.span,
                    );
                    return None;
                };
                for element in elements {
                    self.expr(element, Some(element_type));
                }
                self.expect_expression(expr.id, Type::Array(id), expected, expr.span)?
            }
            ExprKind::Record { record, fields } => {
                let Some(declaration) = self
                    .records
                    .iter()
                    .find(|declaration| declaration.id == *record)
                    .cloned()
                else {
                    self.error("unknown record type", expr.span);
                    return None;
                };
                let mut seen = HashSet::new();
                let mut resolved_fields = Vec::with_capacity(fields.len());
                for (name, value) in fields {
                    if !seen.insert(name.clone()) {
                        self.error(format!("duplicate record field `{name}`"), value.span);
                        continue;
                    }
                    if let Some(field) = declaration.fields.iter().find(|field| field.name == *name)
                    {
                        self.expr(value, Some(self.syntax_type(field.ty)));
                        resolved_fields.push(field.id);
                    } else {
                        self.expr(value, None);
                        self.error(
                            format!("record `{}` has no field `{name}`", declaration.name),
                            value.span,
                        );
                    }
                }
                self.semantics
                    .resolve_record_literal_fields(expr.id, resolved_fields);
                for field in &declaration.fields {
                    if !seen.contains(&field.name) {
                        self.error(
                            format!(
                                "record `{}` initializer is missing field `{}`",
                                declaration.name, field.name
                            ),
                            expr.span,
                        );
                    }
                }
                self.expect_expression(expr.id, self.record_type(*record), expected, expr.span)?
            }
            ExprKind::Enum {
                enumeration,
                variant,
                payload,
            } => {
                let Some(declaration) = self.enum_info(*enumeration) else {
                    self.error("unknown enum type", expr.span);
                    return None;
                };
                let Some(declared_variant) = declaration
                    .variants
                    .iter()
                    .find(|declared| declared.name == *variant)
                else {
                    self.error(
                        format!("enum `{}` has no variant `{variant}`", declaration.name),
                        expr.span,
                    );
                    return None;
                };
                self.semantics
                    .resolve_enum_variant(expr.id, declared_variant.id);
                match (declared_variant.payload, payload) {
                    (Some(payload_type), Some(payload)) => {
                        self.expr(payload, Some(payload_type));
                    }
                    (Some(_), None) => {
                        self.error(format!("variant `{variant}` requires a payload"), expr.span)
                    }
                    (None, Some(payload)) => {
                        self.expr(payload, None);
                        self.error(
                            format!("variant `{variant}` does not accept a payload"),
                            expr.span,
                        );
                    }
                    (None, None) => {}
                }
                self.expect_expression(expr.id, self.enum_type(*enumeration), expected, expr.span)?
            }
            ExprKind::Match { value, arms } => {
                let value_type = self.expr(value, None)?;
                let mut unguarded_patterns = HashSet::new();
                let mut has_unguarded_wildcard = false;
                let mut result_type = expected;
                for arm in arms {
                    if has_unguarded_wildcard {
                        self.error("unreachable match arm after `_`", arm.span);
                    }
                    self.scopes.push(HashMap::new());
                    let pattern_key = match &arm.pattern {
                        MatchPattern::Enum {
                            enumeration,
                            variant,
                            binding,
                        } => {
                            self.unify(value_type, self.enum_type(*enumeration), arm.span);
                            let declaration = self.enum_info(*enumeration);
                            if let Some(declaration) = declaration {
                                if let Some(declared_variant) = declaration
                                    .variants
                                    .iter()
                                    .find(|declared| declared.name == *variant)
                                {
                                    self.semantics.resolve_pattern_variant(
                                        arm.pattern_id,
                                        declared_variant.id,
                                    );
                                    match (declared_variant.payload, binding) {
                                        (Some(payload_type), Some(binding)) => {
                                            self.semantics
                                                .resolve_value_type(binding.id, payload_type);
                                            self.scopes.last_mut().unwrap().insert(
                                                binding.name.clone(),
                                                Binding {
                                                    id: Some(binding.id),
                                                    ty: payload_type,
                                                    mutable: false,
                                                    debug_only: self.debug_context,
                                                },
                                            );
                                        }
                                        (None, Some(_)) => self.error(
                                            format!("variant `{variant}` has no payload to bind"),
                                            arm.span,
                                        ),
                                        _ => {}
                                    }
                                } else {
                                    self.error(
                                        format!(
                                            "enum `{}` has no variant `{variant}`",
                                            declaration.name
                                        ),
                                        arm.span,
                                    );
                                }
                            } else {
                                self.error("unknown enum type", arm.span);
                            }
                            format!("enum:{enumeration}:{variant}")
                        }
                        MatchPattern::Bool(value) => {
                            self.unify(value_type, Type::Bool, arm.span);
                            format!("bool:{value}")
                        }
                        MatchPattern::Int { value, suffix } => {
                            let pattern_type = if let Some(suffix) = suffix {
                                if !suffix.is_integer() {
                                    self.error(
                                        "integer match patterns require an integer type",
                                        arm.span,
                                    );
                                } else if !fits_unsigned_literal(*value, (*suffix).into()) {
                                    self.error(
                                        format!("integer literal does not fit in `{suffix}`"),
                                        arm.span,
                                    );
                                }
                                (*suffix).into()
                            } else {
                                self.fresh_inference(REQUIRE_INTEGER, Some(*value))
                            };
                            self.unify(value_type, pattern_type, arm.span);
                            format!("int:{value}")
                        }
                        MatchPattern::None => {
                            match self.shallow_type(value_type) {
                                Type::Option(option) => {
                                    self.semantics.resolve_wrapper_pattern(
                                        arm.pattern_id,
                                        ResolvedWrapperPattern::OptionNone(option),
                                    );
                                    format!("option:{option}:none")
                                }
                                ty => {
                                    let ty = self.type_name(ty);
                                    self.error(
                                    format!("a `None` pattern requires an optional value, found `{ty}`"),
                                    arm.span,
                                );
                                    format!("invalid:{}", arm.pattern_id.index())
                                }
                            }
                        }
                        MatchPattern::OptionSome(binding) => match self.shallow_type(value_type) {
                            Type::Option(option) => {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::OptionSome(option),
                                );
                                let binding_type = self.inference.option_value(option);
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(binding, binding_type);
                                }
                                format!("option:{option}:some")
                            }
                            ty => {
                                let ty = self.type_name(ty);
                                self.error(
                                    format!(
                                        "a `Some(value)` pattern requires an optional value, found `{ty}`"
                                    ),
                                    arm.span,
                                );
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        },
                        MatchPattern::ResultSuccess(binding) => match self.shallow_type(value_type)
                        {
                            Type::Result(result) => {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::ResultSuccess(result),
                                );
                                let binding_type = self.inference.result_value(result);
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(binding, binding_type);
                                }
                                format!("result:{result}:success")
                            }
                            ty => {
                                self.error(
                                    format!(
                                        "an `Ok(value)` pattern requires a result value, found `{ty}`"
                                    ),
                                    arm.span,
                                );
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        },
                        MatchPattern::ResultError(binding) => match self.shallow_type(value_type) {
                            Type::Result(result) => {
                                self.semantics.resolve_wrapper_pattern(
                                    arm.pattern_id,
                                    ResolvedWrapperPattern::ResultError(result),
                                );
                                if let Some(binding) = binding {
                                    self.bind_pattern_value(
                                        binding,
                                        self.standard_type(StdlibTypeId::String),
                                    );
                                }
                                format!("result:{result}:error")
                            }
                            ty => {
                                self.error(
                                        format!(
                                            "an `Err(error)` pattern requires a result value, found `{ty}`"
                                        ),
                                        arm.span,
                                    );
                                format!("invalid:{}", arm.pattern_id.index())
                            }
                        },
                        MatchPattern::Wildcard => "wildcard".to_owned(),
                    };
                    if let Some(guard) = &arm.guard {
                        self.expr(guard, Some(Type::Bool));
                    }
                    let arm_type = self.expr(&arm.value, result_type);
                    self.scopes.pop();
                    if result_type.is_none() {
                        result_type = arm_type;
                    }

                    if arm.guard.is_none() {
                        if !unguarded_patterns.insert(pattern_key.clone()) {
                            self.error(format!("duplicate match arm `{pattern_key}`"), arm.span);
                        }
                        if matches!(arm.pattern, MatchPattern::Wildcard) {
                            has_unguarded_wildcard = true;
                        }
                    } else if unguarded_patterns.contains(&pattern_key) {
                        self.error("unreachable guarded match arm", arm.span);
                    }
                }

                if !has_unguarded_wildcard {
                    match self.shallow_type(value_type) {
                        ty @ Type::Known(_) => {
                            if let Some((enum_key, declaration)) = self.enum_info_for_type(ty) {
                                for variant in &declaration.variants {
                                    let key = format!("enum:{enum_key}:{}", variant.name);
                                    if !unguarded_patterns.contains(&key) {
                                        self.error(
                                            format!(
                                                "non-exhaustive match: missing `{}`",
                                                variant.name
                                            ),
                                            expr.span,
                                        );
                                    }
                                }
                            }
                        }
                        Type::Bool => {
                            for value in [false, true] {
                                if !unguarded_patterns.contains(&format!("bool:{value}")) {
                                    self.error(
                                        format!("non-exhaustive match: missing `{value}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Option(option) => {
                            for (state, display) in [("none", "None"), ("some", "Some(value)")] {
                                if !unguarded_patterns.contains(&format!("option:{option}:{state}"))
                                {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        Type::Result(result) => {
                            for (state, display) in
                                [("success", "Ok(value)"), ("error", "Err(error)")]
                            {
                                if !unguarded_patterns.contains(&format!("result:{result}:{state}"))
                                {
                                    self.error(
                                        format!("non-exhaustive match: missing `{display}`"),
                                        expr.span,
                                    );
                                }
                            }
                        }
                        ty if ty.is_integer() => {
                            self.error("non-exhaustive integer match: add a `_` arm", expr.span)
                        }
                        Type::Variable(_) => self.error(
                            "match patterns do not determine the matched value's type",
                            value.span,
                        ),
                        ty => {
                            let ty = self.type_name(ty);
                            self.error(format!("type `{ty}` cannot be matched"), value.span);
                        }
                    }
                }
                let Some(result_type) = result_type else {
                    self.error("a match needs at least one arm", expr.span);
                    return None;
                };
                self.expect_expression(expr.id, result_type, expected, expr.span)?
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition, Some(Type::Bool));
                let result_type =
                    expected.unwrap_or_else(|| self.fresh_inference(Requirements::NONE, None));
                self.expr(then_expr, Some(result_type));
                self.expr(else_expr, Some(result_type));
                self.expect_expression(expr.id, result_type, expected, expr.span)?
            }
            ExprKind::Fallback { value, fallback } => {
                let wrapper = self.expr(value, None)?;
                let value_type = match self.shallow_type(wrapper) {
                    Type::Option(option) => self.inference.option_value(option),
                    Type::Result(result) => self.inference.result_value(result),
                    ty => {
                        let ty = self.type_name(ty);
                        self.error(
                            format!("`else` can only unwrap `T?` or `T!`, found `{ty}`"),
                            value.span,
                        );
                        return None;
                    }
                };
                match fallback {
                    FallbackBranch::Value(fallback) => {
                        self.expr(fallback, Some(value_type));
                    }
                    FallbackBranch::Return { value, span } => {
                        self.check_return(value.as_deref(), *span);
                    }
                    FallbackBranch::Break { span } => {
                        if self.loop_depth == 0 {
                            self.error("`else break` is only available inside a loop", *span);
                        }
                    }
                    FallbackBranch::Continue { span } => {
                        if self.loop_depth == 0 {
                            self.error("`else continue` is only available inside a loop", *span);
                        }
                    }
                }
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Propagate(value) => {
                let input = self.expr(value, None)?;
                let Type::Result(input_result) = self.shallow_type(input) else {
                    self.error("`?` requires a result value (`T!`)", value.span);
                    return None;
                };
                let Some(boundary) = self.failure_boundary else {
                    self.error(
                        "`?` needs a state-field boundary or a function returning `T!`",
                        expr.span,
                    );
                    return None;
                };
                let Type::Result(_) = self.shallow_type(boundary) else {
                    unreachable!("failure boundaries are result types")
                };
                self.used_propagation = true;
                self.semantics.resolve_propagation_target(expr.id, boundary);
                let value_type = self.inference.result_value(input_result);
                self.expect_expression(expr.id, value_type, expected, expr.span)?
            }
            ExprKind::Path(path) => {
                let resolution = self.path(path, expr.span, Some(expr.id))?;
                if let Some(value) = resolution.value {
                    self.semantics.resolve_value(expr.id, value);
                }
                if let Some(members) = resolution.members {
                    self.semantics.resolve_path_members(expr.id, members);
                }
                self.expect_expression(expr.id, resolution.ty, expected, expr.span)?
            }
            ExprKind::Member {
                receiver,
                name,
                name_span,
            } => {
                let receiver_ty = self.expr(receiver, None)?;
                let (ty, members) = self.resolve_members_or_defer(
                    receiver_ty,
                    std::slice::from_ref(name),
                    *name_span,
                    Some(expr.id),
                )?;
                if let Some(members) = members {
                    self.semantics.resolve_path_members(expr.id, members);
                }
                self.expect_expression(expr.id, ty, expected, expr.span)?
            }
            ExprKind::Unary { op, expr: inner } => match op {
                UnaryOp::Not => {
                    self.expr(inner, Some(Type::Bool));
                    self.expect_expression(expr.id, Type::Bool, expected, expr.span)?
                }
                UnaryOp::Neg => {
                    let operand_hint = expected.map(|ty| self.expected_value_type(ty));
                    let inner_ty = self.expr(inner, operand_hint)?;
                    self.require(inner_ty, REQUIRE_NUMERIC | REQUIRE_SIGNED, expr.span)?;
                    self.expect_expression(expr.id, inner_ty, expected, expr.span)?
                }
            },
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let source = self.expr(inner, None)?;
                let target = self.syntax_type(*target);
                if target.is_numeric() {
                    self.require(source, REQUIRE_NUMERIC, expr.span)?;
                } else if target == self.standard_type(StdlibTypeId::String) {
                    self.require(source, REQUIRE_STRING_CAST, expr.span)?;
                } else {
                    self.error(
                        format!("`as` cannot convert a value to `{target}`"),
                        expr.span,
                    );
                    return None;
                }
                self.expect_expression(expr.id, target, expected, expr.span)?
            }
            ExprKind::Binary { op, left, right } => {
                self.binary(*op, left, right, expected, expr.id, expr.span)?
            }
            ExprKind::Call {
                callee,
                name_span,
                args,
            } => self.call(callee, *name_span, args, expected, expr.id, expr.span)?,
        };
        self.semantics.resolve_expression_type(expr.id, ty);
        Some(ty)
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if matches!(op, BinaryOp::Or | BinaryOp::And) {
            self.expr(left, Some(Type::Bool));
            self.expr(right, Some(Type::Bool));
            return self.expect_expression(expression, Type::Bool, expected, span);
        }

        let result_is_bool = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        let operand_hint = if result_is_bool {
            None
        } else {
            expected.map(|ty| self.expected_value_type(ty))
        };
        let left_ty = self.expr(left, operand_hint)?;
        let right_ty = self.expr(right, operand_hint)?;
        let operand_ty = self.unify(left_ty, right_ty, span)?;

        self.require_binary_operand(op, operand_ty, span)?;

        let result = if result_is_bool {
            Type::Bool
        } else {
            operand_ty
        };
        self.expect_expression(expression, result, expected, span)
    }

    fn require_binary_operand(&mut self, op: BinaryOp, operand_ty: Type, span: Span) -> Option<()> {
        match op {
            BinaryOp::Eq | BinaryOp::Ne => self.require(operand_ty, REQUIRE_EQUATABLE, span),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                self.require(operand_ty, REQUIRE_NUMERIC, span)
            }
            BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::Shl
            | BinaryOp::Shr => self.require(operand_ty, REQUIRE_INTEGER, span),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => self
                .require(
                    operand_ty,
                    if op == BinaryOp::Rem {
                        REQUIRE_INTEGER
                    } else {
                        REQUIRE_NUMERIC
                    },
                    span,
                ),
            BinaryOp::Or | BinaryOp::And => {
                unreachable!("logical operators are checked separately")
            }
        }
    }

    fn call(
        &mut self,
        callee: &[String],
        name_span: Span,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if callee == ["Some"] {
            if args.len() != 1 {
                self.error("`Some` expects one value", span);
                return None;
            }
            let expected_option = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Option(option) => Some(option),
                _ => None,
            });
            if expected.is_some()
                && expected_option.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Some` constructs an optional value, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_option.map(|option| self.inference.option_value(option));
            let value = self.expr(&args[0], value_hint)?;
            let option = expected_option.unwrap_or_else(|| self.inference.option_type(value));
            let ty = Type::Option(option);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::OptionSome { option });
            return self.expect_expression(expression, ty, expected, span);
        }

        if callee == ["Ok"] {
            if args.len() != 1 {
                self.error("`Ok` expects one value", span);
                return None;
            }
            let expected_result = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Result(result) => Some(result),
                _ => None,
            });
            if expected.is_some()
                && expected_result.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Ok` constructs a result, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_result.map(|result| self.inference.result_value(result));
            let value = self.expr(&args[0], value_hint)?;
            let result = expected_result.unwrap_or_else(|| self.inference.result_type(value));
            let ty = Type::Result(result);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::ResultSuccess { result });
            return self.expect_expression(expression, ty, expected, span);
        }

        if callee == ["Err"] {
            if args.len() != 1 {
                self.error("`Err` expects one error message", span);
                return None;
            }
            self.expr(&args[0], Some(self.standard_type(StdlibTypeId::String)));
            let result = match expected.map(|ty| self.shallow_type(ty)) {
                Some(result @ Type::Result(_)) => result,
                Some(Type::Variable(_)) | None => {
                    self.error(
                        "cannot infer the success type of `Err`; add a `T!` annotation",
                        span,
                    );
                    return None;
                }
                Some(other) => {
                    self.error(
                        format!("`Err` constructs a result, but `{other}` was expected"),
                        span,
                    );
                    return None;
                }
            };
            let Type::Result(result_id) = result else {
                unreachable!()
            };
            self.semantics.resolve_call(
                expression,
                PendingResolvedCall::ResultError { result: result_id },
            );
            return Some(result);
        }

        let standard_library = StandardLibrary::new();
        let mut function_candidates = standard_library.function_candidates(callee);
        if function_candidates.len() > 1 {
            self.ambiguous_catalog_call(callee, &function_candidates, span);
            return None;
        }
        if let Some(candidate) = function_candidates.pop() {
            return self.catalog_call(&candidate, None, args, expected, expression, span);
        }
        let (display_name, signature, parameters, resolved_call) = if let [name] = callee {
            let Some(signature) = self.functions.get(name).cloned() else {
                let suggestion = self.function_name_suggestion(callee);
                self.unknown_function(callee, name_span, span, suggestion.as_deref());
                return None;
            };
            (
                name.clone(),
                signature.clone(),
                signature.params,
                PendingResolvedCall::UserFunction {
                    function: signature.id,
                },
            )
        } else {
            if let Some(suggestion) = self.function_name_suggestion(callee) {
                self.unknown_function(callee, name_span, span, Some(&suggestion));
                return None;
            }
            let (method, receiver_path) = callee.split_last().unwrap();
            let receiver = self.path(receiver_path, span, None)?;
            let receiver_value = receiver
                .value
                .expect("method receiver paths resolve to a declaration or snapshot value");
            let receiver_members = receiver
                .members
                .expect("method receiver types must be known while resolving a call");
            let receiver_type = self.shallow_type(receiver.ty);
            let mut candidates = standard_library
                .method_candidates(method)
                .into_iter()
                .filter(|candidate| self.catalog_candidate_may_apply(candidate, receiver_type))
                .collect::<Vec<_>>();
            if candidates.len() > 1 {
                self.ambiguous_catalog_call(callee, &candidates, span);
                return None;
            }
            if let Some(candidate) = candidates.pop() {
                return self.catalog_call(
                    &candidate,
                    Some(MethodReceiver {
                        ty: receiver_type,
                        value: receiver_value,
                        members: receiver_members,
                    }),
                    args,
                    expected,
                    expression,
                    span,
                );
            }
            let Some(signature) = self.methods.get(&(receiver_type, method.clone())).cloned()
            else {
                let suggestion = self.method_name_suggestion(receiver_type, method);
                self.unknown_method(
                    receiver_type,
                    method,
                    name_span,
                    span,
                    suggestion.as_deref(),
                );
                return None;
            };
            let receiver_name = self.type_name(receiver_type);
            (
                format!("{receiver_name}.{method}"),
                signature.clone(),
                signature.params.into_iter().skip(1).collect(),
                PendingResolvedCall::UserMethod {
                    function: signature.id,
                    receiver: receiver_value,
                    receiver_type,
                    receiver_members,
                },
            )
        };
        if self.debug_functions.contains(&signature.id) && !self.debug_context {
            self.error(
                format!("debug-only function `{display_name}` can only be called from debug code"),
                span,
            );
        }
        if args.len() != parameters.len() {
            self.error(
                format!(
                    "`{display_name}` expects {} arguments, found {}",
                    parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in args.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        let result = self.expect_expression(expression, signature.result, expected, span)?;
        self.semantics.resolve_call(expression, resolved_call);
        Some(result)
    }

    fn function_name_suggestion(&self, callee: &[String]) -> Option<String> {
        let (name, prefix) = callee.split_last()?;
        let standard_library = StandardLibrary::new();
        let mut candidates = standard_library
            .items()
            .iter()
            .filter_map(|item| {
                let path = standard_library.item_path(item)?;
                (path.len() == callee.len()
                    && path[..path.len() - 1]
                        .iter()
                        .copied()
                        .eq(prefix.iter().map(String::as_str)))
                .then_some(item.name)
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if prefix.is_empty() {
            candidates.extend(self.functions.keys().cloned());
            candidates.extend(["Some".to_owned(), "Ok".to_owned(), "Err".to_owned()]);
        }
        closest_name(name, candidates.iter().map(String::as_str))
    }

    fn method_name_suggestion(&mut self, receiver: Type, method: &str) -> Option<String> {
        let standard_library = StandardLibrary::new();
        let mut candidates = Vec::new();
        for item in standard_library.items() {
            let ItemKind::Method { .. } = item.kind else {
                continue;
            };
            let candidate = CallCandidate {
                item,
                type_arguments: Vec::new(),
            };
            if self.catalog_candidate_may_apply(&candidate, receiver) {
                candidates.push(item.name.to_owned());
            }
        }
        candidates.extend(
            self.methods
                .keys()
                .filter(|(candidate_receiver, _)| *candidate_receiver == receiver)
                .map(|(_, name)| name.clone()),
        );
        closest_name(method, candidates.iter().map(String::as_str))
    }

    fn unknown_function(
        &mut self,
        callee: &[String],
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        let name = callee.join(".");
        let Some(suggestion) = suggestion else {
            self.error(format!("unknown function `{name}`"), span);
            return;
        };
        let suggested_name = callee[..callee.len() - 1]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(suggestion))
            .collect::<Vec<_>>()
            .join(".");
        self.errors.push(
            Diagnostic::type_error(
                format!("unknown function `{name}`; did you mean `{suggested_name}`?"),
                name_span,
            )
            .with_primary_label("this name is not defined")
            .with_machine_applicable_fix(
                format!("replace `{}` with `{suggestion}`", callee.last().unwrap()),
                name_span,
                suggestion,
            ),
        );
    }

    fn unknown_method(
        &mut self,
        receiver: Type,
        method: &str,
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        let receiver = self.type_name(receiver);
        let Some(suggestion) = suggestion else {
            self.error(format!("type `{receiver}` has no method `{method}`"), span);
            return;
        };
        self.errors.push(
            Diagnostic::type_error(
                format!("type `{receiver}` has no method `{method}`; did you mean `{suggestion}`?"),
                name_span,
            )
            .with_primary_label("this method is not defined for the receiver type")
            .with_machine_applicable_fix(
                format!("replace `{method}` with `{suggestion}`"),
                name_span,
                suggestion,
            ),
        );
    }

    fn catalog_call(
        &mut self,
        candidate: &CallCandidate,
        receiver: Option<MethodReceiver>,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let item = candidate.item;
        let mut variables = HashMap::new();
        for parameter in item.signature.type_parameters {
            let requirements = parameter.constraints.iter().fold(
                Requirements::NONE,
                |requirements, constraint| {
                    requirements
                        | match constraint {
                            TypeConstraint::Numeric => REQUIRE_NUMERIC,
                            TypeConstraint::MemoryReadable => REQUIRE_MEMORY_READABLE,
                        }
                },
            );
            let ty = candidate
                .type_arguments
                .iter()
                .find(|(name, _)| *name == parameter.name)
                .map(|(_, ty)| ty.legacy())
                .unwrap_or_else(|| self.fresh_inference(requirements, None));
            if requirements != Requirements::NONE {
                self.require(ty, requirements, span)?;
            }
            variables.insert(parameter.name, ty);
        }
        if item.id == StdlibItemId::ProcessRead && candidate.type_arguments.is_empty() {
            self.inferred_process_reads.push((variables["T"], span));
        }
        if let Some(receiver) = &receiver {
            let declared_receiver = self.catalog_type(
                candidate
                    .receiver()
                    .expect("method candidates declare a receiver"),
                &variables,
            );
            self.unify(receiver.ty, declared_receiver, span)?;
        }
        let expected_result = expected.map(|ty| self.shallow_type(ty));
        let result_type = match (item.signature.result, expected_result) {
            (CatalogTypeRef::Result(value), Some(Type::Result(result))) => {
                let declared_value = self.catalog_type(*value, &variables);
                let expected_value = self.inference.result_value(result);
                self.unify(declared_value, expected_value, span)?;
                Type::Result(result)
            }
            _ => self.catalog_type(item.signature.result, &variables),
        };
        let result = self.expect_expression(expression, result_type, expected, span)?;
        if args.len() != item.signature.parameters.len() {
            self.error(
                format!(
                    "`{}` expects {} arguments, found {}",
                    item.qualified_name,
                    item.signature.parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in args.iter().zip(item.signature.parameters) {
            let parameter_type = self.catalog_type(parameter.ty, &variables);
            self.expr(argument, Some(parameter_type));
            self.validate_catalog_argument(argument, parameter.rule, item);
        }
        let operation = item.operation_semantics();
        if operation.availability == Availability::OnAttach
            && (!self.checking_suspension || self.current_action != ActionKind::OnAttach)
        {
            self.error(
                format!("`{}` must be awaited in `onAttach`", item.qualified_name),
                span,
            );
        }
        let type_arguments = item
            .signature
            .type_parameters
            .iter()
            .map(|parameter| variables[parameter.name])
            .collect();
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::StandardLibrary {
                item: item.id,
                type_arguments,
                receiver: receiver.as_ref().map(|receiver| receiver.value),
                receiver_type: receiver.as_ref().map(|receiver| receiver.ty),
                receiver_members: receiver
                    .map(|receiver| receiver.members)
                    .unwrap_or_default(),
            },
        );
        Some(result)
    }

    fn catalog_candidate_may_apply(&mut self, candidate: &CallCandidate, receiver: Type) -> bool {
        let receiver = self.shallow_type(receiver);
        let declared = candidate
            .receiver()
            .expect("only method candidates are matched against receivers");
        match declared {
            CatalogTypeRef::Core(expected) => {
                let expected = self.declared_type(DeclaredTypeRef::Core(expected));
                matches!(receiver, Type::Variable(_)) || receiver == expected
            }
            CatalogTypeRef::Standard(standard) => {
                matches!(receiver, Type::Variable(_)) || receiver == self.standard_type(standard)
            }
            CatalogTypeRef::Array(_) => {
                matches!(receiver, Type::Variable(_) | Type::Array(_))
            }
            CatalogTypeRef::Result(_) => {
                matches!(receiver, Type::Variable(_) | Type::Result(_))
            }
            CatalogTypeRef::Variable(name) => candidate
                .item
                .signature
                .type_parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .is_none_or(|parameter| {
                    parameter.constraints.iter().all(|constraint| {
                        matches!(receiver, Type::Variable(_))
                            || type_may_have_capability(
                                self.inference.type_store(),
                                receiver,
                                constraint.capability(),
                            )
                    })
                }),
        }
    }

    fn ambiguous_catalog_call(
        &mut self,
        callee: &[String],
        candidates: &[CallCandidate],
        span: Span,
    ) {
        let names = candidates
            .iter()
            .map(|candidate| candidate.item.qualified_name)
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            format!(
                "call to `{}` is ambiguous between {names}",
                callee.join(".")
            ),
            span,
        );
    }

    fn catalog_type(
        &mut self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, Type>,
    ) -> Type {
        match ty {
            CatalogTypeRef::Core(core) => self.declared_type(DeclaredTypeRef::Core(core)),
            CatalogTypeRef::Standard(standard) => self.standard_type(standard),
            CatalogTypeRef::Variable(name) => variables[name],
            CatalogTypeRef::Array(element) => {
                let element = self.catalog_type(*element, variables);
                Type::Array(self.array_type_id(element))
            }
            CatalogTypeRef::Result(value) => {
                let value = self.catalog_type(*value, variables);
                Type::Result(self.inference.result_type(value))
            }
        }
    }

    fn validate_catalog_argument(
        &mut self,
        argument: &Expr,
        rule: ParameterRule,
        item: &StdlibItem,
    ) {
        match rule {
            ParameterRule::Value => {}
            ParameterRule::StringLiteral if !matches!(argument.kind, ExprKind::String(_)) => {
                self.error(
                    format!("`{}` expects a string literal", item.qualified_name),
                    argument.span,
                );
            }
            ParameterRule::SignatureLiteral => match &argument.kind {
                ExprKind::Signature(signature) => {
                    if let Err(message) = parse_signature(signature) {
                        self.error(message, argument.span);
                    }
                }
                _ => self.error(
                    format!("`{}` expects a `sig\"...\"` literal", item.qualified_name),
                    argument.span,
                ),
            },
            ParameterRule::StringLiteral => {}
        }
    }

    fn array_type_id(&mut self, element: Type) -> ArrayTypeId {
        self.inference.array_type(element)
    }

    fn path(
        &mut self,
        path: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<PathResolution> {
        match path {
            [root, field, fields @ ..] if root == "current" || root == "old" => {
                if self.checking_state_source {
                    self.error(
                        "a state field cannot read from its own `current` or `old` snapshot",
                        span,
                    );
                    return None;
                }
                if self.in_function {
                    self.error(
                        "functions are independent of action snapshots; pass the value as a parameter",
                        span,
                    );
                    return None;
                }
                if self.current_action == ActionKind::OnAttach {
                    self.error(
                        "state snapshots are not available until `onAttach` completes",
                        span,
                    );
                    return None;
                }
                let Some((id, ty)) = self.state_fields.get(field).copied() else {
                    self.error(format!("unknown state field `{field}`"), span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "current" {
                    ResolvedValue::CurrentState(id)
                } else {
                    ResolvedValue::OldState(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [root, field, fields @ ..] if root == "settings" || root == "oldSettings" => {
                let Some((id, ty)) = self.settings.get(field).copied() else {
                    self.error(format!("unknown setting `{field}`"), span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "settings" {
                    ResolvedValue::Setting(id)
                } else {
                    ResolvedValue::OldSetting(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [name, fields @ ..] => {
                let Some(binding) = self.binding_for_use(name, span) else {
                    self.error(format!("unknown variable `{name}`"), span);
                    return None;
                };
                let (ty, members) =
                    self.resolve_members_or_defer(binding.ty, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: binding.id.map(ResolvedValue::Variable),
                    members,
                })
            }
            _ => {
                self.error(format!("unknown value `{}`", path.join(".")), span);
                None
            }
        }
    }

    fn resolve_members_or_defer(
        &mut self,
        ty: Type,
        fields: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<(Type, Option<Vec<ResolvedMember>>)> {
        if fields.is_empty() {
            return Some((ty, Some(Vec::new())));
        }
        if matches!(self.shallow_type(ty), Type::Variable(_))
            && let Some(expression) = expression
        {
            let result = self.fresh_inference(Requirements::NONE, None);
            self.deferred_member_paths.push(DeferredMemberPath {
                expression,
                receiver: ty,
                fields: fields.to_vec(),
                result,
                span,
            });
            return Some((result, None));
        }
        let (ty, members) = self.resolve_members(ty, fields, span)?;
        Some((ty, Some(members)))
    }

    fn resolve_members(
        &mut self,
        mut ty: Type,
        fields: &[String],
        span: Span,
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for field in fields {
            let shallow_type = self.shallow_type(ty);
            let (next, member) = self.resolve_member(shallow_type, field, span)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    fn resolve_member(
        &mut self,
        ty: Type,
        field: &str,
        span: Span,
    ) -> Option<(Type, ResolvedMember)> {
        if let Some(resolved) = self.lookup_member(ty, field) {
            return Some(resolved);
        }
        match ty {
            Type::Known(_) if self.source_record_id(ty).is_some() => {
                self.error(format!("unknown record field `{field}`"), span)
            }
            Type::Known(id) => {
                let name = self.type_name(Type::Known(id));
                self.error(format!("{name} has no field `{field}`"), span)
            }
            _ => {
                let ty = self.type_name(ty);
                self.error(format!("`{field}` cannot be accessed on `{ty}`"), span);
            }
        }
        None
    }

    fn lookup_member(&self, ty: Type, field: &str) -> Option<(Type, ResolvedMember)> {
        if let Some(owner) = self.standard_type_id(ty)
            && let Some(field) = StandardLibrary::new().public_field(owner, field)
        {
            return Some((
                self.declared_type(field.ty),
                ResolvedMember::StandardField(field.id),
            ));
        }
        match self.source_record_id(ty) {
            Some(record_id) => self
                .records
                .iter()
                .find(|record| record.id == record_id)
                .and_then(|record| record.fields.iter().find(|item| item.name == field))
                .map(|field| {
                    (
                        self.syntax_type(field.ty),
                        ResolvedMember::RecordField(field.id),
                    )
                }),
            None => None,
        }
    }

    fn resolve_deferred_member_paths(&mut self) {
        let mut pending = std::mem::take(&mut self.deferred_member_paths);
        loop {
            let mut unresolved = Vec::new();
            let mut made_progress = false;
            for deferred in pending {
                let receiver = self.shallow_type(deferred.receiver);
                if matches!(receiver, Type::Variable(_)) {
                    unresolved.push(deferred);
                    continue;
                }
                self.finish_deferred_member_path(deferred, receiver);
                made_progress = true;
            }
            pending = unresolved;
            if pending.is_empty() {
                return;
            }

            let mut variables = Vec::new();
            for deferred in &pending {
                if let Type::Variable(variable) = self.shallow_type(deferred.receiver)
                    && !variables.contains(&variable)
                {
                    variables.push(variable);
                }
            }
            for variable in variables {
                let constraints = pending
                    .iter()
                    .filter(|deferred| {
                        self.shallow_type(deferred.receiver) == Type::Variable(variable)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let mut candidates = self.member_receiver_types();
                candidates.retain(|candidate| {
                    constraints.iter().all(|constraint| {
                        let Some((result, _)) = self.lookup_members(*candidate, &constraint.fields)
                        else {
                            return false;
                        };
                        match self.shallow_type(constraint.result) {
                            Type::Variable(_) => true,
                            expected => result == expected,
                        }
                    })
                });
                if let [candidate] = candidates.as_slice() {
                    self.unify(Type::Variable(variable), *candidate, constraints[0].span);
                    made_progress = true;
                }
            }
            if !made_progress {
                break;
            }
        }

        let mut diagnosed = HashSet::new();
        for deferred in &pending {
            let Type::Variable(variable) = self.shallow_type(deferred.receiver) else {
                continue;
            };
            if !diagnosed.insert(variable) {
                continue;
            }
            let constraints = pending
                .iter()
                .filter(|candidate| {
                    self.shallow_type(candidate.receiver) == Type::Variable(variable)
                })
                .collect::<Vec<_>>();
            let mut candidates = self.member_receiver_types();
            candidates.retain(|candidate| {
                constraints.iter().all(|constraint| {
                    self.lookup_members(*candidate, &constraint.fields)
                        .is_some()
                })
            });
            let fields = constraints
                .iter()
                .flat_map(|constraint| constraint.fields.iter())
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = if candidates.is_empty() {
                format!("cannot infer a type that provides the accessed fields {fields}")
            } else {
                let candidates = candidates
                    .into_iter()
                    .map(|candidate| self.type_name(candidate))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "member access does not uniquely determine its receiver type; fields {fields} match {candidates}"
                )
            };
            self.error(message, constraints[0].span);
        }
    }

    fn finish_deferred_member_path(&mut self, deferred: DeferredMemberPath, receiver: Type) {
        let Some((result, members)) =
            self.resolve_members(receiver, &deferred.fields, deferred.span)
        else {
            return;
        };
        if self.unify(result, deferred.result, deferred.span).is_some() {
            self.semantics
                .resolve_path_members(deferred.expression, members);
        }
    }

    fn lookup_members(
        &self,
        mut ty: Type,
        fields: &[String],
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for field in fields {
            let (next, member) = self.lookup_member(ty, field)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    fn member_receiver_types(&self) -> Vec<Type> {
        StandardLibrary::new()
            .types()
            .iter()
            .filter(|ty| StandardLibrary::new().public_fields(ty.id).next().is_some())
            .map(|ty| self.standard_type(ty.id))
            .chain(
                self.records
                    .iter()
                    .map(|record| self.record_type(record.id)),
            )
            .collect()
    }

    fn type_name(&mut self, ty: Type) -> String {
        let ty = self.shallow_type(ty);
        match ty {
            Type::Array(array) => {
                let element = self.inference.array_element(array);
                format!("Array<{}>", self.type_name(element))
            }
            Type::Option(option) => {
                let value = self.inference.option_value(option);
                format!("{}?", self.type_name(value))
            }
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                format!("{}!", self.type_name(value))
            }
            Type::Variable(_) => "an inferred type".to_owned(),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                TypeKind::Record(record) => self
                    .records
                    .iter()
                    .find(|candidate| candidate.id == *record)
                    .map_or_else(|| ty.to_string(), |record| record.name.clone()),
                TypeKind::Enum(enumeration) => self
                    .enums
                    .iter()
                    .find(|candidate| candidate.id == *enumeration)
                    .map_or_else(|| ty.to_string(), |enumeration| enumeration.name.clone()),
                _ => self.inference.known_type_name(id),
            },
            _ => ty.to_string(),
        }
    }

    fn inference_error_message(&mut self, error: InferenceError) -> String {
        match error {
            InferenceError::Message(message) => message,
            InferenceError::TypeMismatch { left, right } => format!(
                "types do not match: `{}` and `{}`",
                self.type_name(left),
                self.type_name(right)
            ),
            InferenceError::UnsupportedOperation(ty) => {
                format!(
                    "type `{}` does not support this operation",
                    self.type_name(ty)
                )
            }
            InferenceError::UnsatisfiedConstraints(ty) => format!(
                "type `{}` does not satisfy the inferred constraints",
                self.type_name(ty)
            ),
            InferenceError::IntegerLiteralOutOfRange(ty) => {
                format!("integer literal does not fit in `{}`", self.type_name(ty))
            }
        }
    }

    fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .or_else(|| self.globals.get(name).copied())
    }

    fn binding_for_use(&mut self, name: &str, span: Span) -> Option<Binding> {
        let binding = self.binding(name)?;
        if binding.debug_only && !self.debug_context {
            self.error(
                format!("debug-only binding `{name}` can only be used from debug code"),
                span,
            );
        }
        Some(binding)
    }

    fn bind_pattern_value(&mut self, binding: &crate::ast::PatternBinding, ty: Type) {
        self.semantics.resolve_value_type(binding.id, ty);
        self.scopes.last_mut().unwrap().insert(
            binding.name.clone(),
            Binding {
                id: Some(binding.id),
                ty,
                mutable: false,
                debug_only: self.debug_context,
            },
        );
    }

    fn expected_value_type(&mut self, ty: Type) -> Type {
        match self.shallow_type(ty) {
            Type::Option(option) => self.inference.option_value(option),
            Type::Result(result) => self.inference.result_value(result),
            ty => ty,
        }
    }

    fn expect_expression(
        &mut self,
        expression: ExprId,
        actual: Type,
        expected: Option<Type>,
        span: Span,
    ) -> Option<Type> {
        let Some(expected) = expected else {
            return Some(actual);
        };
        let expected = self.shallow_type(expected);
        let actual_shallow = self.shallow_type(actual);
        let (kind, value) = match (expected, actual_shallow) {
            (Type::Option(_), Type::Option(_)) | (Type::Result(_), Type::Result(_)) => {
                return self.unify(actual, expected, span);
            }
            (Type::Variable(_), _) => return self.unify(actual, expected, span),
            (_, Type::Result(result)) => {
                let value = self.inference.result_value(result);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify(value, expected, span)?;
                }
                let actual = self.type_name(actual_shallow);
                let expected = self.type_name(expected);
                self.error(
                    format!(
                        "cannot use fallible `{actual}` where `{expected}` is required; unwrap it with `else`, propagate it with `?`, or use `retry` in `onAttach`"
                    ),
                    span,
                );
                return None;
            }
            (_, Type::Option(option)) => {
                let value = self.inference.option_value(option);
                if matches!(self.shallow_type(value), Type::Variable(_)) {
                    self.unify(value, expected, span)?;
                }
                let actual = self.type_name(actual_shallow);
                let expected = self.type_name(expected);
                self.error(
                    format!(
                        "cannot use optional `{actual}` where `{expected}` is required; unwrap it with `else` or handle it with `match`"
                    ),
                    span,
                );
                return None;
            }
            (Type::Option(option), _) => (
                ValueConversionKind::LiftOption,
                self.inference.option_value(option),
            ),
            (Type::Result(result), _) => (
                ValueConversionKind::LiftResult,
                self.inference.result_value(result),
            ),
            _ => return self.unify(actual, expected, span),
        };
        self.unify(actual, value, span)?;
        self.semantics
            .resolve_value_conversion(expression, kind, actual, expected);
        Some(expected)
    }

    fn fresh_inference(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
    ) -> Type {
        self.inference.fresh(requirements, largest_literal)
    }

    fn shallow_type(&mut self, ty: Type) -> Type {
        self.inference.shallow(ty)
    }

    fn unify(&mut self, left: Type, right: Type, span: Span) -> Option<Type> {
        match self.inference.unify(left, right) {
            Ok(ty) => Some(ty),
            Err(error) => {
                let message = self.inference_error_message(error);
                self.error(message, span);
                None
            }
        }
    }

    fn require(&mut self, ty: Type, requirements: Requirements, span: Span) -> Option<()> {
        match self.inference.require(ty, requirements) {
            Ok(()) => Some(()),
            Err(error) => {
                let message = self.inference_error_message(error);
                self.error(message, span);
                None
            }
        }
    }

    fn default_inference_variables(&mut self) {
        for error in self.inference.default_unbound() {
            let message = self.inference_error_message(error);
            self.error(message, Span::default());
        }
    }

    fn diagnose_ambiguous_process_reads(&mut self) {
        let reads = self.inferred_process_reads.clone();
        for (ty, span) in reads {
            if self.inference.is_unbound_without_default(ty) {
                self.error(
                    "cannot infer the memory type read by `process.read`; add a result annotation such as `let value: i32! = process.read(address)`, or use `process.read.i32(address)`",
                    span,
                );
            }
        }
    }

    fn resolved_type(&mut self, ty: Type) -> Type {
        self.inference.resolve(ty)
    }

    fn enum_type(&self, enumeration: EnumTypeId) -> Type {
        match enumeration {
            EnumTypeId::Source(id) => Type::Known(self.inference.type_store().id_for_enum(id)),
            EnumTypeId::Standard(id) => self.standard_type(id),
        }
    }

    fn enum_info(&self, enumeration: EnumTypeId) -> Option<EnumInfo> {
        match enumeration {
            EnumTypeId::Source(id) => self
                .enums
                .iter()
                .find(|declaration| declaration.id == id)
                .map(|declaration| EnumInfo {
                    name: declaration.name.clone(),
                    variants: declaration
                        .variants
                        .iter()
                        .map(|variant| EnumVariantInfo {
                            id: ResolvedEnumVariantId::Source(variant.id),
                            name: variant.name.clone(),
                            payload: variant.payload.map(|ty| self.syntax_type(ty)),
                        })
                        .collect(),
                }),
            EnumTypeId::Standard(id) => {
                let library = StandardLibrary::new();
                let declaration = library.type_decl(id);
                let variants = library.variants_of(id).collect::<Vec<_>>();
                (!variants.is_empty()).then(|| EnumInfo {
                    name: declaration.name.to_owned(),
                    variants: variants
                        .into_iter()
                        .map(|variant| EnumVariantInfo {
                            id: ResolvedEnumVariantId::Standard(variant.id),
                            name: variant.name.to_owned(),
                            payload: None,
                        })
                        .collect(),
                })
            }
        }
    }

    fn enum_info_for_type(&self, ty: Type) -> Option<(EnumTypeId, EnumInfo)> {
        let enumeration = match (ty, self.source_enum_id(ty)) {
            (Type::Known(_), Some(id)) => EnumTypeId::Source(id),
            (Type::Known(_), None) => EnumTypeId::Standard(self.standard_type_id(ty)?),
            _ => return None,
        };
        self.enum_info(enumeration)
            .map(|declaration| (enumeration, declaration))
    }

    fn finalize_array_types(&mut self) {
        self.inference.finalize_arrays();
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(Diagnostic::type_error(message, span));
    }
}

fn inference_type_for_declared(types: &TypeStore, ty: DeclaredTypeRef) -> Type {
    match ty {
        DeclaredTypeRef::Core(core) => Type::from_core(core),
        DeclaredTypeRef::Standard(standard) => Type::Known(types.id_for_standard(standard)),
    }
}

fn syntax_type(ty: TypeRef, named_types: &HashMap<TypeNameId, Type>, types: &TypeStore) -> Type {
    match ty {
        TypeRef::Named(id) => named_types[&id],
        TypeRef::Standard(standard) => Type::Known(types.id_for_standard(standard)),
        TypeRef::Record(record) => Type::Known(types.id_for_record(record)),
        TypeRef::Enum(enumeration) => Type::Known(types.id_for_enum(enumeration)),
        ty => Type::from(ty),
    }
}

fn closest_name<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let normalized_name = normalize_name(name);
    let maximum_distance = match normalized_name.chars().count() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    let mut seen = HashSet::new();
    let mut best: Option<(usize, String)> = None;
    let mut tied = false;
    for candidate in candidates {
        if candidate == name || !seen.insert(candidate) {
            continue;
        }
        let distance = edit_distance(&normalized_name, &normalize_name(candidate));
        if distance > maximum_distance {
            continue;
        }
        match &best {
            None => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance < *best_distance => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance == *best_distance => tied = true,
            Some(_) => {}
        }
    }
    (!tied)
        .then(|| best.map(|(_, candidate)| candidate))
        .flatten()
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_character != *right_character));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn is_constant(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Bool(_) | ExprKind::Int { .. } | ExprKind::Float(_) => true,
        ExprKind::Enum { payload: None, .. } => true,
        ExprKind::Unary { expr, .. } => is_constant(expr),
        _ => false,
    }
}

fn definitely_returns(block: &Block) -> bool {
    block.statements.iter().any(|statement| match statement {
        Stmt::Return { .. } | Stmt::Throw { .. } => true,
        Stmt::If {
            then_block,
            else_block: Some(else_block),
            ..
        } => definitely_returns(then_block) && definitely_returns(else_block),
        Stmt::Suspend { .. } => false,
        _ => false,
    })
}

fn contains_value_return(block: &Block) -> bool {
    let mut finder = ValueReturnFinder(false);
    finder.visit_block(block);
    finder.0
}

fn contains_propagation(expression: &Expr) -> bool {
    #[derive(Default)]
    struct PropagationFinder(bool);

    impl<'ast> Visitor<'ast> for PropagationFinder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(expression.kind, ExprKind::Propagate(_)) {
                self.0 = true;
            } else if !self.0 {
                visit::walk_expr(self, expression);
            }
        }
    }

    let mut finder = PropagationFinder::default();
    finder.visit_expr(expression);
    finder.0
}

struct ValueReturnFinder(bool);

impl<'ast> Visitor<'ast> for ValueReturnFinder {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if matches!(statement, Stmt::Return { value: Some(_), .. }) {
            self.0 = true;
        } else if !self.0 {
            visit::walk_stmt(self, statement);
        }
    }

    fn visit_expr(&mut self, _expression: &'ast Expr) {}
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    use super::*;

    fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
        let program = parser::parse(source, lexer::lex(source).unwrap()).unwrap();
        check(&program).map(|_| ())
    }

    #[test]
    fn infers_local_from_precisely_typed_state() {
        check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            split {
                let next = current.level + 1;
                return next != old.level;
            }
            "#,
        )
        .unwrap();
    }

    #[test]
    fn rejects_mixed_integer_widths() {
        let errors = check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            split { return current.level == 1u32; }
            "#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("types do not match"))
        );
    }

    #[test]
    fn infers_integer_literals_bidirectionally_and_from_array_elements() {
        check_source(
            r#"
            state "game" { level: u16 at 0x1234 }
            whileAttached {
                let byte: u8 = 0x8b
                let bytes = [0x48, byte]
                if (0 == current.level && (1 + current.level) == 2 && bytes.get(0) == 0x48) {
                    print("inferred")
                }
            }
            "#,
        )
        .unwrap();
    }
}
