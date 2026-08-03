//! Resolved declaration index and typed body HIR.
//!
//! Lowering first establishes an inspectable declaration index. After checking,
//! typed expressions and blocks own body shape, child identities, types, and
//! type-directed resolutions without attaching them to syntax nodes.

use std::collections::HashMap;

use crate::{
    ast::{
        ActionKind, AssignmentId, BinaryOp, Block, EnumId, EnumVariantId, Expr, ExprId, ExprKind,
        FallbackBranch, FunctionId, InterpolatedPart, MatchArm, MatchPattern, PatternBinding,
        PatternId, Program as SyntaxProgram, RecordId, SettingChoiceOptionId, SettingKind, Span,
        Stmt, SuspensionMode, TypeRef, UnaryOp, ValueId,
    },
    semantic::{
        FunctionInstance, ResolvedCall, ResolvedEnumVariantId, ResolvedMember,
        ResolvedRecordFieldId, ResolvedRecordId, ResolvedValue, ResolvedWrapperPattern,
        SemanticModel, ValueConversion,
    },
    stdlib::{Implementation, StandardLibrary, StdlibItemId, StdlibTypeId},
    types::{EnumTypeId, TypeId, TypeKind},
    visit::{self, Visitor as SyntaxVisitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclarationId {
    StateField(ValueId),
    Setting(ValueId),
    Global(ValueId),
    Record(RecordId),
    Enum(EnumId),
    Function(FunctionId),
    Action(ActionKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub id: DeclarationId,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct DeclarationIndex {
    declarations: Vec<Declaration>,
    by_name: HashMap<String, Vec<usize>>,
}

impl DeclarationIndex {
    pub(crate) fn lower(syntax: &SyntaxProgram) -> Self {
        let mut program = Self::default();
        if let Some(state) = &syntax.state {
            for field in state.canonical_fields() {
                program.push(DeclarationId::StateField(field.id), &field.name, field.span);
            }
            if let (Some(value), Some(enumeration)) = (state.layout_value, &state.layout_enum) {
                program.push(DeclarationId::Global(value), "layout", state.span);
                program.push(
                    DeclarationId::Enum(enumeration.id),
                    &enumeration.name,
                    enumeration.span,
                );
            }
        }
        for setting in &syntax.settings {
            program.push(
                DeclarationId::Setting(setting.id),
                &setting.name,
                setting.span,
            );
        }
        for global in &syntax.globals {
            program.push(DeclarationId::Global(global.id), &global.name, global.span);
        }
        for record in &syntax.records {
            program.push(DeclarationId::Record(record.id), &record.name, record.span);
        }
        for enumeration in &syntax.enums {
            program.push(
                DeclarationId::Enum(enumeration.id),
                &enumeration.name,
                enumeration.span,
            );
        }
        for function in &syntax.functions {
            program.push(
                DeclarationId::Function(function.id),
                &function.name,
                function.span,
            );
        }
        for action in &syntax.actions {
            program.push(
                DeclarationId::Action(action.kind),
                action.kind.name(),
                action.span,
            );
        }
        program
    }

    fn push(&mut self, id: DeclarationId, name: &str, span: Span) {
        let index = self.declarations.len();
        self.declarations.push(Declaration {
            id,
            name: name.to_owned(),
            span,
        });
        self.by_name.entry(name.to_owned()).or_default().push(index);
    }

    pub fn declarations(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations.iter()
    }

    pub fn declarations_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Declaration> + 'a {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .map(|index| &self.declarations[*index])
    }

    pub fn declaration(&self, id: DeclarationId) -> Option<&Declaration> {
        self.declarations
            .iter()
            .find(|declaration| declaration.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionResolution {
    ValuePath {
        root: Option<ResolvedValue>,
        members: Vec<ResolvedMember>,
    },
    Member {
        members: Vec<ResolvedMember>,
    },
    Call(ResolvedCall),
    RecordLiteral {
        record: ResolvedRecordId,
        fields: Vec<ResolvedRecordFieldId>,
    },
    EnumConstructor {
        variant: ResolvedEnumVariantId,
    },
}

#[derive(Debug, Clone)]
pub enum TypedInterpolatedPart {
    Text(String),
    Expression {
        expression: ExprId,
        conversion: Option<ImplicitConversion>,
    },
}

/// A conversion inserted by type checking rather than written with `as`.
///
/// This conversion is specific to interpolation operands. General value
/// conversions such as lifting `T` into `T?` or `T!` are recorded on
/// [`TypedExpression::conversion`] with semantic source and target types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplicitConversion {
    ToString { source: TypeId },
}

#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    pub resolution: ResolvedPattern,
    pub guard: Option<ExprId>,
    pub value: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedPattern {
    Enum {
        enumeration: EnumTypeId,
        variant: String,
        binding: Option<PatternBinding>,
    },
    Bool(bool),
    Int {
        value: u64,
        suffix: Option<TypeRef>,
    },
    None,
    OptionSome(Option<PatternBinding>),
    ResultSuccess(Option<PatternBinding>),
    ResultError(Option<PatternBinding>),
    Wildcard,
}

#[derive(Debug, Clone)]
pub enum TypedExpressionKind {
    None,
    Bool(bool),
    Int {
        value: u64,
        suffix: Option<TypeRef>,
    },
    Float(f64),
    String(String),
    InterpolatedString(Vec<TypedInterpolatedPart>),
    Signature(String),
    Array(Vec<ExprId>),
    Record {
        record: ResolvedRecordId,
        fields: Vec<(String, ExprId)>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: String,
        payload: Option<ExprId>,
    },
    Match {
        value: ExprId,
        arms: Vec<TypedMatchArm>,
    },
    If {
        condition: ExprId,
        then_expr: ExprId,
        else_expr: ExprId,
    },
    Fallback {
        value: ExprId,
        fallback: TypedFallbackBranch,
    },
    Suspend {
        mode: SuspensionMode,
        destination: ValueId,
        value: ExprId,
    },
    /// Unwraps a result or transfers its error to the nearest failure boundary.
    Propagate {
        value: ExprId,
        target: TypeId,
    },
    Path(Vec<String>),
    Member {
        receiver: ExprId,
        name: String,
        name_span: Span,
    },
    Unary {
        op: UnaryOp,
        expression: ExprId,
    },
    Cast {
        expression: ExprId,
        target: TypeRef,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        source_path: Vec<String>,
        receiver: Option<ExprId>,
        arguments: Vec<ExprId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedFallbackBranch {
    Value(ExprId),
    Return(Option<ExprId>),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct TypedExpression {
    pub id: ExprId,
    pub ty: TypeId,
    pub kind: TypedExpressionKind,
    pub resolution: Option<ExpressionResolution>,
    pub conversion: Option<ValueConversion>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAssignment {
    pub id: AssignmentId,
    pub target: ValueId,
    pub operator: Option<ResolvedCall>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPattern {
    pub id: PatternId,
    pub variant: Option<ResolvedEnumVariantId>,
    pub wrapper: Option<ResolvedWrapperPattern>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedStatementKind {
    Variable {
        value: ValueId,
        initializer: ExprId,
    },
    Assign {
        assignment: ResolvedAssignment,
        op: Option<BinaryOp>,
        value: ExprId,
    },
    If {
        condition: ExprId,
        then_block: TypedBlock,
        else_block: Option<TypedBlock>,
    },
    While {
        condition: ExprId,
        body: TypedBlock,
    },
    For {
        binding: ValueId,
        iterable_value: ValueId,
        index_value: ValueId,
        iterable: ExprId,
        body: TypedBlock,
    },
    Break,
    Continue,
    Return(Option<ExprId>),
    Throw {
        error: ExprId,
        target: TypeId,
    },
    Suspend {
        mode: SuspensionMode,
        binding: Option<ValueId>,
        returns: bool,
        value: ExprId,
    },
    Expression(ExprId),
}

#[derive(Debug, Clone)]
pub struct TypedStatement {
    pub kind: TypedStatementKind,
    pub debug_only: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedBlock {
    pub statements: Vec<TypedStatement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionBody {
    pub function: FunctionInstance,
    pub debug_only: bool,
    pub body: TypedBlock,
}

#[derive(Debug, Clone)]
pub struct ActionBody {
    pub action: ActionKind,
    pub body: TypedBlock,
}

#[derive(Debug, Clone, Copy)]
pub struct GlobalInitializer {
    pub value: ValueId,
    pub expression: ExprId,
    pub debug_only: bool,
}

#[derive(Debug, Clone)]
pub struct TypedProgram {
    standard_library: StandardLibrary,
    declarations: DeclarationIndex,
    expressions: Vec<TypedExpression>,
    assignments: Vec<ResolvedAssignment>,
    patterns: Vec<ResolvedPattern>,
    function_bodies: Vec<FunctionBody>,
    action_bodies: Vec<ActionBody>,
    global_initializers: Vec<GlobalInitializer>,
    state_sources: Vec<(ValueId, ExprId)>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    visible_expression_count: usize,
    visible_function_count: usize,
    library_functions: HashMap<StdlibItemId, FunctionId>,
}

pub(crate) fn visible_expression_count(program: &SyntaxProgram) -> usize {
    #[derive(Default)]
    struct Counter {
        count: usize,
    }

    impl<'ast> SyntaxVisitor<'ast> for Counter {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            self.count = self.count.max(expression.id.index() + 1);
            visit::walk_expr(self, expression);
        }
    }

    let mut counter = Counter::default();
    counter.visit_program(program);
    counter.count
}

impl TypedProgram {
    pub(crate) fn build(
        declarations: DeclarationIndex,
        syntax: &SyntaxProgram,
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
        visible_expression_count: usize,
        visible_function_count: usize,
    ) -> Self {
        let mut builder = TypedBodyBuilder {
            semantics,
            syntax,
            standard_library: &standard_library,
            expressions: HashMap::new(),
            assignments: HashMap::new(),
            patterns: HashMap::new(),
        };
        builder.visit_program(syntax);
        let mut expressions = builder.expressions.into_values().collect::<Vec<_>>();
        expressions.sort_by_key(|expression| expression.id.index());
        let mut assignments = builder.assignments.into_values().collect::<Vec<_>>();
        assignments.sort_by_key(|assignment| assignment.id.index());
        let mut patterns = builder.patterns.into_values().collect::<Vec<_>>();
        patterns.sort_by_key(|pattern| pattern.id.index());
        let function_bodies = syntax
            .functions
            .iter()
            .map(|function| FunctionBody {
                function: FunctionInstance::monomorphic(function.id),
                debug_only: function.debug_only,
                body: lower_block(
                    &function.body,
                    semantics,
                    semantics.function_completion(function.id).filter(|result| {
                        matches!(semantics.types().kind(*result), TypeKind::Result { .. })
                    }),
                ),
            })
            .collect();
        let action_bodies = syntax
            .actions
            .iter()
            .map(|action| ActionBody {
                action: action.kind,
                body: lower_block(&action.body, semantics, None),
            })
            .collect();
        let global_initializers = syntax
            .globals
            .iter()
            .map(|global| GlobalInitializer {
                value: global.id,
                expression: global.value.id,
                debug_only: global.debug_only,
            })
            .collect();
        let state_sources = syntax
            .state
            .iter()
            .flat_map(|state| state.all_fields())
            .filter_map(|field| match &field.source {
                crate::ast::StateSource::Expression(expression) => Some((field.id, expression.id)),
                crate::ast::StateSource::Pointer(_) => None,
            })
            .collect();

        let mut setting_choice_defaults = HashMap::new();
        let mut setting_choice_options = HashMap::new();
        for setting in &syntax.settings {
            if let SettingKind::Choice { options, .. } = &setting.kind {
                setting_choice_defaults.insert(
                    setting.id,
                    semantics
                        .setting_choice_default(setting.id)
                        .expect("checked choice settings have resolved defaults"),
                );
                for option in options {
                    setting_choice_options.insert(
                        option.id,
                        semantics
                            .setting_choice_option(option.id)
                            .expect("checked choice options have resolved variants"),
                    );
                }
            }
        }

        let library_functions = standard_library
            .items()
            .iter()
            .filter_map(|item| {
                let Implementation::LibraryBody { function_name, .. } = item.implementation else {
                    return None;
                };
                let function = syntax
                    .functions
                    .iter()
                    .find(|function| function.name == function_name)?;
                Some((item.id, function.id))
            })
            .collect();

        Self {
            standard_library,
            declarations,
            expressions,
            assignments,
            patterns,
            function_bodies,
            action_bodies,
            global_initializers,
            state_sources,
            setting_choice_defaults,
            setting_choice_options,
            visible_expression_count,
            visible_function_count,
            library_functions,
        }
    }

    pub fn declarations(&self) -> &DeclarationIndex {
        &self.declarations
    }

    pub fn standard_library(&self) -> &StandardLibrary {
        &self.standard_library
    }

    /// Resolves a catalog-owned source implementation to its inferred hidden
    /// function template. Concrete calls instantiate this declaration through
    /// the same `FunctionInstance` path as user-authored generic functions.
    pub fn library_function(&self, item: StdlibItemId) -> Option<FunctionId> {
        self.library_functions.get(&item).copied()
    }

    pub fn expression(&self, id: ExprId) -> Option<&TypedExpression> {
        self.expressions
            .binary_search_by_key(&id.index(), |expression| expression.id.index())
            .ok()
            .map(|index| &self.expressions[index])
    }

    pub fn expressions(&self) -> impl Iterator<Item = &TypedExpression> {
        self.expressions
            .iter()
            .filter(|expression| expression.id.index() < self.visible_expression_count)
    }

    pub(crate) fn all_expressions(&self) -> impl Iterator<Item = &TypedExpression> {
        self.expressions.iter()
    }

    pub(crate) fn visible_expression_count(&self) -> usize {
        self.visible_expression_count
    }

    pub fn call(&self, id: ExprId) -> Option<&ResolvedCall> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::Call(call)) => Some(call),
            _ => None,
        }
    }

    pub fn value_path(&self, id: ExprId) -> Option<(Option<ResolvedValue>, &[ResolvedMember])> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::ValuePath { root, members }) => {
                Some((*root, members.as_slice()))
            }
            _ => None,
        }
    }

    pub fn record_literal_fields(&self, id: ExprId) -> Option<&[ResolvedRecordFieldId]> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::RecordLiteral { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    pub fn enum_variant(&self, id: ExprId) -> Option<ResolvedEnumVariantId> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::EnumConstructor { variant }) => Some(*variant),
            _ => None,
        }
    }

    pub fn assignment(&self, id: AssignmentId) -> Option<ResolvedAssignment> {
        self.assignments
            .binary_search_by_key(&id.index(), |assignment| assignment.id.index())
            .ok()
            .map(|index| self.assignments[index].clone())
    }

    pub fn pattern(&self, id: PatternId) -> Option<ResolvedPattern> {
        self.patterns
            .binary_search_by_key(&id.index(), |pattern| pattern.id.index())
            .ok()
            .map(|index| self.patterns[index])
    }

    pub fn patterns(&self) -> impl Iterator<Item = ResolvedPattern> + '_ {
        self.patterns.iter().copied()
    }

    pub fn function_body(&self, function: FunctionId) -> Option<&TypedBlock> {
        self.function_bodies
            .iter()
            .find(|body| {
                body.function.function == function && body.function.type_arguments.is_empty()
            })
            .map(|body| &body.body)
    }

    pub fn function_instance_body(&self, function: &FunctionInstance) -> Option<&TypedBlock> {
        self.function_bodies
            .iter()
            .find(|body| &body.function == function)
            .map(|body| &body.body)
    }

    pub fn action_body(&self, action: ActionKind) -> Option<&TypedBlock> {
        self.action_bodies
            .iter()
            .find(|body| body.action == action)
            .map(|body| &body.body)
    }

    pub fn function_bodies(&self) -> impl Iterator<Item = &FunctionBody> {
        self.function_bodies
            .iter()
            .filter(|body| body.function.function.index() < self.visible_function_count)
    }

    pub(crate) fn all_function_bodies(&self) -> impl Iterator<Item = &FunctionBody> {
        self.function_bodies.iter()
    }

    pub fn action_bodies(&self) -> impl Iterator<Item = &ActionBody> {
        self.action_bodies.iter()
    }

    pub fn global_initializers(&self) -> impl Iterator<Item = GlobalInitializer> + '_ {
        self.global_initializers.iter().copied()
    }

    pub fn state_sources(&self) -> impl Iterator<Item = (ValueId, ExprId)> + '_ {
        self.state_sources.iter().copied()
    }

    pub fn setting_choice_default(&self, setting: ValueId) -> Option<EnumVariantId> {
        self.setting_choice_defaults.get(&setting).copied()
    }

    pub fn setting_choice_option(&self, option: SettingChoiceOptionId) -> Option<EnumVariantId> {
        self.setting_choice_options.get(&option).copied()
    }
}

struct TypedBodyBuilder<'a> {
    semantics: &'a SemanticModel,
    syntax: &'a SyntaxProgram,
    standard_library: &'a StandardLibrary,
    expressions: HashMap<ExprId, TypedExpression>,
    assignments: HashMap<AssignmentId, ResolvedAssignment>,
    patterns: HashMap<PatternId, ResolvedPattern>,
}

