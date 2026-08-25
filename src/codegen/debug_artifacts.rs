//! Profile-aware debugger metadata assembled only after Wasm indices are final.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
};

use gimli::{
    Encoding, Format, LineEncoding, LittleEndian,
    write::{
        Address, AttributeValue, DwarfUnit, EndianVec, Expression, LineProgram, LineString,
        Sections, UnitEntryId,
    },
};
use wasm_encoder::{Function, IndirectNameMap, NameMap, NameSection};

use super::{Type, imports::Abi};
use crate::{
    ast::{
        EnumDecl, Expr, ExprKind, FunctionDecl, MatchArm, MatchPattern, Program, Span,
        TypeApplicationId, ValueId,
    },
    semantic::{FunctionInstance, SemanticModel},
    stdlib::StandardLibrary,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

pub(super) struct DebugArtifactPlan {
    names: NameSection,
    dwarf: Vec<DebugSection>,
}

pub(super) struct DebugArtifactInputs<'a> {
    pub abi: &'a Abi,
    pub defined_functions: &'a [(u32, String)],
    pub recorder: &'a DebugRecorder,
    pub global_indices: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub program: &'a Program,
    pub source_name: &'a str,
    pub source: &'a str,
}

pub(super) struct DebugSection {
    pub name: &'static str,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct BodyMarker {
    function: u32,
    body_offset: u32,
    source: Span,
    kind: BodyMarkerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyMarkerKind {
    Source,
    Suspend,
    Resume,
}

impl BodyMarkerKind {
    const fn discriminator(self) -> u64 {
        match self {
            Self::Source => 0,
            Self::Suspend => 1,
            Self::Resume => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BodyRange {
    raw_body_start: u32,
    raw_body_length: u32,
}

#[derive(Debug, Clone, Copy)]
struct DebugVariable {
    function: u32,
    value: ValueId,
    local: u32,
    ty: Type,
    parameter: bool,
}

#[derive(Debug, Clone, Copy)]
struct DebugGlobal {
    value: ValueId,
    index: u32,
    ty: Type,
}

/// Interior-mutability is intentionally confined to debug emission. Normal
/// code generation stays unaware of line-table state and release compilation
/// never constructs this recorder.
#[derive(Default)]
pub(super) struct DebugRecorder {
    markers: RefCell<Vec<BodyMarker>>,
    bodies: RefCell<BTreeMap<u32, BodyRange>>,
    variables: RefCell<Vec<DebugVariable>>,
}

#[derive(Clone, Copy)]
pub(super) struct DebugEmission<'a> {
    pub recorder: &'a DebugRecorder,
    pub function: u32,
}

impl DebugEmission<'_> {
    pub fn mark(self, function: &Function, source: Option<Span>) {
        self.mark_kind(function, source, BodyMarkerKind::Source);
    }

    pub fn mark_suspend(self, function: &Function, source: Option<Span>) {
        self.mark_kind(function, source, BodyMarkerKind::Suspend);
    }

    pub fn mark_resume(self, function: &Function, source: Option<Span>) {
        self.mark_kind(function, source, BodyMarkerKind::Resume);
    }

    fn mark_kind(self, function: &Function, source: Option<Span>, kind: BodyMarkerKind) {
        let Some(source) = source else {
            return;
        };
        let body_offset = u32::try_from(function.byte_len())
            .expect("debuggable Wasm function bodies fit in 4 GiB");
        let mut markers = self.recorder.markers.borrow_mut();
        if markers.last().is_some_and(|marker| {
            marker.function == self.function && marker.body_offset == body_offset
        }) {
            // Recursive expression emission can visit multiple nested source
            // nodes before producing an instruction. Keep the outermost span,
            // which best represents the statement-level breakpoint.
            return;
        }
        markers.push(BodyMarker {
            function: self.function,
            body_offset,
            source,
            kind,
        });
    }
}

impl DebugRecorder {
    pub fn emission(&self, function: u32) -> DebugEmission<'_> {
        DebugEmission {
            recorder: self,
            function,
        }
    }

