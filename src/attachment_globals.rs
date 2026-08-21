//! Attachment-scoped global lifetime and initialization analysis.
//!
//! A bare top-level `let` is backed by an ordinary mutable Wasm global, but
//! its source lifetime begins only when `onAttach` initializes it and ends
//! when that process detaches. This pass is the semantic authority for which
//! named state layouts establish each value. Backend defaults must never
//! become observable source values.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use crate::{
    Diagnostic,
    ast::{ActionKind, EnumVariantId, FunctionId, Program, Span, ValueId},
    hir::{TypedBlock, TypedExpressionKind, TypedProgram, TypedStatementKind},
    semantic::{ResolvedCall, ResolvedEnumVariantId, ResolvedValue, SemanticModel},
    stdlib::CoreTypeId,
    types::TypeKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentLayout {
    Single,
    Named(EnumVariantId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttachmentGlobalAnalysis {
    globals: HashSet<ValueId>,
    layouts: Vec<AttachmentLayout>,
    available_in: HashMap<ValueId, HashSet<AttachmentLayout>>,
    function_layouts: HashMap<FunctionId, HashSet<AttachmentLayout>>,
}

impl AttachmentGlobalAnalysis {
    pub fn is_attachment_global(&self, value: ValueId) -> bool {
        self.globals.contains(&value)
    }

    pub fn layouts(&self) -> &[AttachmentLayout] {
        &self.layouts
    }

    pub fn is_available_in(&self, value: ValueId, layout: AttachmentLayout) -> bool {
        self.available_in
            .get(&value)
            .is_some_and(|layouts| layouts.contains(&layout))
    }

    pub fn available_layouts(&self, value: ValueId) -> impl Iterator<Item = AttachmentLayout> + '_ {
        self.available_in.get(&value).into_iter().flatten().copied()
    }

    /// Layouts in which a helper's attachment-global preconditions hold.
    /// An empty result means the helper has no valid attached call context.
    pub fn function_layouts(
        &self,
        function: FunctionId,
    ) -> impl Iterator<Item = AttachmentLayout> + '_ {
        self.function_layouts
            .get(&function)
            .into_iter()
            .flatten()
            .copied()
    }
}

#[derive(Clone)]
struct EvalPath {
    assigned: HashSet<ValueId>,
    layouts: Option<HashSet<EnumVariantId>>,
}

impl EvalPath {
    fn new(assigned: HashSet<ValueId>) -> Self {
        Self {
            assigned,
            layouts: None,
        }
    }
}

#[derive(Default)]
struct Flow {
    normal: Vec<EvalPath>,
    returned: Vec<EvalPath>,
}

struct Initializer<'a> {
    hir: &'a TypedProgram,
    semantics: &'a SemanticModel,
    globals: &'a HashSet<ValueId>,
    debug_globals: HashSet<ValueId>,
    layout_variants: HashSet<EnumVariantId>,
    requirements: &'a HashMap<FunctionId, FunctionRequirements>,
    uninitialized_reads: RefCell<HashSet<(ValueId, Span)>>,
}

