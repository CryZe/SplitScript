//! Profile-aware debugger metadata assembled only after Wasm indices are final.

use std::{cell::RefCell, collections::BTreeMap};

use gimli::{
    Encoding, Format, LineEncoding, LittleEndian,
    write::{Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Sections},
};
use wasm_encoder::{Function, NameMap, NameSection};

use super::imports::Abi;
use crate::{
    ast::{EnumDecl, Program, Span, TypeApplicationId},
    semantic::{FunctionInstance, SemanticModel},
    stdlib::StandardLibrary,
    types::{TypeId, TypeKind},
};

pub(super) struct DebugArtifactPlan {
    names: NameSection,
    dwarf: Vec<DebugSection>,
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
}

#[derive(Debug, Clone, Copy)]
struct BodyRange {
    raw_body_start: u32,
    raw_body_length: u32,
}

/// Interior-mutability is intentionally confined to debug emission. Normal
/// code generation stays unaware of line-table state and release compilation
/// never constructs this recorder.
#[derive(Default)]
pub(super) struct DebugRecorder {
    markers: RefCell<Vec<BodyMarker>>,
    bodies: RefCell<BTreeMap<u32, BodyRange>>,
}

#[derive(Clone, Copy)]
pub(super) struct DebugEmission<'a> {
    pub recorder: &'a DebugRecorder,
    pub function: u32,
}

impl DebugEmission<'_> {
    pub fn mark(self, function: &Function, source: Option<Span>) {
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
        TypeKind::Set { element, .. } => format!("Set<{}>", nested(*element)),
    }
}

impl DebugArtifactPlan {
    pub(super) fn new(
        abi: &Abi,
        defined_functions: &[(u32, String)],
        recorder: &DebugRecorder,
        source_name: &str,
        source: &str,
    ) -> Self {
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
        let dwarf = encode_dwarf(
            &entries_by_index(abi, defined_functions),
            recorder,
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
    if functions.is_empty() {
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
        root.set(
            gimli::DW_AT_language,
            AttributeValue::Language(gimli::DW_LANG_lo_user),
        );
        root.set(gimli::DW_AT_stmt_list, AttributeValue::LineProgramRef);
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
            row.is_statement = true;
            row.prologue_end = index == 0;
            line_program.generate_row();
        }
        line_program.end_sequence(u64::from(end_address - first_address));

        let (decl_line, decl_column) = source_position(source, function_markers[0].source.start);
        let entry = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        let entry = dwarf.unit.get_mut(entry);
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