    pub fn register_body(&self, function: u32, raw_body_start: u32, raw_body_length: u32) {
        let previous = self.bodies.borrow_mut().insert(
            function,
            BodyRange {
                raw_body_start,
                raw_body_length,
            },
        );
        assert!(
            previous.is_none(),
            "Wasm function bodies are registered once"
        );
    }

    pub fn register_variable(
        &self,
        function: u32,
        value: ValueId,
        local: u32,
        ty: Type,
        parameter: bool,
    ) {
        if !ty.has_runtime_value() {
            return;
        }
        let mut variables = self.variables.borrow_mut();
        assert!(
            !variables
                .iter()
                .any(|variable| variable.function == function && variable.value == value),
            "source variables are registered once per Wasm function"
        );
        variables.push(DebugVariable {
            function,
            value,
            local,
            ty,
            parameter,
        });
    }
}

pub(super) fn set_function_name(set: TypeApplicationId, operation: &str) -> String {
    format!("__splitscript::set#{}::{operation}", set.index())
}

pub(super) fn user_function_name(
    name: &str,
    instance: &FunctionInstance,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    enums: &[EnumDecl],
) -> String {
    if instance.type_arguments.is_empty() {
        return name.to_owned();
    }
    let arguments = instance
        .type_arguments
        .iter()
        .map(|ty| type_name(*ty, program, semantics, standard_library, enums))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}<{arguments}>")
}

fn type_name(
    ty: TypeId,
    program: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    enums: &[EnumDecl],
) -> String {
    let nested = |ty| type_name(ty, program, semantics, standard_library, enums);
    match semantics.types().kind(ty) {
        TypeKind::Error => "<unknown>".to_owned(),
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Standard(standard) => standard_library.type_decl(*standard).name.to_owned(),
        TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
        TypeKind::SettingsView => "SettingsView".to_owned(),
        TypeKind::Record(id) => program
            .records
            .iter()
            .find(|record| record.id == *id)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| format!("record#{}", id.index())),
        TypeKind::Enum(id) => enums
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{}", id.index())),
        TypeKind::ManagedClass(id) => program
            .managed_class(*id)
            .map(|class| class.name.clone())
            .unwrap_or_else(|| format!("class#{}", id.index())),
        TypeKind::ManagedReference(id) => program
            .managed_class(*id)
            .map(|class| format!("{}.Ref", class.name))
            .unwrap_or_else(|| format!("class#{}.Ref", id.index())),
        TypeKind::GenericParameter { index, .. } => crate::types::generic_parameter_name(*index),
        TypeKind::Array {
            element, length, ..
        } => match length {
            Some(length) => format!("[{}; {length}]", nested(*element)),
            None => format!("[{}]", nested(*element)),
        },
        TypeKind::Option { value, .. } => format!("{}?", nested(*value)),
        TypeKind::Result { value, .. } => format!("{}!", nested(*value)),
        TypeKind::Async { value, .. } => format!("async {}", nested(*value)),
        TypeKind::Callable {
            parameters, result, ..
        } => {
            let parameters = parameters
                .iter()
                .map(|parameter| nested(*parameter))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({parameters}) -> {}", nested(*result))
        }
        TypeKind::Set { element, .. } => format!("Set<{}>", nested(*element)),
        TypeKind::Application {
            constructor,
            arguments,
            ..
        } => {
            let name = standard_library.type_constructor(*constructor).name;
            let arguments = arguments
                .iter()
                .map(|argument| nested(*argument))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}<{arguments}>")
        }
        TypeKind::Range { bound, kind, .. } => {
            let bound = nested(*bound);
            format!("{bound}{}{bound}", kind.operator())
        }
    }
}

#[derive(Debug, Clone)]
struct SourceVariable {
    name: String,
    name_span: Span,
    declaration_span: Span,
    scope_span: Span,
}

