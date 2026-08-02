//! SplitScript compiler library.
//!
//! The compiler intentionally has a small public API: parse, type-check, and
//! compile a source file to a WebAssembly GC module.

mod abi;
pub use splitscript_syntax::ast;
mod capabilities;
mod catalog;
mod codegen;
pub mod compiler;
mod completion;
mod database;
pub use splitscript_syntax::diagnostic;
mod documentation;
mod effects;
mod equality;
mod formatter;
mod highlight;
mod hir;
mod inference;
mod inlay_hints;
mod insight;
mod intrinsic_registry;
mod language;
mod lexer;
mod lsp;
mod memory;
use splitscript_syntax::parser;
mod resolution;
mod semantic;
mod service;
mod signature;
mod stdlib;
mod stdlib_semantic;
mod symbols;
pub use splitscript_syntax::source as syntax;
pub use splitscript_syntax::visit;
pub mod tooling;
mod type_display;
mod typeck;
mod types;
mod validation;
mod wasm_ir;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticFix, DiagnosticLabel, DiagnosticLabelStyle,
    DiagnosticSeverity, FixApplicability, TextEdit,
};
pub use formatter::format_source;

/// Controls profile-sensitive semantic lowering and WebAssembly generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuildProfile {
    /// Keeps constructs marked for development-time diagnostics and logging.
    #[default]
    Debug,
    /// Erases debug-only constructs before backend reachability is computed.
    Release,
}

/// Options shared by staged and one-shot compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompilerOptions {
    pub profile: BuildProfile,
}

/// Immutable compiler-wide services shared by every stage of one compilation.
///
/// The build-time privileged SplitScript loader supplies the bundled catalog;
/// this context is the runtime injection boundary for that validated graph.
/// Individual passes consume the context instead of reconstructing global
/// catalog state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CompilerContext {
    standard_library: stdlib::StandardLibrary,
}

impl CompilerContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_standard_library(standard_library: stdlib::StandardLibrary) -> Self {
        Self { standard_library }
    }

    pub fn standard_library(&self) -> stdlib::StandardLibrary {
        self.standard_library.clone()
    }
}

/// Compiles the bundled source bodies as a self-contained program and derives
/// their transitive operational metadata through the ordinary semantic
/// pipeline. This runs once for each independently owned standard-library
/// graph and never depends on user source.
fn derive_standard_library_operation_metadata(
    standard_library: stdlib::StandardLibrary,
) -> Result<
    std::collections::HashMap<stdlib::StdlibItemId, stdlib::OperationMetadata>,
    Vec<Diagnostic>,
> {
    let context = CompilerContext::with_standard_library(standard_library.clone());
    let checked = check(lower(parse_with_context(
        context,
        "state \"__splitscript_standard_library__\" {}",
    )?))?;
    let mut operations = std::collections::HashMap::new();
    for item in standard_library.items() {
        if !matches!(
            item.implementation,
            stdlib::Implementation::LibraryBody { .. }
        ) {
            continue;
        }
        let function = checked
            .hir
            .library_function(item.id)
            .expect("checked standard-library bodies have function identities");
        let metadata = checked.effects.function(function).metadata();
        operations.insert(item.id, metadata);
    }
    Ok(operations)
}

/// A source file that has been parsed but not semantically checked.
#[derive(Debug, Clone)]
pub struct ParsedProgram {
    context: CompilerContext,
    document: syntax::SourceDocument,
    syntax: ast::Program,
    resolution_diagnostics: Vec<Diagnostic>,
}

/// A lossless, partial parse intended for diagnostics and editor tooling.
/// Unlike [`ParsedProgram`], this remains available when syntax errors were
/// recovered at top-level declaration boundaries.
#[derive(Debug, Clone)]
pub struct RecoveredParse {
    context: CompilerContext,
    document: syntax::SourceDocument,
    syntax: ast::Program,
    diagnostics: Vec<Diagnostic>,
    resolution_diagnostics: Vec<Diagnostic>,
    recovery_nodes: Vec<syntax::RecoveryNode>,
}