impl<'ast> SyntaxVisitor<'ast> for TypedBodyBuilder<'_> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Assign { id, span, .. } = statement {
            self.assignments.insert(
                *id,
                ResolvedAssignment {
                    id: *id,
                    target: self
                        .semantics
                        .assignment_target(*id)
                        .expect("checked assignments have resolved targets"),
                    operator: self.semantics.assignment_call(*id).cloned(),
                    span: *span,
                },
            );
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        let resolution = if let Some(variant) = self.semantics.enum_variant(expression.id) {
            Some(ExpressionResolution::EnumConstructor { variant })
        } else if let Some(call) = self.semantics.call(expression.id) {
            Some(ExpressionResolution::Call(call.clone()))
        } else {
            match &expression.kind {
                ExprKind::Path(_) => Some(ExpressionResolution::ValuePath {
                    root: self.semantics.value(expression.id),
                    members: self
                        .semantics
                        .path_members(expression.id)
                        .unwrap_or_default()
                        .to_vec(),
                }),
                ExprKind::Member { .. } => Some(ExpressionResolution::Member {
                    members: self
                        .semantics
                        .path_members(expression.id)
                        .unwrap_or_default()
                        .to_vec(),
                }),
                ExprKind::Call { .. } => None,
                ExprKind::Record { .. } => Some(ExpressionResolution::RecordLiteral {
                    record: self
                        .semantics
                        .record_literal(expression.id)
                        .expect("checked record literals have resolved nominal identities"),
                    fields: self
                        .semantics
                        .record_literal_fields(expression.id)
                        .expect("checked record literals have resolved fields")
                        .to_vec(),
                }),
                _ => None,
            }
        };
        self.expressions.insert(
            expression.id,
            TypedExpression {
                id: expression.id,
                ty: self
                    .semantics
                    .expression_type(expression.id)
                    .expect("checked expressions have resolved types"),
                kind: lower_expression_kind(
                    expression,
                    self.semantics,
                    self.syntax,
                    self.standard_library,
                ),
                resolution,
                conversion: self.semantics.value_conversion(expression.id),
                span: expression.span,
            },
        );
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        self.patterns.insert(
            arm.pattern_id,
            ResolvedPattern {
                id: arm.pattern_id,
                variant: self.semantics.pattern_variant(arm.pattern_id),
                wrapper: self.semantics.wrapper_pattern(arm.pattern_id),
                span: arm.span,
            },
        );
        visit::walk_match_arm(self, arm);
    }
}