fn source_variables(program: &Program) -> HashMap<ValueId, SourceVariable> {
    #[derive(Default)]
    struct Collector {
        variables: HashMap<ValueId, SourceVariable>,
        scopes: Vec<Span>,
    }

    impl Collector {
        fn insert(
            &mut self,
            id: ValueId,
            name: &str,
            name_span: Span,
            declaration_span: Span,
            scope_span: Span,
        ) {
            let previous = self.variables.insert(
                id,
                SourceVariable {
                    name: name.to_owned(),
                    name_span,
                    declaration_span,
                    scope_span,
                },
            );
            debug_assert!(previous.is_none());
        }
    }

    impl<'ast> Visitor<'ast> for Collector {
        fn visit_function(&mut self, function: &'ast FunctionDecl) {
            self.scopes.push(function.body.span);
            visit::walk_function(self, function);
            self.scopes.pop();
        }

        fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
            if let Some(scope) = self.scopes.last().copied() {
                self.insert(
                    parameter.id,
                    &parameter.name,
                    parameter.name_span,
                    parameter.span,
                    scope,
                );
            }
            visit::walk_parameter(self, parameter);
        }

        fn visit_block(&mut self, block: &'ast crate::ast::Block) {
            self.scopes.push(block.span);
            visit::walk_block(self, block);
            self.scopes.pop();
        }

        fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
            if let Some(scope) = self.scopes.last().copied() {
                self.insert(
                    variable.id,
                    &variable.name,
                    variable.name_span,
                    variable.span,
                    scope,
                );
            }
            visit::walk_variable(self, variable);
        }

        fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
            if let Some(scope) = self.scopes.last().copied() {
                self.insert(
                    binding.id,
                    &binding.name,
                    binding.name_span,
                    binding.span,
                    scope,
                );
            }
            if let Some(annotation) = &binding.annotation {
                self.visit_type_ref(annotation);
            }
        }

        fn visit_stmt(&mut self, statement: &'ast crate::ast::Stmt) {
            if let crate::ast::Stmt::For { binding, body, .. } = statement {
                self.insert(
                    binding.id,
                    &binding.name,
                    binding.span,
                    binding.span,
                    body.span,
                );
            }
            visit::walk_stmt(self, statement);
        }

        fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
            let binding = match &arm.pattern {
                MatchPattern::Enum { binding, .. }
                | MatchPattern::OptionSome(binding)
                | MatchPattern::IteratorItem(binding)
                | MatchPattern::ResultSuccess(binding)
                | MatchPattern::ResultError(binding) => binding.as_ref(),
                MatchPattern::Bool(_)
                | MatchPattern::Char(_)
                | MatchPattern::String(_)
                | MatchPattern::Int { .. }
                | MatchPattern::FileVersion(_)
                | MatchPattern::None
                | MatchPattern::IteratorEnd
                | MatchPattern::Wildcard => None,
            };
            if let Some(binding) = binding {
                self.insert(
                    binding.id,
                    &binding.name,
                    binding.name_span,
                    binding.name_span,
                    arm.span,
                );
            }
            visit::walk_match_arm(self, arm);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Closure { params, body, .. } = &expression.kind {
                self.scopes.push(body.span);
                for parameter in params {
                    self.visit_parameter(parameter);
                }
                self.visit_expr(body);
                self.scopes.pop();
            } else {
                visit::walk_expr(self, expression);
            }
        }
    }

    let mut collector = Collector::default();
    collector.visit_program(program);
    collector.variables
}

fn source_globals(program: &Program) -> HashMap<ValueId, SourceVariable> {
    program
        .globals
        .iter()
        .map(|global| {
            (
                global.id,
                SourceVariable {
                    name: global.name.clone(),
                    name_span: global.name_span,
                    declaration_span: global.span,
                    scope_span: global.span,
                },
            )
        })
        .collect()
}

impl DebugArtifactPlan {
    pub(super) fn new(inputs: DebugArtifactInputs<'_>) -> Self {
        let DebugArtifactInputs {
            abi,
            defined_functions,
            recorder,
            global_indices,
            global_types,
            program,
            source_name,
            source,
        } = inputs;
        let mut entries = abi
            .debug_names()
            .map(|(index, name)| (index, format!("env::{name}")))
            .chain(defined_functions.iter().cloned())
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|(index, _)| *index);
        assert!(
            entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "final function indices must have exactly one debug name"
        );
        assert!(
            entries
                .iter()
                .enumerate()
                .all(|(expected, (actual, _))| *actual == expected as u32),
            "every imported and defined function must receive a debug name"
        );