impl RecoveredParse {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Nominal declaration conflicts retained independently from syntax
    /// recovery. These are reported by checking, not by parsing.
    pub fn resolution_diagnostics(&self) -> &[Diagnostic] {
        &self.resolution_diagnostics
    }

    pub fn recovery_nodes(&self) -> &[syntax::RecoveryNode] {
        &self.recovery_nodes
    }
}

/// A parsed program with declaration identities collected into an inspectable
/// pre-type-check HIR product.
#[derive(Debug, Clone)]
pub struct LoweredProgram {
    context: CompilerContext,
    document: syntax::SourceDocument,
    syntax: ast::Program,
    /// User syntax plus compiler-owned standard-library bodies. Kept private
    /// so editor and public compiler queries never expose injected symbols.
    compilation_syntax: ast::Program,
    hir: hir::DeclarationIndex,
    resolutions: resolution::ProgramResolutions,
    resolution_diagnostics: Vec<Diagnostic>,
}

impl LoweredProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        &self.hir
    }

    pub fn resolution_diagnostics(&self) -> &[Diagnostic] {
        &self.resolution_diagnostics
    }
}

impl From<ParsedProgram> for LoweredProgram {
    fn from(parsed: ParsedProgram) -> Self {
        lower(parsed)
    }
}

impl ParsedProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn into_syntax(self) -> ast::Program {
        self.syntax
    }
}

/// A successfully checked program and the resolved semantic facts needed by
/// later compiler stages and editor tooling.
#[derive(Debug, Clone)]
pub struct CheckedProgram {
    context: CompilerContext,
    document: syntax::SourceDocument,
    syntax: ast::Program,
    compilation_syntax: ast::Program,
    hir: hir::TypedProgram,
    semantics: semantic::SemanticModel,
    capabilities: capabilities::CapabilityAnalysis,
    effects: effects::OperationAnalysis,
    enum_types: Vec<ast::EnumDecl>,
    array_types: Vec<types::ResolvedArrayType>,
    option_types: Vec<types::ResolvedOptionType>,
    result_types: Vec<types::ResolvedResultType>,
}

/// Semantic facts retained for editor tooling even when type checking reports
/// errors. Expressions that could not be typed may be absent, while facts from
/// independent declarations and expressions remain queryable.
#[derive(Debug, Clone)]
pub struct RecoveredCheck {
    context: CompilerContext,
    document: syntax::SourceDocument,
    syntax: ast::Program,
    hir: hir::DeclarationIndex,
    semantics: semantic::SemanticModel,
    diagnostics: Vec<Diagnostic>,
    enum_types: Vec<ast::EnumDecl>,
    effects: Option<effects::OperationAnalysis>,
}

impl RecoveredCheck {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        &self.hir
    }

    pub fn semantics(&self) -> &semantic::SemanticModel {
        &self.semantics
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn enum_types(&self) -> &[ast::EnumDecl] {
        &self.enum_types
    }

    /// Operational effects are available when type recovery completed without
    /// errors, even if a later semantic validation rejected the program.
    pub fn effects(&self) -> Option<&effects::OperationAnalysis> {
        self.effects.as_ref()
    }
}

impl CheckedProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn semantics(&self) -> &semantic::SemanticModel {
        &self.semantics
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        self.hir.declarations()
    }

    pub fn typed_hir(&self) -> &hir::TypedProgram {
        &self.hir
    }

    pub fn memory_layouts(&self) -> &memory::MemoryLayouts {
        self.capabilities.memory()
    }

    pub fn equality(&self) -> &equality::EqualityCapabilities {
        self.capabilities.equality()
    }

    pub fn capabilities(&self) -> &capabilities::CapabilityAnalysis {
        &self.capabilities
    }

    pub fn effects(&self) -> &effects::OperationAnalysis {
        &self.effects
    }

    /// Source enum layouts visible to semantic analysis. Standard-library
    /// enums retain their catalog identities and are not synthesized here.
    pub fn enum_types(&self) -> &[ast::EnumDecl] {
        &self.enum_types
    }
}

/// Parses one SplitScript source file without running semantic analysis.
pub fn parse(source: &str) -> Result<ParsedProgram, Vec<Diagnostic>> {
    parse_with_context(CompilerContext::default(), source)
}