pub(crate) fn analyze(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> (AttachmentGlobalAnalysis, Vec<Diagnostic>) {
    let globals = hir.attachment_globals().collect::<HashSet<_>>();
    let layouts = syntax.state.as_ref().map_or_else(
        || vec![AttachmentLayout::Single],
        |state| {
            if state.layouts.is_empty() {
                vec![AttachmentLayout::Single]
            } else {
                state
                    .layouts
                    .iter()
                    .map(|layout| AttachmentLayout::Named(layout.variant))
                    .collect()
            }
        },
    );
    let mut analysis = AttachmentGlobalAnalysis {
        globals: globals.clone(),
        layouts,
        available_in: HashMap::new(),
        function_layouts: HashMap::new(),
    };
    if globals.is_empty() {
        return (analysis, Vec::new());
    }

    let Some(on_attach) = hir.action_body(ActionKind::OnAttach) else {
        let diagnostics = syntax
            .globals
            .iter()
            .filter(|global| global.value.is_none())
            .map(|global| {
                Diagnostic::semantic(
                    format!(
                        "attachment-scoped global `{}` requires an `onAttach` block",
                        global.name
                    ),
                    global.name_span,
                )
                .with_primary_label("this value has no attachment-time initializer")
            })
            .collect();
        return (analysis, diagnostics);
    };

    let requirements = infer_function_requirements(syntax, hir, &analysis);
    let layout_variants = analysis
        .layouts
        .iter()
        .filter_map(|layout| match layout {
            AttachmentLayout::Named(variant) => Some(*variant),
            AttachmentLayout::Single => None,
        })
        .collect();
    let initializer = Initializer {
        hir,
        semantics,
        globals: &globals,
        debug_globals: hir
            .attachment_globals_with_debug()
            .filter_map(|(value, debug_only)| debug_only.then_some(value))
            .collect(),
        layout_variants,
        requirements: &requirements,
        uninitialized_reads: RefCell::new(HashSet::new()),
    };
    let mut flow = initializer.eval_block(on_attach, vec![EvalPath::new(HashSet::new())]);
    if analysis.layouts == [AttachmentLayout::Single] {
        // Explicit empty returns and ordinary fallthrough both complete a
        // single-layout attachment successfully.
        flow.returned.append(&mut flow.normal);
        record_availability(
            &mut analysis.available_in,
            AttachmentLayout::Single,
            &flow.returned,
            &globals,
        );
    } else {
        for layout in analysis.layouts.iter().copied() {
            let AttachmentLayout::Named(variant) = layout else {
                unreachable!()
            };
            let paths = flow
                .returned
                .iter()
                .filter(|path| {
                    path.layouts
                        .as_ref()
                        .is_none_or(|variants| variants.contains(&variant))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                record_availability(&mut analysis.available_in, layout, &paths, &globals);
            }
        }
    }

    let assigned_on_any_return = flow
        .returned
        .iter()
        .flat_map(|path| path.assigned.iter().copied())
        .collect::<HashSet<_>>();
    let declarations = syntax
        .globals
        .iter()
        .filter(|global| global.value.is_none())
        .map(|global| (global.id, global))
        .collect::<HashMap<_, _>>();
    let mut diagnostics: Vec<Diagnostic> = globals
        .iter()
        .filter(|global| {
            analysis
                .available_in
                .get(global)
                .is_none_or(HashSet::is_empty)
        })
        .filter_map(|global| declarations.get(global))
        .map(|global| {
            let (message, label) = if assigned_on_any_return.contains(&global.id) {
                (
                    format!(
                        "attachment-scoped global `{}` is not initialized on every successful `onAttach` path",
                        global.name
                    ),
                    "assign this value on every path that completes the corresponding attachment layout",
                )
            } else {
                (
                    format!(
                        "attachment-scoped global `{}` is never initialized by `onAttach`",
                        global.name
                    ),
                    "assign this value before completing a supported attachment",
                )
            };
            Diagnostic::semantic(
                message,
                global.name_span,
            )
            .with_primary_label(label)
            .with_secondary_label(on_attach.span, "attachment initialization happens here")
        })
        .collect();
    for (value, span) in initializer.uninitialized_reads.into_inner() {
        let Some(global) = declarations.get(&value) else {
            continue;
        };
        diagnostics.push(
            Diagnostic::semantic(
                format!(
                    "attachment-scoped global `{}` may be read before it is initialized",
                    global.name
                ),
                span,
            )
            .with_primary_label("this path has not assigned the value yet")
            .with_secondary_label(
                global.name_span,
                "the attachment-scoped global is declared here",
            ),
        );
    }
    analysis.function_layouts = requirements
        .iter()
        .map(|(function, facts)| {
            let valid = analysis
                .layouts
                .iter()
                .copied()
                .filter(|layout| {
                    facts
                        .globals
                        .iter()
                        .all(|global| analysis.is_available_in(*global, *layout))
                })
                .collect();
            (*function, valid)
        })
        .collect();
    diagnostics.extend(validate_uses(syntax, hir, &analysis, &requirements));
    (analysis, diagnostics)
}

fn record_availability(
    availability: &mut HashMap<ValueId, HashSet<AttachmentLayout>>,
    layout: AttachmentLayout,
    paths: &[EvalPath],
    globals: &HashSet<ValueId>,
) {
    let mut definitely_assigned = globals.clone();
    for path in paths {
        definitely_assigned.retain(|global| path.assigned.contains(global));
    }
    for global in definitely_assigned {
        availability.entry(global).or_default().insert(layout);
    }
}

impl Initializer<'_> {
    fn eval_block(&self, block: &TypedBlock, mut normal: Vec<EvalPath>) -> Flow {
        let mut returned = Vec::new();
        for statement in &block.statements {
            if normal.is_empty() {
                break;
            }
            if statement.debug_only {
                // Debug statements disappear in release builds. Their writes
                // may establish debug-only attachment globals, but must never
                // establish a release-visible value or alter release control
                // flow. Reads are still analyzed against the incoming path.
                let debug_flow = self.eval_statement(statement, normal.clone());
                let debug_normal = collapse_paths(debug_flow.normal);
                if let Some(debug_path) = debug_normal.first() {
                    for path in &mut normal {
                        path.assigned.extend(
                            debug_path
                                .assigned
                                .iter()
                                .filter(|value| self.debug_globals.contains(value))
                                .copied(),
                        );
                    }
                }
                continue;
            }
            let flow = self.eval_statement(statement, normal);
            normal = collapse_paths(flow.normal);
            returned.extend(flow.returned);
        }
        Flow { normal, returned }
    }

    fn eval_statement(&self, statement: &crate::hir::TypedStatement, input: Vec<EvalPath>) -> Flow {
        match &statement.kind {
            TypedStatementKind::Variable { initializer, .. }
            | TypedStatementKind::Expression(initializer) => self.eval_expr(*initializer, input),
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                if op.is_some() && self.globals.contains(&assignment.target) {
                    self.record_uninitialized(assignment.target, assignment.span, &input);
                }
                let mut flow = self.eval_expr(*value, input);
                if op.is_none() && self.globals.contains(&assignment.target) {
                    for path in &mut flow.normal {
                        path.assigned.insert(assignment.target);
                    }
                }
                flow
            }
            TypedStatementKind::StateAssign { target, value, .. }
            | TypedStatementKind::IndexAssign { target, value, .. } => {
                self.eval_sequence(&[*target, *value], input)
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                let mut normal = Vec::new();
                for path in condition.normal {
                    let then_flow = self.eval_block(then_block, vec![path.clone()]);
                    normal.extend(then_flow.normal);
                    returned.extend(then_flow.returned);
                    if let Some(else_block) = else_block {
                        let else_flow = self.eval_block(else_block, vec![path]);
                        normal.extend(else_flow.normal);
                        returned.extend(else_flow.returned);
                    } else {
                        normal.push(path);
                    }
                }
                Flow { normal, returned }
            }
            TypedStatementKind::While { condition, body } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                for path in &condition.normal {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                Flow {
                    normal: condition.normal,
                    returned,
                }
            }
            TypedStatementKind::For { iterable, body, .. } => {
                let iterable = self.eval_expr(*iterable, input);
                let mut returned = iterable.returned;
                for path in &iterable.normal {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                Flow {
                    normal: iterable.normal,
                    returned,
                }
            }
            TypedStatementKind::Suspend { value, returns, .. } => {
                let mut flow = self.eval_expr(*value, input);
                if *returns {
                    flow.returned.append(&mut flow.normal);
                }
                flow
            }
        }
    }

    fn eval_expr(&self, id: crate::ast::ExprId, input: Vec<EvalPath>) -> Flow {
        let expression = self
            .hir
            .expression(id)
            .expect("attachment analysis only visits typed expressions");
        if let Some((Some(ResolvedValue::Variable(value)), _)) = self.hir.value_path(id)
            && self.globals.contains(&value)
        {
            self.record_uninitialized(value, expression.span, &input);
        }
        if let Some(call) = self.hir.call(id) {
            if let Some(value) = call
                .receiver()
                .and_then(|receiver| receiver.path().map(|(root, _)| root))
                .and_then(ResolvedValue::source_value)
                && self.globals.contains(&value)
            {
                self.record_uninitialized(value, expression.span, &input);
            }
            if let ResolvedCall::UserFunction { function, .. }
            | ResolvedCall::UserMethod { function, .. } = call
                && let Some(requirements) = self.requirements.get(function)
            {
                for value in &requirements.globals {
                    self.record_uninitialized(*value, expression.span, &input);
                }
            }
        }
        let mut flow = match &expression.kind {
            TypedExpressionKind::Enum { payload, .. } => {
                let mut flow = if let Some(payload) = payload {
                    self.eval_expr(*payload, input)
                } else {
                    Flow {
                        normal: input,
                        returned: Vec::new(),
                    }
                };
                if let Some(ResolvedEnumVariantId::Source(variant)) = self.hir.enum_variant(id)
                    && self.layout_variants.contains(&variant)
                {
                    for path in &mut flow.normal {
                        path.layouts = Some(HashSet::from([variant]));
                    }
                }
                flow
            }
            TypedExpressionKind::Block { statements, value } => {
                let mut flow = self.eval_block(statements, input);
                if let Some(value) = value {
                    let tail = self.eval_expr(*value, flow.normal);
                    flow.normal = tail.normal;
                    flow.returned.extend(tail.returned);
                }
                flow
            }
            TypedExpressionKind::Match { value, arms } => {
                let subject = self.eval_expr(*value, input);
                let mut returned = subject.returned;
                let mut normal = Vec::new();
                for path in subject.normal {
                    for arm in arms {
                        let guard = arm.guard.map_or_else(
                            || Flow {
                                normal: vec![path.clone()],
                                returned: Vec::new(),
                            },
                            |guard| self.eval_expr(guard, vec![path.clone()]),
                        );
                        returned.extend(guard.returned);
                        let arm_flow = self.eval_expr(arm.value, guard.normal);
                        normal.extend(arm_flow.normal);
                        returned.extend(arm_flow.returned);
                    }
                }
                Flow { normal, returned }
            }
            TypedExpressionKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                let mut normal = Vec::new();
                for path in condition.normal {
                    let then_flow = self.eval_expr(*then_expr, vec![path.clone()]);
                    normal.extend(then_flow.normal);
                    returned.extend(then_flow.returned);
                    let else_flow = self.eval_expr(*else_expr, vec![path]);
                    normal.extend(else_flow.normal);
                    returned.extend(else_flow.returned);
                }
                Flow { normal, returned }
            }
            TypedExpressionKind::Fallback { value, fallback } => {
                let value_flow = self.eval_expr(*value, input.clone());
                let fallback_flow = self.eval_expr(*fallback, input);
                Flow {
                    normal: value_flow
                        .normal
                        .into_iter()
                        .chain(fallback_flow.normal)
                        .collect(),
                    returned: value_flow
                        .returned
                        .into_iter()
                        .chain(fallback_flow.returned)
                        .collect(),
                }
            }
            TypedExpressionKind::Return(value) => {
                let mut flow = if let Some(value) = value {
                    self.eval_expr(*value, input)
                } else {
                    Flow {
                        normal: input,
                        returned: Vec::new(),
                    }
                };
                for path in &mut flow.normal {
                    if path.layouts.is_none() && !self.layout_variants.is_empty() {
                        path.layouts = Some(self.layout_variants.clone());
                    }
                }
                flow.returned.append(&mut flow.normal);
                flow
            }
            TypedExpressionKind::Break(_)
            | TypedExpressionKind::Continue
            | TypedExpressionKind::Throw { .. } => {
                let children = match &expression.kind {
                    TypedExpressionKind::Break(Some(value)) => vec![*value],
                    TypedExpressionKind::Throw { error, .. } => vec![*error],
                    _ => Vec::new(),
                };
                let mut flow = self.eval_sequence(&children, input);
                flow.normal.clear();
                flow
            }
            TypedExpressionKind::Loop { body } => {
                let mut returned = Vec::new();
                for path in &input {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                // Conservative until value-producing breaks are explicit CFG
                // edges in typed HIR.
                Flow {
                    normal: input,
                    returned,
                }
            }
            _ => self.eval_sequence(&expression_children(&expression.kind), input),
        };
        if matches!(
            self.semantics.types().kind(expression.ty),
            TypeKind::Builtin(CoreTypeId::Never)
        ) {
            // `await process.closed()`, a divergent value block, and any
            // future source-defined `Never` expression terminate this path.
            // Keep this semantic rather than enumerating terminal syntax.
            flow.normal.clear();
        }
        flow
    }

    fn eval_sequence(&self, children: &[crate::ast::ExprId], input: Vec<EvalPath>) -> Flow {
        let mut normal = input;
        let mut returned = Vec::new();
        for child in children {
            let flow = self.eval_expr(*child, normal);
            normal = flow.normal;
            returned.extend(flow.returned);
            if normal.is_empty() {
                break;
            }
        }
        for path in &mut normal {
            path.layouts = None;
        }
        Flow { normal, returned }
    }

    fn record_uninitialized(&self, value: ValueId, span: Span, paths: &[EvalPath]) {
        if paths.iter().any(|path| !path.assigned.contains(&value)) {
            self.uninitialized_reads.borrow_mut().insert((value, span));
        }
    }
}

fn collapse_paths(paths: Vec<EvalPath>) -> Vec<EvalPath> {
    let Some(first) = paths.first() else {
        return Vec::new();
    };
    let mut assigned = first.assigned.clone();
    for path in &paths[1..] {
        assigned.retain(|global| path.assigned.contains(global));
    }
    vec![EvalPath::new(assigned)]
}

#[derive(Clone, Default)]
struct FunctionRequirements {
    globals: HashSet<ValueId>,
    callees: HashSet<FunctionId>,
}

trait AttachmentUseVisitor {
    fn global(&mut self, value: ValueId, span: Span, refined: Option<AttachmentLayout>);
    fn global_write(&mut self, value: ValueId, span: Span, refined: Option<AttachmentLayout>);
    fn call(&mut self, function: FunctionId, span: Span, refined: Option<AttachmentLayout>);
}

fn infer_function_requirements(
    _syntax: &Program,
    hir: &TypedProgram,
    analysis: &AttachmentGlobalAnalysis,
) -> HashMap<FunctionId, FunctionRequirements> {
    struct Collector<'a> {
        analysis: &'a AttachmentGlobalAnalysis,
        facts: FunctionRequirements,
    }
    impl AttachmentUseVisitor for Collector<'_> {
        fn global(&mut self, value: ValueId, _span: Span, refined: Option<AttachmentLayout>) {
            if refined.is_none() && self.analysis.is_attachment_global(value) {
                self.facts.globals.insert(value);
            }
        }

        fn global_write(&mut self, value: ValueId, _span: Span, refined: Option<AttachmentLayout>) {
            if refined.is_none() && self.analysis.is_attachment_global(value) {
                self.facts.globals.insert(value);
            }
        }

        fn call(&mut self, function: FunctionId, _span: Span, refined: Option<AttachmentLayout>) {
            if refined.is_none() {
                self.facts.callees.insert(function);
            }
        }
    }

    let layout_value = hir
        .declarations()
        .declarations_named("layout")
        .find_map(|declaration| match declaration.id {
            crate::hir::DeclarationId::Global(value) => Some(value),
            _ => None,
        });
    let mut requirements = HashMap::new();
    for function in hir.function_bodies() {
        let mut collector = Collector {
            analysis,
            facts: FunctionRequirements::default(),
        };
        walk_block(&mut collector, &function.body, hir, layout_value, None);
        requirements.insert(function.function.function, collector.facts);
    }

    // A helper inherits every attachment value needed by a helper it calls.
    // This is the same viral shape as process effects, but retains the exact
    // globals so `onAttach` can eventually prove call-site initialization.
    loop {
        let previous = requirements.clone();
        let mut changed = false;
        for facts in requirements.values_mut() {
            for callee in facts.callees.clone() {
                if let Some(callee) = previous.get(&callee) {
                    let old_len = facts.globals.len();
                    facts.globals.extend(callee.globals.iter().copied());
                    changed |= facts.globals.len() != old_len;
                }
            }
        }
        if !changed {
            break;
        }
    }
    requirements
}