        let mut function_names = NameMap::new();
        for (index, name) in entries {
            function_names.append(index, &name);
        }
        let mut names = NameSection::new();
        names.module("SplitScript autosplitter");
        names.functions(&function_names);
        let source_variables = source_variables(program);
        let source_globals = source_globals(program);
        let mut variables = recorder.variables.borrow().clone();
        variables.sort_unstable_by_key(|variable| (variable.function, variable.local));
        let mut local_names = IndirectNameMap::new();
        for (function, variables) in variables
            .iter()
            .filter(|variable| source_variables.contains_key(&variable.value))
            .fold(
                BTreeMap::<u32, Vec<&DebugVariable>>::new(),
                |mut functions, variable| {
                    functions
                        .entry(variable.function)
                        .or_default()
                        .push(variable);
                    functions
                },
            )
        {
            let mut function_names = NameMap::new();
            let mut indices = HashSet::new();
            for variable in variables {
                assert!(
                    indices.insert(variable.local),
                    "one source name is assigned to each Wasm local"
                );
                function_names.append(variable.local, &source_variables[&variable.value].name);
            }
            local_names.append(function, &function_names);
        }
        names.locals(&local_names);
        let mut debug_globals = global_indices
            .iter()
            .filter_map(|(&value, &index)| {
                source_globals.contains_key(&value).then_some(DebugGlobal {
                    value,
                    index,
                    ty: global_types[&value],
                })
            })
            .filter(|global| global.index != u32::MAX && global.ty.has_runtime_value())
            .collect::<Vec<_>>();
        debug_globals.sort_unstable_by_key(|global| global.index);
        let mut global_names = NameMap::new();
        for global in &debug_globals {
            global_names.append(global.index, &source_globals[&global.value].name);
        }
        names.globals(&global_names);
        let dwarf = encode_dwarf(
            &entries_by_index(abi, defined_functions),
            recorder,
            &source_variables,
            &debug_globals,
            &source_globals,
            source_name,
            source,
        );
        Self { names, dwarf }
    }

    pub(super) fn names(&self) -> &NameSection {
        &self.names
    }

    pub(super) fn dwarf(&self) -> &[DebugSection] {
        &self.dwarf
    }
}

fn entries_by_index(abi: &Abi, defined_functions: &[(u32, String)]) -> BTreeMap<u32, String> {
    abi.debug_names()
        .map(|(index, name)| (index, format!("env::{name}")))
        .chain(defined_functions.iter().cloned())
        .collect()
}