pub fn parse_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<ParsedProgram, Vec<Diagnostic>> {
    let recovered = parse_recovering_with_context(context.clone(), source)?;
    if !recovered.diagnostics.is_empty() {
        return Err(recovered.diagnostics);
    }
    Ok(ParsedProgram {
        context,
        document: recovered.document,
        syntax: recovered.syntax,
        resolution_diagnostics: recovered.resolution_diagnostics,
    })
}

/// Parses as much of one SplitScript source file as possible. Lexer failures
/// remain fatal for now; recoverable parser errors and explicit recovery nodes
/// are returned alongside the partial syntax tree.
pub fn parse_recovering(source: &str) -> Result<RecoveredParse, Vec<Diagnostic>> {
    parse_recovering_with_context(CompilerContext::default(), source)
}

pub fn parse_recovering_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<RecoveredParse, Vec<Diagnostic>> {
    let lexed = lexer::lex_lossless(source).map_err(|error| vec![error])?;
    let tokens = lexed.tokens().cloned().collect();
    let output = parser::parse_recovering(source, tokens);
    let resolution_diagnostics =
        resolution::validate_declarations(&output.program, &context.standard_library());
    Ok(RecoveredParse {
        context,
        document: syntax::SourceDocument::from_lexed(source, lexed),
        syntax: output.program,
        diagnostics: output.diagnostics,
        resolution_diagnostics,
        recovery_nodes: output.recovery_nodes,
    })
}

/// Lowers parsed declarations into the inspectable pre-type-check HIR.
pub fn lower(parsed: ParsedProgram) -> LoweredProgram {
    let syntax = parsed.syntax;
    let mut compilation_syntax = syntax.clone();
    let mut resolution_diagnostics = parsed.resolution_diagnostics;
    if let Some(augmented) = stdlib::augment_program_with_library_bodies(
        parsed.document.source(),
        &parsed.context.standard_library(),
    )
    .unwrap_or_else(|diagnostics| {
        panic!(
            "validated standard-library bodies must parse as ordinary SplitScript: {diagnostics:#?}"
        )
    }) {
        compilation_syntax = augmented;
    }
    let mut resolutions = resolution::ProgramResolutions::default();
    resolution_diagnostics.extend(resolution::resolve_program(
        &compilation_syntax,
        &parsed.context.standard_library(),
        &mut resolutions,
    ));
    let hir = hir::DeclarationIndex::lower(&syntax);
    LoweredProgram {
        context: parsed.context,
        document: parsed.document,
        syntax,
        compilation_syntax,
        hir,
        resolutions,
        resolution_diagnostics,
    }
}

/// Resolves and type-checks a parsed program without invoking the Wasm backend.
pub fn check(lowered: impl Into<LoweredProgram>) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let LoweredProgram {
        context,
        document,
        syntax,
        compilation_syntax,
        hir,
        resolutions,
        resolution_diagnostics,
    } = lowered.into();
    if !resolution_diagnostics.is_empty() {
        return Err(resolution_diagnostics);
    }
    let mut output = typeck::check_with_library(
        &compilation_syntax,
        &resolutions,
        context.standard_library(),
    )?;
    let typed_hir = hir::TypedProgram::build(
        hir,
        &compilation_syntax,
        &output.semantics,
        context.standard_library(),
        hir::visible_expression_count(&syntax),
        syntax.functions.len(),
    );
    output
        .semantics
        .set_visible_expression_count(typed_hir.visible_expression_count());
    let validation = validation::validate(
        context.standard_library(),
        &compilation_syntax,
        &typed_hir,
        &output.semantics,
        &output.enum_types,
    );
    if !validation.diagnostics.is_empty() {
        return Err(validation.diagnostics);
    }
    Ok(CheckedProgram {
        context,
        document,
        syntax,
        compilation_syntax,
        hir: typed_hir,
        semantics: output.semantics,
        capabilities: validation.capabilities,
        effects: validation.effects,
        enum_types: output.enum_types,
        array_types: output.array_types,
        option_types: output.option_types,
        result_types: output.result_types,
    })
}