fn validate_uses(
    syntax: &Program,
    hir: &TypedProgram,
    analysis: &AttachmentGlobalAnalysis,
    requirements: &HashMap<FunctionId, FunctionRequirements>,
) -> Vec<Diagnostic> {
    struct Validator<'a> {
        syntax: &'a Program,
        analysis: &'a AttachmentGlobalAnalysis,
        requirements: &'a HashMap<FunctionId, FunctionRequirements>,
        base: Vec<AttachmentLayout>,
        detached_action: Option<ActionKind>,
        diagnostics: Vec<Diagnostic>,
        seen: HashSet<(usize, usize, Option<ValueId>, Option<FunctionId>)>,
    }

    impl Validator<'_> {
        fn active(&self, refined: Option<AttachmentLayout>) -> Vec<AttachmentLayout> {
            refined.map_or_else(|| self.base.clone(), |layout| vec![layout])
        }

        fn global_name(&self, value: ValueId) -> &str {
            self.syntax
                .globals
                .iter()
                .find(|global| global.id == value)
                .map_or("attachment value", |global| global.name.as_str())
        }

        fn global_declaration_span(&self, value: ValueId) -> Option<Span> {
            self.syntax
                .globals
                .iter()
                .find(|global| global.id == value)
                .map(|global| global.name_span)
        }

        fn invalid_layouts(
            &self,
            value: ValueId,
            refined: Option<AttachmentLayout>,
        ) -> Vec<AttachmentLayout> {
            self.active(refined)
                .into_iter()
                .filter(|layout| !self.analysis.is_available_in(value, *layout))
                .collect()
        }

        fn layout_names(&self, layouts: &[AttachmentLayout]) -> String {
            let names = layouts
                .iter()
                .map(|layout| match layout {
                    AttachmentLayout::Single => "the attachment".to_owned(),
                    AttachmentLayout::Named(variant) => self
                        .syntax
                        .state
                        .as_ref()
                        .and_then(|state| state.layout_enum.as_ref())
                        .and_then(|enumeration| {
                            enumeration
                                .variants
                                .iter()
                                .find(|candidate| candidate.id == *variant)
                        })
                        .map_or_else(
                            || "an unknown layout".to_owned(),
                            |variant| format!("`StateLayout.{}`", variant.name),
                        ),
                })
                .collect::<Vec<_>>();
            match names.as_slice() {
                [] => String::new(),
                [one] => one.clone(),
                _ => names.join(", "),
            }
        }

        fn validate_global_access(
            &mut self,
            value: ValueId,
            span: Span,
            refined: Option<AttachmentLayout>,
            access: &str,
        ) {
            if !self.analysis.is_attachment_global(value) {
                return;
            }
            if let Some(action) = self.detached_action {
                if self.seen.insert((span.start, span.end, Some(value), None)) {
                    let name = self.global_name(value).to_owned();
                    let mut diagnostic = Diagnostic::semantic(
                        format!(
                            "attachment-scoped global `{name}` is unavailable in `{}`",
                            action.name()
                        ),
                        span,
                    )
                    .with_primary_label(format!(
                        "this {access} occurs without an attached process"
                    ));
                    if let Some(declaration) = self.global_declaration_span(value) {
                        diagnostic = diagnostic.with_secondary_label(
                            declaration,
                            "the attachment-scoped global is declared here",
                        );
                    }
                    self.diagnostics.push(diagnostic);
                }
                return;
            }
            let invalid = self.invalid_layouts(value, refined);
            if invalid.is_empty() || !self.seen.insert((span.start, span.end, Some(value), None)) {
                return;
            }
            let name = self.global_name(value).to_owned();
            let mut diagnostic = Diagnostic::semantic(
                format!(
                    "attachment-scoped global `{name}` is not initialized for {}",
                    self.layout_names(&invalid)
                ),
                span,
            )
            .with_primary_label(format!(
                "this {access} is not valid on every path that reaches it"
            ));
            if let Some(declaration) = self.global_declaration_span(value) {
                diagnostic = diagnostic.with_secondary_label(
                    declaration,
                    "the attachment-scoped global is declared here",
                );
            }
            self.diagnostics.push(diagnostic);
        }
    }

    impl AttachmentUseVisitor for Validator<'_> {
        fn global(&mut self, value: ValueId, span: Span, refined: Option<AttachmentLayout>) {
            self.validate_global_access(value, span, refined, "read");
        }

        fn global_write(&mut self, value: ValueId, span: Span, refined: Option<AttachmentLayout>) {
            self.validate_global_access(value, span, refined, "write");
        }

        fn call(&mut self, function: FunctionId, span: Span, refined: Option<AttachmentLayout>) {
            let Some(facts) = self.requirements.get(&function) else {
                return;
            };
            let invalid = self
                .active(refined)
                .into_iter()
                .filter(|layout| {
                    facts
                        .globals
                        .iter()
                        .any(|global| !self.analysis.is_available_in(*global, *layout))
                })
                .collect::<Vec<_>>();
            if invalid.is_empty()
                || !self
                    .seen
                    .insert((span.start, span.end, None, Some(function)))
            {
                return;
            }
            let name = self
                .syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)
                .map_or("function", |candidate| candidate.name.as_str());
            let mut diagnostic = Diagnostic::semantic(
                format!(
                    "`{name}` requires attachment values unavailable for {}",
                    self.layout_names(&invalid)
                ),
                span,
            )
            .with_primary_label("refine `layout` before calling this helper");
            if let Some(declaration) = self
                .syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)
            {
                diagnostic = diagnostic.with_secondary_label(
                    declaration.name_span,
                    "the helper's attachment requirements originate here",
                );
            }
            self.diagnostics.push(diagnostic);
        }
    }

    let layout_value = syntax.state.as_ref().and_then(|state| state.layout_value);
    let mut diagnostics = Vec::new();
    for action in hir
        .action_bodies()
        .filter(|action| action.action != ActionKind::OnAttach)
    {
        let detached_action =
            (!crate::effects::action_has_attached_process(action.action)).then_some(action.action);
        let mut validator = Validator {
            syntax,
            analysis,
            requirements,
            base: analysis.layouts.clone(),
            detached_action,
            diagnostics: Vec::new(),
            seen: HashSet::new(),
        };
        walk_block(&mut validator, &action.body, hir, layout_value, None);
        diagnostics.extend(validator.diagnostics);
    }

    for function in hir.function_bodies() {
        let mut validator = Validator {
            syntax,
            analysis,
            requirements,
            base: analysis
                .function_layouts(function.function.function)
                .collect(),
            detached_action: None,
            diagnostics: Vec::new(),
            seen: HashSet::new(),
        };
        walk_block(&mut validator, &function.body, hir, layout_value, None);
        diagnostics.extend(validator.diagnostics);
    }

    if let Some(state) = &syntax.state {
        let field_layouts = if state.layouts.is_empty() {
            state
                .fields
                .iter()
                .map(|field| (field.id, AttachmentLayout::Single))
                .collect::<HashMap<_, _>>()
        } else {
            state
                .layouts
                .iter()
                .flat_map(|layout| {
                    layout
                        .fields
                        .iter()
                        .map(move |field| (field.id, AttachmentLayout::Named(layout.variant)))
                })
                .collect()
        };
        for (field, expression) in hir.state_sources() {
            let Some(layout) = field_layouts.get(&field).copied() else {
                continue;
            };
            let mut validator = Validator {
                syntax,
                analysis,
                requirements,
                base: vec![layout],
                detached_action: None,
                diagnostics: Vec::new(),
                seen: HashSet::new(),
            };
            walk_expression(&mut validator, expression, hir, layout_value, None);
            diagnostics.extend(validator.diagnostics);
        }
        for transform in hir.state_transforms() {
            let Some(layout) = field_layouts.get(&transform.field).copied() else {
                continue;
            };
            let mut validator = Validator {
                syntax,
                analysis,
                requirements,
                base: vec![layout],
                detached_action: None,
                diagnostics: Vec::new(),
                seen: HashSet::new(),
            };
            walk_expression(
                &mut validator,
                transform.expression,
                hir,
                layout_value,
                None,
            );
            diagnostics.extend(validator.diagnostics);
        }
    }
    diagnostics
}