fn encode_dwarf(
    names: &BTreeMap<u32, String>,
    recorder: &DebugRecorder,
    source_variables: &HashMap<ValueId, SourceVariable>,
    debug_globals: &[DebugGlobal],
    source_globals: &HashMap<ValueId, SourceVariable>,
    source_name: &str,
    source: &str,
) -> Vec<DebugSection> {
    let bodies = recorder.bodies.borrow();
    let mut markers = recorder.markers.borrow().clone();
    markers.sort_unstable_by_key(|marker| (marker.function, marker.body_offset));

    let mut functions = BTreeMap::<u32, Vec<BodyMarker>>::new();
    for marker in markers {
        let Some(body) = bodies.get(&marker.function) else {
            continue;
        };
        if marker.source.start >= marker.source.end
            || marker.source.end > source.len()
            || marker.body_offset >= body.raw_body_length
        {
            continue;
        }
        functions.entry(marker.function).or_default().push(marker);
    }
    if functions.is_empty() && debug_globals.is_empty() {
        return Vec::new();
    }

    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 5,
        address_size: 4,
    };
    let file_name = source_name.as_bytes().to_vec();
    let mut dwarf = DwarfUnit::new(encoding);
    let mut line_program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(b".".to_vec()),
        None,
        LineString::String(file_name.clone()),
        None,
    );
    let file = line_program.add_file(
        LineString::String(file_name.clone()),
        line_program.default_directory(),
        None,
    );

    let root = dwarf.unit.root();
    {
        let root = dwarf.unit.get_mut(root);
        root.set(gimli::DW_AT_name, AttributeValue::String(file_name));
        root.set(gimli::DW_AT_comp_dir, AttributeValue::String(b".".to_vec()));
        root.set(
            gimli::DW_AT_producer,
            AttributeValue::String(b"SplitScript compiler".to_vec()),
        );
        // SplitScript does not have an assigned DWARF language code or an LLDB
        // language plugin yet. `DW_LANG_lo_user` looks semantically tempting,
        // but LLDB then treats the JIT image as an unsupported language: line
        // stepping still works while function names and local variables stay
        // hidden. Use C as the debugger compatibility language until a
        // SplitScript plugin exists. This affects debugger presentation only;
        // all source names, types, locations, and line mappings below remain
        // SplitScript's own metadata.
        root.set(
            gimli::DW_AT_language,
            AttributeValue::Language(gimli::DW_LANG_C11),
        );
        root.set(gimli::DW_AT_stmt_list, AttributeValue::LineProgramRef);

        // Deliberately do not put `DW_AT_low_pc`, `DW_AT_high_pc`, or
        // `DW_AT_ranges` on the compilation unit. Wasmtime 45 special-cases a
        // subprogram DIE and expands it to the complete native JIT function,
        // but translates compilation-unit ranges with its generic source-range
        // algorithm. For non-monotonic control flow that algorithm can omit
        // native regions which are nevertheless present in the transformed
        // line table. LLDB then cannot associate those PCs with this unit and
        // reports anonymous frames with no variables. With no explicit unit
        // range, LLDB correctly derives ownership from the complete child
        // subprogram ranges.
    }
    let debug_variables = recorder.variables.borrow();
    let mut scalar_types = HashMap::new();

    for global in debug_globals {
        let Some(source_global) = source_globals.get(&global.value) else {
            continue;
        };
        let Some(ty) = scalar_type_entry(&mut dwarf, root, &mut scalar_types, global.ty) else {
            continue;
        };
        let variable = dwarf.unit.add(root, gimli::DW_TAG_variable);
        let (line, column) = source_position(source, source_global.name_span.start);
        let mut location = Expression::new();
        location.op_wasm_global(global.index);
        let entry = dwarf.unit.get_mut(variable);
        entry.set(
            gimli::DW_AT_name,
            AttributeValue::String(source_global.name.as_bytes().to_vec()),
        );
        entry.set(
            gimli::DW_AT_decl_file,
            AttributeValue::FileIndex(Some(file)),
        );
        entry.set(gimli::DW_AT_decl_line, AttributeValue::Udata(line));
        entry.set(gimli::DW_AT_decl_column, AttributeValue::Udata(column));
        entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
        entry.set(gimli::DW_AT_external, AttributeValue::Flag(true));
        entry.set(gimli::DW_AT_location, AttributeValue::Exprloc(location));
    }

    for (function, mut function_markers) in functions {
        let body = bodies[&function];
        function_markers.dedup_by_key(|marker| marker.body_offset);
        let first_address = body.raw_body_start + function_markers[0].body_offset;
        let end_address = body.raw_body_start + body.raw_body_length;

        line_program.begin_sequence(Some(Address::Constant(u64::from(first_address))));
        for (index, marker) in function_markers.iter().enumerate() {
            let address = body.raw_body_start + marker.body_offset;
            let (line, column) = source_position(source, marker.source.start);
            let row = line_program.row();
            row.address_offset = u64::from(address - first_address);
            row.file = file;
            row.line = line;
            row.column = column;
            row.discriminator = marker.kind.discriminator();
            row.is_statement = true;
            row.prologue_end = index == 0;
            line_program.generate_row();
        }
        line_program.end_sequence(u64::from(end_address - first_address));

        let (decl_line, decl_column) = source_position(source, function_markers[0].source.start);
        let subprogram = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        {
            let entry = dwarf.unit.get_mut(subprogram);
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(
                    names
                        .get(&function)
                        .expect("source-backed functions have symbolic names")
                        .as_bytes()
                        .to_vec(),
                ),
            );
            entry.set(
                gimli::DW_AT_decl_file,
                AttributeValue::FileIndex(Some(file)),
            );
            entry.set(gimli::DW_AT_decl_line, AttributeValue::Udata(decl_line));
            entry.set(gimli::DW_AT_decl_column, AttributeValue::Udata(decl_column));
            entry.set(
                gimli::DW_AT_low_pc,
                AttributeValue::Address(Address::Constant(u64::from(first_address))),
            );
            entry.set(
                gimli::DW_AT_high_pc,
                AttributeValue::Udata(u64::from(end_address - first_address)),
            );
        }

        for variable in debug_variables
            .iter()
            .filter(|variable| variable.function == function)
        {
            let Some(source_variable) = source_variables.get(&variable.value) else {
                continue;
            };
            let Some(ty) = scalar_type_entry(&mut dwarf, root, &mut scalar_types, variable.ty)
            else {
                continue;
            };
            let parent = if variable.parameter {
                subprogram
            } else {
                let Some((start, end)) = variable_range(body, &function_markers, source_variable)
                else {
                    continue;
                };
                let scope = dwarf.unit.add(subprogram, gimli::DW_TAG_lexical_block);
                let entry = dwarf.unit.get_mut(scope);
                entry.set(
                    gimli::DW_AT_low_pc,
                    AttributeValue::Address(Address::Constant(u64::from(start))),
                );
                entry.set(
                    gimli::DW_AT_high_pc,
                    AttributeValue::Udata(u64::from(end - start)),
                );
                scope
            };
            let tag = if variable.parameter {
                gimli::DW_TAG_formal_parameter
            } else {
                gimli::DW_TAG_variable
            };
            let variable_entry = dwarf.unit.add(parent, tag);
            let (line, column) = source_position(source, source_variable.name_span.start);
            let mut location = Expression::new();
            location.op_wasm_local(variable.local);
            // `DW_OP_WASM_location` identifies the Wasm local, and the
            // trailing stack-value operator tells Wasmtime that the local is
            // the value itself rather than a linear-memory address to
            // dereference.
            location.op(gimli::DW_OP_stack_value);
            let entry = dwarf.unit.get_mut(variable_entry);
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(source_variable.name.as_bytes().to_vec()),
            );
            entry.set(
                gimli::DW_AT_decl_file,
                AttributeValue::FileIndex(Some(file)),
            );
            entry.set(gimli::DW_AT_decl_line, AttributeValue::Udata(line));
            entry.set(gimli::DW_AT_decl_column, AttributeValue::Udata(column));
            entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
            entry.set(gimli::DW_AT_location, AttributeValue::Exprloc(location));
        }
    }
    dwarf.unit.line_program = line_program;

    let mut sections = Sections::new(EndianVec::new(LittleEndian));
    dwarf
        .write(&mut sections)
        .expect("compiler-generated DWARF should be internally valid");
    let mut output = Vec::new();
    sections
        .for_each(|id, section| {
            if !section.slice().is_empty() {
                output.push(DebugSection {
                    name: id.name(),
                    data: section.slice().to_vec(),
                });
            }
            Ok::<_, std::convert::Infallible>(())
        })
        .expect("infallible DWARF section collection");
    output
}

