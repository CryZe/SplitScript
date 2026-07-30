//! SplitScript compiler library.
//!
//! The compiler intentionally has a small public API: parse, type-check, and
//! compile a source file to a WebAssembly GC module.

pub mod abi;
pub mod ast;
pub mod catalog;
pub mod codegen;
pub mod completion;
pub mod database;
pub mod diagnostic;
pub mod documentation;
pub mod effects;
pub mod equality;
pub mod formatter;
pub mod highlight;
pub mod hir;
mod inference;
pub mod insight;
pub mod language;
pub mod lexer;
pub mod lsp;
pub mod memory;
pub mod parser;
pub mod semantic;
pub mod signature;
pub mod stdlib;
pub mod symbols;
pub mod syntax;
pub mod typeck;
pub mod types;
pub mod visit;
pub mod wasm_ir;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticFix, DiagnosticLabel, DiagnosticLabelStyle,
    DiagnosticSeverity, FixApplicability, TextEdit,
};
pub use formatter::format_source;

/// Controls profile-sensitive semantic lowering and WebAssembly generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// A source file that has been parsed but not semantically checked.
#[derive(Debug, Clone)]
pub struct ParsedProgram {
    document: syntax::SourceDocument,
    syntax: ast::Program,
}

/// A lossless, partial parse intended for diagnostics and editor tooling.
/// Unlike [`ParsedProgram`], this remains available when syntax errors were
/// recovered at top-level declaration boundaries.
#[derive(Debug, Clone)]
pub struct RecoveredParse {
    document: syntax::SourceDocument,
    syntax: ast::Program,
    diagnostics: Vec<Diagnostic>,
    recovery_nodes: Vec<syntax::RecoveryNode>,
}

impl RecoveredParse {
    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn recovery_nodes(&self) -> &[syntax::RecoveryNode] {
        &self.recovery_nodes
    }
}

/// A parsed program with declaration identities collected into an inspectable
/// pre-type-check HIR product.
#[derive(Debug, Clone)]
pub struct LoweredProgram {
    document: syntax::SourceDocument,
    syntax: ast::Program,
    hir: hir::Program,
}

impl LoweredProgram {
    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn hir(&self) -> &hir::Program {
        &self.hir
    }
}

impl From<ParsedProgram> for LoweredProgram {
    fn from(parsed: ParsedProgram) -> Self {
        lower(parsed)
    }
}

impl ParsedProgram {
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
    document: syntax::SourceDocument,
    syntax: ast::Program,
    hir: hir::TypedProgram,
    semantics: semantic::SemanticModel,
    memory_layouts: memory::MemoryLayouts,
    equality: equality::EqualityCapabilities,
    effects: effects::OperationAnalysis,
    enum_types: Vec<ast::EnumDecl>,
    array_types: Vec<ast::ArrayTypeDecl>,
    option_types: Vec<ast::OptionTypeDecl>,
    result_types: Vec<ast::ResultTypeDecl>,
}

/// Semantic facts retained for editor tooling even when type checking reports
/// errors. Expressions that could not be typed may be absent, while facts from
/// independent declarations and expressions remain queryable.
#[derive(Debug, Clone)]
pub struct RecoveredCheck {
    document: syntax::SourceDocument,
    syntax: ast::Program,
    hir: hir::Program,
    semantics: semantic::SemanticModel,
    diagnostics: Vec<Diagnostic>,
    enum_types: Vec<ast::EnumDecl>,
    effects: Option<effects::OperationAnalysis>,
}

impl RecoveredCheck {
    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn hir(&self) -> &hir::Program {
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
    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.document
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.syntax
    }

    pub fn semantics(&self) -> &semantic::SemanticModel {
        &self.semantics
    }

    pub fn hir(&self) -> &hir::Program {
        self.hir.declarations()
    }

    pub fn typed_hir(&self) -> &hir::TypedProgram {
        &self.hir
    }

    pub fn memory_layouts(&self) -> &memory::MemoryLayouts {
        &self.memory_layouts
    }