pub trait TypedVisitor: Sized {
    fn visit_program(&mut self, program: &TypedProgram) {
        walk_typed_program(self, program);
    }

    fn visit_block(&mut self, block: &TypedBlock, program: &TypedProgram) {
        walk_typed_block(self, block, program);
    }

    fn visit_statement(&mut self, statement: &TypedStatement, program: &TypedProgram) {
        walk_typed_statement(self, statement, program);
    }

    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        walk_typed_expression(self, expression, program);
    }

    fn visit_match_arm(&mut self, arm: &TypedMatchArm, program: &TypedProgram) {
        walk_typed_match_arm(self, arm, program);
    }
}

pub fn walk_typed_program<V: TypedVisitor>(visitor: &mut V, program: &TypedProgram) {
    for initializer in program.global_initializers() {
        visitor.visit_expression(
            program
                .expression(initializer.expression)
                .expect("global initializer belongs to typed HIR"),
            program,
        );
    }
    for (_, expression) in program.state_sources() {
        visitor.visit_expression(
            program
                .expression(expression)
                .expect("state source belongs to typed HIR"),
            program,
        );
    }
    for function in program.function_bodies() {
        visitor.visit_block(&function.body, program);
    }
    for action in program.action_bodies() {
        visitor.visit_block(&action.body, program);
    }
}