fn source_position(source: &str, offset: usize) -> (u64, u64) {
    let before = &source[..offset];
    let line_start = before.rfind('\n').map_or(0, |position| position + 1);
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u64 + 1;
    let column = source[line_start..offset].chars().count() as u64 + 1;
    (line, column)
}

fn scalar_type_entry(
    dwarf: &mut DwarfUnit,
    root: UnitEntryId,
    types: &mut HashMap<Type, UnitEntryId>,
    ty: Type,
) -> Option<UnitEntryId> {
    if let Some(entry) = types.get(&ty) {
        return Some(*entry);
    }
    let (name, byte_size, encoding) = match ty {
        Type::Bool => ("bool", 1, gimli::DW_ATE_boolean),
        Type::Char => ("char", 4, gimli::DW_ATE_UTF),
        Type::I8 => ("i8", 1, gimli::DW_ATE_signed),
        Type::U8 => ("u8", 1, gimli::DW_ATE_unsigned),
        Type::I16 => ("i16", 2, gimli::DW_ATE_signed),
        Type::U16 => ("u16", 2, gimli::DW_ATE_unsigned),
        Type::I32 => ("i32", 4, gimli::DW_ATE_signed),
        Type::U32 => ("u32", 4, gimli::DW_ATE_unsigned),
        Type::I64 => ("i64", 8, gimli::DW_ATE_signed),
        Type::U64 => ("u64", 8, gimli::DW_ATE_unsigned),
        Type::Address => ("address", 8, gimli::DW_ATE_unsigned),
        Type::F32 => ("f32", 4, gimli::DW_ATE_float),
        Type::F64 => ("f64", 8, gimli::DW_ATE_float),
        Type::Never
        | Type::None
        | Type::Standard(_)
        | Type::StateSnapshot
        | Type::SettingsView
        | Type::Record(_)
        | Type::Enum(_)
        | Type::ArrayStorage(_)
        | Type::Array(_)
        | Type::Option(_)
        | Type::Result(_)
        | Type::Async(_)
        | Type::Callable(_)
        | Type::Range(_)
        | Type::Set(_)
        | Type::Application(_) => return None,
    };
    let entry = dwarf.unit.add(root, gimli::DW_TAG_base_type);
    let base = dwarf.unit.get_mut(entry);
    base.set(
        gimli::DW_AT_name,
        AttributeValue::String(name.as_bytes().to_vec()),
    );
    base.set(gimli::DW_AT_byte_size, AttributeValue::Udata(byte_size));
    base.set(gimli::DW_AT_encoding, AttributeValue::Encoding(encoding));
    types.insert(ty, entry);
    Some(entry)
}