/// Runs error-tolerant type inference without invoking typed-HIR construction
/// or the WebAssembly backend.
pub fn check_recovering(lowered: impl Into<LoweredProgram>) -> RecoveredCheck {
    let LoweredProgram {
        context,
        document,
        syntax,
        compilation_syntax,
        hir,
        resolutions,
        resolution_diagnostics,
    } = lowered.into();
    let mut recovered = typeck::check_recovering_with_library(
        &compilation_syntax,
        &resolutions,
        context.standard_library(),
    );
    let validation =
        (resolution_diagnostics.is_empty() && recovered.diagnostics.is_empty()).then(|| {
            let typed_hir = hir::TypedProgram::build(
                hir.clone(),
                &compilation_syntax,
                &recovered.output.semantics,
                context.standard_library(),
                hir::visible_expression_count(&syntax),
                syntax.functions.len(),
            );
            recovered
                .output
                .semantics
                .set_visible_expression_count(typed_hir.visible_expression_count());
            validation::validate(
                context.standard_library(),
                &compilation_syntax,
                &typed_hir,
                &recovered.output.semantics,
                &recovered.output.enum_types,
            )
        });
    let mut diagnostics = resolution_diagnostics;
    diagnostics.extend(recovered.diagnostics);
    if let Some(validation) = &validation {
        diagnostics.extend(validation.diagnostics.iter().cloned());
    }
    RecoveredCheck {
        context,
        document,
        syntax,
        hir,
        semantics: recovered.output.semantics,
        diagnostics,
        enum_types: recovered.output.enum_types,
        effects: validation.map(|validation| validation.effects),
    }
}

/// Lowers a checked program into the inspectable Wasm-oriented control-flow
/// and storage plan consumed by the binary encoder.
pub fn lower_wasm(checked: &CheckedProgram) -> codegen::BackendProgram<'_> {
    lower_wasm_with_options(checked, CompilerOptions::default())
}

/// Lowers with explicit profile-sensitive compiler options.
pub fn lower_wasm_with_options(
    checked: &CheckedProgram,
    options: CompilerOptions,
) -> codegen::BackendProgram<'_> {
    let wasm_ir = wasm_ir::Program::lower(&checked.hir, &checked.semantics, options.profile);
    codegen::BackendProgram::new(checked, wasm_ir)
}

/// Generates WebAssembly from a successfully checked program.
pub fn codegen(checked: &CheckedProgram) -> Vec<u8> {
    codegen_with_options(checked, CompilerOptions::default())
}

/// Generates WebAssembly with explicit compiler options.
pub fn codegen_with_options(checked: &CheckedProgram, options: CompilerOptions) -> Vec<u8> {
    codegen::compile(lower_wasm_with_options(checked, options))
}

pub fn compile(source: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_options(source, CompilerOptions::default())
}

pub fn compile_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_context_and_options(context, source, CompilerOptions::default())
}

/// Runs the complete compiler pipeline with explicit options.
pub fn compile_with_options(
    source: &str,
    options: CompilerOptions,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_context_and_options(CompilerContext::default(), source, options)
}

pub fn compile_with_context_and_options(
    context: CompilerContext,
    source: &str,
    options: CompilerOptions,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let parsed = parse_with_context(context, source)?;
    let lowered = lower(parsed);
    let checked = check(lowered)?;
    Ok(codegen_with_options(&checked, options))
}

#[cfg(test)]
mod compiler_context_tests {
    use super::*;

    #[test]
    fn separately_owned_standard_library_flows_through_every_stage() {
        let bundled = stdlib::StandardLibrary::new();
        let isolated = stdlib::StandardLibrary::isolated_bundled();
        assert_ne!(bundled, isolated, "graphs must have independent identity");

        let context = CompilerContext::with_standard_library(isolated.clone());
        let parsed = parse_with_context(context, "state \"game.exe\" {}")
            .expect("the alternate validated graph should parse");
        assert_eq!(parsed.context().standard_library(), isolated);
        let checked = check(lower(parsed)).expect("the alternate graph should type-check");
        assert_eq!(checked.context().standard_library(), isolated);
        let lowered = lower_wasm(&checked);
        assert_eq!(lowered.standard_library(), &isolated);
        assert!(!codegen(&checked).is_empty());
    }
}