    pub fn equality(&self) -> &equality::EqualityCapabilities {
        &self.equality
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
    let recovered = parse_recovering(source)?;
    if !recovered.diagnostics.is_empty() {
        return Err(recovered.diagnostics);
    }
    Ok(ParsedProgram {
        document: recovered.document,
        syntax: recovered.syntax,
    })
}

/// Parses as much of one SplitScript source file as possible. Lexer failures
/// remain fatal for now; recoverable parser errors and explicit recovery nodes
/// are returned alongside the partial syntax tree.
pub fn parse_recovering(source: &str) -> Result<RecoveredParse, Vec<Diagnostic>> {
    let lexed = lexer::lex_lossless(source).map_err(|error| vec![error])?;
    let tokens = lexed.tokens().cloned().collect();
    let output = parser::parse_recovering(source, tokens);
    Ok(RecoveredParse {
        document: syntax::SourceDocument::new(source, lexed),
        syntax: output.program,
        diagnostics: output.diagnostics,
        recovery_nodes: output.recovery_nodes,
    })
}

/// Lowers parsed declarations into the inspectable pre-type-check HIR.
pub fn lower(parsed: ParsedProgram) -> LoweredProgram {
    let hir = hir::Program::lower(&parsed.syntax);
    LoweredProgram {
        document: parsed.document,
        syntax: parsed.syntax,
        hir,
    }
}

/// Resolves and type-checks a parsed program without invoking the Wasm backend.
pub fn check(lowered: impl Into<LoweredProgram>) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let LoweredProgram {
        document,
        syntax,
        hir,
    } = lowered.into();
    let output = typeck::check(&syntax)?;
    let typed_hir = hir::TypedProgram::build(hir, &syntax, &output.semantics);
    let effects = effects::OperationAnalysis::infer(&typed_hir);
    let memory_layouts = memory::MemoryLayouts::build(&syntax.records, &output.semantics);
    let equality = equality::EqualityCapabilities::build(
        &syntax.records,
        &output.enum_types,
        &output.semantics,
    );
    let mut semantic_errors = Vec::new();
    for violation in effects.detached_call_violations(&typed_hir) {
        let name = violation
            .standard_library_name
            .map(str::to_owned)
            .or_else(|| {
                let function = violation.function?;
                syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.name.clone())
            });
        semantic_errors.push(Diagnostic::semantic(
            format!(
                "`{}` requires an attached process and is unavailable in `onDetached`",
                name.unwrap_or_else(|| "function".to_owned())
            ),
            violation.expression_span,
        ));
    }
    for expression in typed_hir.expressions() {
        if let hir::TypedExpressionKind::Binary {
            op: ast::BinaryOp::Eq | ast::BinaryOp::Ne,
            left,
            ..
        } = expression.kind
        {
            let operand = typed_hir
                .expression(left)
                .expect("binary operands belong to typed HIR");
            if let Err(error) = equality.require(operand.ty, &output.semantics) {
                semantic_errors.push(Diagnostic::semantic(error, expression.span));
            }
        }
        if let Some(semantic::ResolvedCall::StandardLibrary {
            item: stdlib::StdlibItemId::ProcessRead,
            type_arguments,
            ..
        }) = typed_hir.call(expression.id)
            && let Err(error) = memory_layouts.layout(type_arguments[0], &output.semantics)
        {
            semantic_errors.push(Diagnostic::semantic(error, expression.span));
        }
    }
    if let Some(state) = &syntax.state {
        for field in &state.fields {
            if matches!(field.source, ast::StateSource::Pointer(_)) {
                let ty = output
                    .semantics
                    .value_type(field.id)
                    .expect("checked state fields have semantic types");
                if let Err(error) = memory_layouts.layout(ty, &output.semantics) {
                    semantic_errors.push(Diagnostic::semantic(error, field.span));
                }
            }
        }
    }
    if !semantic_errors.is_empty() {
        return Err(semantic_errors);
    }
    Ok(CheckedProgram {
        document,
        syntax,
        hir: typed_hir,
        semantics: output.semantics,
        memory_layouts,
        equality,
        effects,
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
        document,
        syntax,
        hir,
    } = lowered.into();
    let recovered = typeck::check_recovering(&syntax);
    let effects = recovered.diagnostics.is_empty().then(|| {
        let typed_hir = hir::TypedProgram::build(hir.clone(), &syntax, &recovered.output.semantics);
        effects::OperationAnalysis::infer(&typed_hir)
    });
    RecoveredCheck {
        document,
        syntax,
        hir,
        semantics: recovered.output.semantics,
        diagnostics: recovered.diagnostics,
        enum_types: recovered.output.enum_types,
        effects,
    }
}

/// Lowers a checked program into the inspectable Wasm-oriented control-flow
/// and storage plan consumed by the binary encoder.
pub fn lower_wasm(checked: &CheckedProgram) -> wasm_ir::Program {
    lower_wasm_with_options(checked, CompilerOptions::default())
}

/// Lowers with explicit profile-sensitive compiler options.
pub fn lower_wasm_with_options(
    checked: &CheckedProgram,
    options: CompilerOptions,
) -> wasm_ir::Program {
    wasm_ir::Program::lower(&checked.hir, &checked.semantics, options.profile)
}

/// Generates WebAssembly from a successfully checked program.
pub fn codegen(checked: &CheckedProgram) -> Vec<u8> {
    codegen_with_options(checked, CompilerOptions::default())
}

/// Generates WebAssembly with explicit compiler options.
pub fn codegen_with_options(checked: &CheckedProgram, options: CompilerOptions) -> Vec<u8> {
    let wasm_ir = lower_wasm_with_options(checked, options);
    codegen::compile(
        &checked.syntax,
        &checked.semantics,
        &checked.hir,
        &wasm_ir,
        codegen::ConstructedTypes {
            enums: &checked.enum_types,
            arrays: &checked.array_types,
            options: &checked.option_types,
            results: &checked.result_types,
        },
        &checked.memory_layouts,
        &checked.equality,
    )
}

pub fn compile(source: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_options(source, CompilerOptions::default())
}

/// Runs the complete compiler pipeline with explicit options.
pub fn compile_with_options(
    source: &str,
    options: CompilerOptions,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let parsed = parse(source)?;
    let lowered = lower(parsed);
    let checked = check(lowered)?;
    Ok(codegen_with_options(&checked, options))
}