fn walk_block(
    visitor: &mut impl AttachmentUseVisitor,
    block: &TypedBlock,
    hir: &TypedProgram,
    layout_value: Option<ValueId>,
    refined: Option<AttachmentLayout>,
) {
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Variable { initializer, .. }
            | TypedStatementKind::Expression(initializer) => {
                walk_expression(visitor, *initializer, hir, layout_value, refined)
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                if op.is_some() {
                    visitor.global(assignment.target, assignment.span, refined);
                } else {
                    visitor.global_write(assignment.target, assignment.span, refined);
                }
                walk_expression(visitor, *value, hir, layout_value, refined);
            }
            TypedStatementKind::StateAssign { target, value, .. }
            | TypedStatementKind::IndexAssign { target, value, .. } => {
                walk_expression(visitor, *target, hir, layout_value, refined);
                walk_expression(visitor, *value, hir, layout_value, refined);
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                walk_expression(visitor, *condition, hir, layout_value, refined);
                walk_block(visitor, then_block, hir, layout_value, refined);
                if let Some(else_block) = else_block {
                    walk_block(visitor, else_block, hir, layout_value, refined);
                }
            }
            TypedStatementKind::While { condition, body } => {
                walk_expression(visitor, *condition, hir, layout_value, refined);
                walk_block(visitor, body, hir, layout_value, refined);
            }
            TypedStatementKind::For { iterable, body, .. } => {
                walk_expression(visitor, *iterable, hir, layout_value, refined);
                walk_block(visitor, body, hir, layout_value, refined);
            }
            TypedStatementKind::Suspend { value, .. } => {
                walk_expression(visitor, *value, hir, layout_value, refined)
            }
        }
    }
}