pub fn walk_typed_block<V: TypedVisitor>(
    visitor: &mut V,
    block: &TypedBlock,
    program: &TypedProgram,
) {
    for statement in &block.statements {
        visitor.visit_statement(statement, program);
    }
}

pub fn walk_typed_statement<V: TypedVisitor>(
    visitor: &mut V,
    statement: &TypedStatement,
    program: &TypedProgram,
) {
    let mut visit_expression = |id| {
        visitor.visit_expression(
            program
                .expression(id)
                .expect("statement expression belongs to typed HIR"),
            program,
        );
    };
    match &statement.kind {
        TypedStatementKind::Variable { initializer, .. } => visit_expression(*initializer),
        TypedStatementKind::Assign { value, .. } => visit_expression(*value),
        TypedStatementKind::If {
            condition,
            then_block,
            else_block,
        } => {
            visit_expression(*condition);
            visitor.visit_block(then_block, program);
            if let Some(else_block) = else_block {
                visitor.visit_block(else_block, program);
            }
        }
        TypedStatementKind::While { condition, body } => {
            visit_expression(*condition);
            visitor.visit_block(body, program);
        }
        TypedStatementKind::For { iterable, body, .. } => {
            visit_expression(*iterable);
            visitor.visit_block(body, program);
        }
        TypedStatementKind::Break | TypedStatementKind::Continue => {}
        TypedStatementKind::Return(value) => {
            if let Some(value) = value {
                visit_expression(*value);
            }
        }
        TypedStatementKind::Throw { error, .. } => visit_expression(*error),
        TypedStatementKind::Suspend { value, .. } | TypedStatementKind::Expression(value) => {
            visit_expression(*value);
        }
    }
}