fn variable_range(
    body: BodyRange,
    markers: &[BodyMarker],
    variable: &SourceVariable,
) -> Option<(u32, u32)> {
    let in_scope = |marker: &BodyMarker| {
        variable.scope_span.start <= marker.source.start
            && marker.source.end <= variable.scope_span.end
    };
    let start_index = markers.iter().position(|marker| {
        in_scope(marker) && marker.source.end > variable.declaration_span.start
    })?;
    let last_index = markers
        .iter()
        .enumerate()
        .skip(start_index)
        .take_while(|(_, marker)| in_scope(marker))
        .map(|(index, _)| index)
        .last()?;
    let start = body.raw_body_start + markers[start_index].body_offset;
    let end_offset = markers
        .iter()
        .skip(last_index + 1)
        .find(|marker| marker.body_offset > markers[last_index].body_offset)
        .map_or(body.raw_body_length, |marker| marker.body_offset);
    let end = body.raw_body_start + end_offset;
    (start < end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::{BodyMarker, BodyMarkerKind, BodyRange, SourceVariable, variable_range};
    use crate::ast::Span;

    #[test]
    fn local_ranges_start_at_declaration_and_stop_when_the_source_scope_ends() {
        let body = BodyRange {
            raw_body_start: 100,
            raw_body_length: 50,
        };
        let markers = [
            BodyMarker {
                function: 0,
                body_offset: 2,
                source: Span { start: 2, end: 4 },
                kind: BodyMarkerKind::Source,
            },
            BodyMarker {
                function: 0,
                body_offset: 10,
                source: Span { start: 22, end: 28 },
                kind: BodyMarkerKind::Source,
            },
            BodyMarker {
                function: 0,
                body_offset: 18,
                source: Span { start: 30, end: 35 },
                kind: BodyMarkerKind::Source,
            },
            BodyMarker {
                function: 0,
                body_offset: 26,
                source: Span { start: 45, end: 48 },
                kind: BodyMarkerKind::Source,
            },
        ];
        let variable = SourceVariable {
            name: "inner".to_owned(),
            name_span: Span { start: 20, end: 25 },
            declaration_span: Span { start: 18, end: 29 },
            scope_span: Span { start: 15, end: 40 },
        };
        assert_eq!(variable_range(body, &markers, &variable), Some((110, 126)));
    }
}