fn walk_expression(
    visitor: &mut impl AttachmentUseVisitor,
    id: crate::ast::ExprId,
    hir: &TypedProgram,
    layout_value: Option<ValueId>,
    refined: Option<AttachmentLayout>,
) {
    let expression = hir
        .expression(id)
        .expect("attachment use analysis only visits typed expressions");
    if let Some((Some(ResolvedValue::Variable(value)), _)) = hir.value_path(id) {
        visitor.global(value, expression.span, refined);
    }
    if let Some(call) = hir.call(id) {
        if let Some(value) = call
            .receiver()
            .and_then(|receiver| receiver.path().map(|(root, _)| root))
            .and_then(ResolvedValue::source_value)
        {
            visitor.global(value, expression.span, refined);
        }
        if let ResolvedCall::UserFunction { function, .. }
        | ResolvedCall::UserMethod { function, .. } = call
        {
            visitor.call(*function, expression.span, refined);
        }
    }

    match &expression.kind {
        TypedExpressionKind::Block { statements, value } => {
            walk_block(visitor, statements, hir, layout_value, refined);
            if let Some(value) = value {
                walk_expression(visitor, *value, hir, layout_value, refined);
            }
        }
        TypedExpressionKind::Loop { body } => walk_block(visitor, body, hir, layout_value, refined),
        TypedExpressionKind::Match { value, arms }
            if layout_value.is_some_and(|layout| {
                hir.value_path(*value).and_then(|(root, _)| root)
                    == Some(ResolvedValue::Variable(layout))
            }) =>
        {
            walk_expression(visitor, *value, hir, layout_value, refined);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    walk_expression(visitor, guard, hir, layout_value, refined);
                }
                let arm_layout = match arm.resolution.variant {
                    Some(ResolvedEnumVariantId::Source(variant)) => {
                        Some(AttachmentLayout::Named(variant))
                    }
                    _ => refined,
                };
                walk_expression(visitor, arm.value, hir, layout_value, arm_layout);
            }
        }
        TypedExpressionKind::Match { value, arms } => {
            walk_expression(visitor, *value, hir, layout_value, refined);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    walk_expression(visitor, guard, hir, layout_value, refined);
                }
                walk_expression(visitor, arm.value, hir, layout_value, refined);
            }
        }
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            walk_expression(visitor, *condition, hir, layout_value, refined);
            walk_expression(visitor, *then_expr, hir, layout_value, refined);
            walk_expression(visitor, *else_expr, hir, layout_value, refined);
        }
        TypedExpressionKind::Fallback { value, fallback } => {
            walk_expression(visitor, *value, hir, layout_value, refined);
            walk_expression(visitor, *fallback, hir, layout_value, refined);
        }
        _ => {
            for child in expression_children(&expression.kind) {
                walk_expression(visitor, child, hir, layout_value, refined);
            }
        }
    }
}