pub fn walk_typed_expression<V: TypedVisitor>(
    visitor: &mut V,
    expression: &TypedExpression,
    program: &TypedProgram,
) {
    let mut visit_expression = |id| {
        visitor.visit_expression(
            program
                .expression(id)
                .expect("child expression belongs to typed HIR"),
            program,
        );
    };
    match &expression.kind {
        TypedExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                if let TypedInterpolatedPart::Expression { expression, .. } = part {
                    visit_expression(*expression);
                }
            }
        }
        TypedExpressionKind::Array(elements) => {
            for element in elements {
                visit_expression(*element);
            }
        }
        TypedExpressionKind::Record { fields, .. } => {
            for (_, value) in fields {
                visit_expression(*value);
            }
        }
        TypedExpressionKind::Enum { payload, .. } => {
            if let Some(payload) = payload {
                visit_expression(*payload);
            }
        }
        TypedExpressionKind::Match { value, arms } => {
            visit_expression(*value);
            for arm in arms {
                visitor.visit_match_arm(arm, program);
            }
        }
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_expression(*condition);
            visit_expression(*then_expr);
            visit_expression(*else_expr);
        }
        TypedExpressionKind::Fallback { value, fallback } => {
            visit_expression(*value);
            match fallback {
                TypedFallbackBranch::Value(fallback) => visit_expression(*fallback),
                TypedFallbackBranch::Return(Some(value)) => visit_expression(*value),
                TypedFallbackBranch::Return(None)
                | TypedFallbackBranch::Break
                | TypedFallbackBranch::Continue => {}
            }
        }
        TypedExpressionKind::Suspend { value, .. }
        | TypedExpressionKind::Propagate { value, .. } => visit_expression(*value),
        TypedExpressionKind::Member { receiver, .. } => visit_expression(*receiver),
        TypedExpressionKind::Unary { expression, .. }
        | TypedExpressionKind::Cast { expression, .. } => visit_expression(*expression),
        TypedExpressionKind::Binary { left, right, .. } => {
            visit_expression(*left);
            visit_expression(*right);
        }
        TypedExpressionKind::Call {
            receiver,
            arguments,
            ..
        } => {
            if let Some(receiver) = receiver {
                visit_expression(*receiver);
            }
            for argument in arguments {
                visit_expression(*argument);
            }
        }
        TypedExpressionKind::None
        | TypedExpressionKind::Bool(_)
        | TypedExpressionKind::Int { .. }
        | TypedExpressionKind::Float(_)
        | TypedExpressionKind::String(_)
        | TypedExpressionKind::Signature(_)
        | TypedExpressionKind::Path(_) => {}
    }
}