fn expression_children(kind: &TypedExpressionKind) -> Vec<crate::ast::ExprId> {
    match kind {
        TypedExpressionKind::InterpolatedString(parts) => parts
            .iter()
            .filter_map(|part| match part {
                crate::hir::TypedInterpolatedPart::Expression { expression, .. } => {
                    Some(*expression)
                }
                crate::hir::TypedInterpolatedPart::Text(_) => None,
            })
            .collect(),
        TypedExpressionKind::Array(values) => values.clone(),
        TypedExpressionKind::Record { fields, .. } => {
            fields.iter().map(|(_, value)| *value).collect()
        }
        TypedExpressionKind::Enum { payload, .. } => payload.iter().copied().collect(),
        TypedExpressionKind::Break(value) | TypedExpressionKind::Return(value) => {
            value.iter().copied().collect()
        }
        TypedExpressionKind::Throw { error, .. }
        | TypedExpressionKind::Suspend { value: error, .. }
        | TypedExpressionKind::Propagate { value: error, .. }
        | TypedExpressionKind::Unary {
            expression: error, ..
        }
        | TypedExpressionKind::Cast {
            expression: error, ..
        } => vec![*error],
        TypedExpressionKind::Member { receiver, .. } => vec![*receiver],
        TypedExpressionKind::Index { receiver, index } => vec![*receiver, *index],
        TypedExpressionKind::Binary { left, right, .. } => vec![*left, *right],
        TypedExpressionKind::Call {
            receiver,
            arguments,
            ..
        } => receiver
            .iter()
            .copied()
            .chain(arguments.iter().copied())
            .collect(),
        TypedExpressionKind::None
        | TypedExpressionKind::Bool(_)
        | TypedExpressionKind::Int { .. }
        | TypedExpressionKind::Float(_)
        | TypedExpressionKind::Char(_)
        | TypedExpressionKind::String(_)
        | TypedExpressionKind::Signature(_)
        | TypedExpressionKind::Path(_)
        | TypedExpressionKind::Continue => Vec::new(),
        TypedExpressionKind::Block { .. }
        | TypedExpressionKind::Loop { .. }
        | TypedExpressionKind::Match { .. }
        | TypedExpressionKind::If { .. }
        | TypedExpressionKind::Fallback { .. } => {
            unreachable!("control-flow expressions are handled before child collection")
        }
    }
}