pub fn walk_typed_match_arm<V: TypedVisitor>(
    visitor: &mut V,
    arm: &TypedMatchArm,
    program: &TypedProgram,
) {
    if let Some(guard) = arm.guard {
        visitor.visit_expression(
            program
                .expression(guard)
                .expect("match guard belongs to typed HIR"),
            program,
        );
    }
    visitor.visit_expression(
        program
            .expression(arm.value)
            .expect("match value belongs to typed HIR"),
        program,
    );
}

fn lower_expression_kind(
    expression: &Expr,
    semantics: &SemanticModel,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> TypedExpressionKind {
    if let Some(resolved_variant) = semantics.enum_variant(expression.id) {
        let (variant, payload) = match &expression.kind {
            ExprKind::Path(path) => {
                let [_, variant] = path.as_slice() else {
                    unreachable!("resolved enum paths have two segments")
                };
                (variant.clone(), None)
            }
            ExprKind::Call {
                callee,
                receiver,
                args,
                ..
            } => {
                debug_assert!(receiver.is_none());
                let [_, variant] = callee.as_slice() else {
                    unreachable!("resolved enum constructors have two segments")
                };
                (variant.clone(), args.first().map(|payload| payload.id))
            }
            _ => unreachable!("only enum-shaped syntax resolves an enum variant"),
        };
        return TypedExpressionKind::Enum {
            // The expression's published type may be the target of an implicit
            // wrapper conversion (for example, returning `Chapter.Village`
            // from a `Chapter?` function). The resolved variant remains the
            // canonical identity of the enum value being constructed.
            enumeration: enum_type_for_variant(resolved_variant, syntax, standard_library),
            variant,
            payload,
        };
    }
    match &expression.kind {
        ExprKind::Error => unreachable!("recovery expressions cannot reach typed HIR"),
        ExprKind::None => TypedExpressionKind::None,
        ExprKind::Bool(value) => TypedExpressionKind::Bool(*value),
        ExprKind::Int { value, suffix } => TypedExpressionKind::Int {
            value: *value,
            suffix: *suffix,
        },
        ExprKind::Float(value) => TypedExpressionKind::Float(*value),
        ExprKind::String(value) => TypedExpressionKind::String(value.clone()),
        ExprKind::InterpolatedString(parts) => TypedExpressionKind::InterpolatedString(
            parts
                .iter()
                .map(|part| match part {
                    InterpolatedPart::Text(value) => TypedInterpolatedPart::Text(value.clone()),
                    InterpolatedPart::Expr(expression) => {
                        let source = semantics
                            .expression_type(expression.id)
                            .expect("interpolation operands have resolved types");
                        let conversion = (!matches!(
                            semantics.types().kind(source),
                            TypeKind::Standard(StdlibTypeId::String)
                        ))
                        .then_some(ImplicitConversion::ToString { source });
                        TypedInterpolatedPart::Expression {
                            expression: expression.id,
                            conversion,
                        }
                    }
                })
                .collect(),
        ),
        ExprKind::Signature(value) => TypedExpressionKind::Signature(value.clone()),
        ExprKind::Array(elements) => {
            TypedExpressionKind::Array(elements.iter().map(|element| element.id).collect())
        }
        ExprKind::Record { fields, .. } => TypedExpressionKind::Record {
            record: semantics
                .record_literal(expression.id)
                .expect("checked record literals resolve their nominal declaration"),
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), value.id))
                .collect(),
        },
        ExprKind::Match { value, arms } => TypedExpressionKind::Match {
            value: value.id,
            arms: arms
                .iter()
                .map(|arm| TypedMatchArm {
                    pattern: match &arm.pattern {
                        MatchPattern::Enum {
                            variant, binding, ..
                        } => TypedPattern::Enum {
                            enumeration: enum_type_for_variant(
                                semantics
                                    .pattern_variant(arm.pattern_id)
                                    .expect("typed enum patterns have resolved variants"),
                                syntax,
                                standard_library,
                            ),
                            variant: variant.clone(),
                            binding: binding.clone(),
                        },
                        MatchPattern::Bool(value) => TypedPattern::Bool(*value),
                        MatchPattern::Int { value, suffix } => TypedPattern::Int {
                            value: *value,
                            suffix: *suffix,
                        },
                        MatchPattern::None => TypedPattern::None,
                        MatchPattern::OptionSome(binding) => {
                            TypedPattern::OptionSome(binding.clone())
                        }
                        MatchPattern::ResultSuccess(binding) => {
                            TypedPattern::ResultSuccess(binding.clone())
                        }
                        MatchPattern::ResultError(binding) => {
                            TypedPattern::ResultError(binding.clone())
                        }
                        MatchPattern::Wildcard => TypedPattern::Wildcard,
                    },
                    resolution: ResolvedPattern {
                        id: arm.pattern_id,
                        variant: semantics.pattern_variant(arm.pattern_id),
                        wrapper: semantics.wrapper_pattern(arm.pattern_id),
                        span: arm.span,
                    },
                    guard: arm.guard.as_ref().map(|guard| guard.id),
                    value: arm.value.id,
                    span: arm.span,
                })
                .collect(),
        },
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => TypedExpressionKind::If {
            condition: condition.id,
            then_expr: then_expr.id,
            else_expr: else_expr.id,
        },
        ExprKind::Fallback { value, fallback } => TypedExpressionKind::Fallback {
            value: value.id,
            fallback: match fallback {
                FallbackBranch::Value(fallback) => TypedFallbackBranch::Value(fallback.id),
                FallbackBranch::Return { value, .. } => {
                    TypedFallbackBranch::Return(value.as_ref().map(|value| value.id))
                }
                FallbackBranch::Break { .. } => TypedFallbackBranch::Break,
                FallbackBranch::Continue { .. } => TypedFallbackBranch::Continue,
            },
        },
        ExprKind::Suspend {
            mode,
            destination,
            value,
        } => TypedExpressionKind::Suspend {
            mode: *mode,
            destination: *destination,
            value: value.id,
        },
        ExprKind::Propagate(value) => TypedExpressionKind::Propagate {
            value: value.id,
            target: semantics
                .propagation_target(expression.id)
                .expect("checked propagation expressions have a failure boundary"),
        },
        ExprKind::Path(path) => TypedExpressionKind::Path(path.clone()),
        ExprKind::Member {
            receiver,
            name,
            name_span,
        } => TypedExpressionKind::Member {
            receiver: receiver.id,
            name: name.clone(),
            name_span: *name_span,
        },
        ExprKind::Unary { op, expr } => TypedExpressionKind::Unary {
            op: *op,
            expression: expr.id,
        },
        ExprKind::Cast { expr, target } => TypedExpressionKind::Cast {
            expression: expr.id,
            target: *target,
        },
        ExprKind::Binary { op, left, right } => TypedExpressionKind::Binary {
            op: *op,
            left: left.id,
            right: right.id,
        },
        ExprKind::Call {
            callee,
            receiver,
            args,
            ..
        } => TypedExpressionKind::Call {
            source_path: callee.clone(),
            receiver: receiver.as_ref().map(|receiver| receiver.id),
            arguments: args.iter().map(|argument| argument.id).collect(),
        },
    }
}

fn enum_type_for_variant(
    variant: ResolvedEnumVariantId,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> EnumTypeId {
    match variant {
        ResolvedEnumVariantId::Source(variant) => EnumTypeId::Source(
            syntax
                .enum_declarations()
                .find(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|candidate| candidate.id == variant)
                })
                .expect("resolved source variants belong to a source enum")
                .id,
        ),
        ResolvedEnumVariantId::Standard(variant) => {
            EnumTypeId::Standard(standard_library.variant(variant).owner)
        }
    }
}

fn lower_block(
    block: &Block,
    semantics: &SemanticModel,
    failure_boundary: Option<TypeId>,
) -> TypedBlock {
    TypedBlock {
        statements: block
            .statements
            .iter()
            .map(|source_statement| {
                let (statement, debug_only, span) = match source_statement {
                    Stmt::Debug { statement, span } => (statement.as_ref(), true, *span),
                    statement => {
                        let span = match statement {
                            Stmt::Variable(variable) => variable.span,
                            Stmt::Assign { span, .. }
                            | Stmt::If { span, .. }
                            | Stmt::While { span, .. }
                            | Stmt::For { span, .. }
                            | Stmt::Break { span }
                            | Stmt::Continue { span }
                            | Stmt::Return { span, .. }
                            | Stmt::Throw { span, .. }
                            | Stmt::Suspend { span, .. } => *span,
                            Stmt::Expression(expression) => expression.span,
                            Stmt::Debug { .. } => {
                                unreachable!("nested debug modifiers are rejected during checking")
                            }
                        };
                        (statement, false, span)
                    }
                };
                TypedStatement {
                    kind: match statement {
                        Stmt::Debug { .. } => {
                            unreachable!(
                                "debug modifiers are unwrapped into typed statement metadata"
                            )
                        }
                        Stmt::Variable(variable) => TypedStatementKind::Variable {
                            value: variable.id,
                            initializer: variable.value.id,
                        },
                        Stmt::Assign {
                            id,
                            op,
                            value,
                            span,
                            ..
                        } => TypedStatementKind::Assign {
                            assignment: ResolvedAssignment {
                                id: *id,
                                target: semantics
                                    .assignment_target(*id)
                                    .expect("checked assignments have resolved targets"),
                                operator: semantics.assignment_call(*id).cloned(),
                                span: *span,
                            },
                            op: *op,
                            value: value.id,
                        },
                        Stmt::If {
                            condition,
                            then_block,
                            else_block,
                            ..
                        } => TypedStatementKind::If {
                            condition: condition.id,
                            then_block: lower_block(then_block, semantics, failure_boundary),
                            else_block: else_block
                                .as_ref()
                                .map(|block| lower_block(block, semantics, failure_boundary)),
                        },
                        Stmt::While {
                            condition, body, ..
                        } => TypedStatementKind::While {
                            condition: condition.id,
                            body: lower_block(body, semantics, failure_boundary),
                        },
                        Stmt::For {
                            binding,
                            iterable_value,
                            index_value,
                            iterable,
                            body,
                            ..
                        } => TypedStatementKind::For {
                            binding: binding.id,
                            iterable_value: *iterable_value,
                            index_value: *index_value,
                            iterable: iterable.id,
                            body: lower_block(body, semantics, failure_boundary),
                        },
                        Stmt::Break { .. } => TypedStatementKind::Break,
                        Stmt::Continue { .. } => TypedStatementKind::Continue,
                        Stmt::Return { value, .. } => {
                            TypedStatementKind::Return(value.as_ref().map(|value| value.id))
                        }
                        Stmt::Throw { error, .. } => TypedStatementKind::Throw {
                            error: error.id,
                            target: failure_boundary
                                .expect("checked throw statements have a failure boundary"),
                        },
                        Stmt::Suspend {
                            mode,
                            binding,
                            returns,
                            value,
                            ..
                        } => TypedStatementKind::Suspend {
                            mode: *mode,
                            binding: binding.as_ref().map(|binding| binding.id),
                            returns: *returns,
                            value: value.id,
                        },
                        Stmt::Expression(expression) => {
                            TypedStatementKind::Expression(expression.id)
                        }
                    },
                    debug_only,
                    span,
                }
            })
            .collect(),
        span: block.span,
    }
}
