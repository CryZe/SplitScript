# Compiler architecture

SplitScript exposes its front-end stages as separate inspectable products:

```text
source
  -> parse -> ParsedProgram (syntax AST)
  -> lower -> LoweredProgram (immutable syntax + declaration HIR)
  -> check -> CheckedProgram (syntax + declaration HIR + typed body HIR + inferred layouts)
  -> lower_wasm -> wasm_ir::Program (control flow + storage plans)
  -> codegen -> WebAssembly GC
```

`CompilerOptions` selects `BuildProfile::Debug` or `BuildProfile::Release` for
the profile-aware `lower_wasm_with_options`, `codegen_with_options`, and
`compile_with_options` entry points. The selected profile is stored on the
Wasm-oriented program so profile erasure can remain a semantic lowering pass.
The convenience entry points default to debug. Until debug-only syntax is
encountered, both profiles emit identical modules. Debug statements, bindings,
globals, and functions remain in typed HIR for diagnostics in every profile.
Release Wasm lowering filters them before local planning, async-state
construction, global storage planning, and backend reachability. The backend
consumes the resulting Wasm-IR global set rather than making its own profile
decision.

`lower` currently establishes a deterministic declaration index with stable
IDs for named types, functions, values, and lifecycle actions. Tools can query
this HIR even when type checking would fail.

Every successful compiler stage also retains a `SourceDocument` produced by
the same lexer pass used by the parser. Its ordered lexeme stream includes
ordinary line and block comments, whitespace, documentation comments, and
semantic tokens with exact byte spans, so concatenating their raw source slices
reproduces the input byte-for-byte. Formatter and editor consumers can use this
layer without reconstructing spelling or layout from the semantic AST.

`parse_recovering` is the editor-facing entry point. It returns the lossless
document, a partial syntax AST, every diagnostic recoverable at top-level
declaration boundaries, and zero-width missing or source-backed error recovery
nodes. The ordinary `parse` entry point uses the same recovery pass but remains
strict: any collected syntax diagnostic makes the batch parse fail.

Statement blocks also recover at semicolons, their closing brace, or the next
plausible statement on a later line. Nested blocks recover independently, so a
bad local declaration does not discard later statements in the same function
or lifecycle action. Failed expectations leave the unexpected token untouched,
which prevents a missing delimiter from consuming a valid recovery boundary.

Record and enum bodies use the same principle at member boundaries. Invalid
fields or variants are represented by recovery nodes, while later valid
members remain in their enclosing declaration and retain ordinary stable IDs.
State declarations recover field-by-field as well, for both the current DSL
and the older constructor-shaped syntax accepted by the prototype.
Settings recover entry-by-entry in the current simple and nested documentation
DSLs as well as the older constructor-shaped syntax. Documentation comments
following an invalid entry remain attached to the next valid setting.
Choice options and file filters are recovery boundaries too; their containing
setting remains available when at least its other entries are valid.
Match expressions recover arm-by-arm, retaining later patterns and the
enclosing expression, statement, block, and function.
Function parameter lists likewise retain valid parameters and the function
body after an invalid parameter or a missing closing parenthesis.
Array literals and function calls recover each comma-delimited expression
independently. A malformed element or argument therefore produces a localized
recovery region while later values and the enclosing statement remain in the
partial syntax tree.
Record literals apply the same behavior at field boundaries. Template strings
recover within each interpolation boundary, so invalid embedded syntax does
not discard later text, later interpolations, or the call containing the
template.
Missing unary or binary operands and malformed parenthesized expressions use
an explicit syntax-only error expression. This keeps the surrounding operator,
initializer, and later statements structurally available to editor tooling.
The checker rejects recovery expressions, so they cannot enter typed HIR or
code generation.
Expression-valued `if` uses the same placeholder for missing conditions, empty
or malformed branches, and a missing required `else`. Consequently the
conditional itself and later statements remain queryable. Every missing
recovery node is represented by a zero-width source span.
At declaration and statement roots, failed expressions synchronize before a
semicolon, closing delimiter, or the next source line and become error
expressions. This retains globals, expression-backed state fields, locals,
assignments, conditions, throws, suspensions, standalone expression
statements, and matches with missing scrutinees. Newline synchronization does
not require the next token itself to look valid, so consecutive malformed
statements remain distinct syntax nodes and diagnostics.

Diagnostics are backend-independent values defined in
[`src/diagnostic.rs`](../src/diagnostic.rs). They carry a stable category code
and severity: `SS0001` for lexical errors, `SS0002` for syntax errors, `SS0003`
for type errors, and `SS0004` for semantic validation after type checking. The
same value owns its primary and secondary labels, notes, and fixes. Fixes have
an explicit applicability and may contain multiple source edits. The repeated
optional/result-postfix diagnostic already supplies a machine-applicable edit
that removes the extra postfix, while the model remains available for future
LSP code actions. The CLI renderer consumes these fields directly. Spans
remain byte offsets within the one source file; a `FileId` and source map are
deliberately deferred until the language gains modules or another feature that
accepts multiple sources.

[`src/database.rs`](../src/database.rs) provides the revisioned single-source
query facade used as the foundation for editor tooling. Recovering parse,
strict parse, declaration lowering, semantic checking, and diagnostics are
cached as shared results. Supplying identical text preserves both the revision
and cached allocations; a changed buffer invalidates all dependent stages.
Diagnostics return the first failing stage, while the recovering parse remains
independently queryable after syntax errors. This is intentionally explicit
invalidation rather than a dependency framework: more granular machinery can
be justified later by measurements and additional consumers.

The database also owns the editor-facing semantic query surface. Clients can
look up declarations by name; request inferred expression, value, and function
result types; inspect semantic type shapes; resolve calls, paths, and
assignment targets; and obtain a cached reference index without navigating the
checked program's internal products. References are deterministically ordered,
distinguish reads from writes, and include value paths and method receivers.
The recovering AST can also be lowered into declaration HIR when strict parsing
fails, preserving valid globals, types, functions, and actions for outline and
name queries. For checked source, `analysis_at` selects the smallest typed
expression containing a byte offset and returns its inferred type shape and
semantic resolution. When syntax is valid but type checking reports errors,
`recovering_check` keeps the diagnostics and publishes all semantic facts that
could still be resolved. Database type, call/path, assignment, position, and
definition queries transparently use that partial model, so an unrelated bad
expression does not disable hover or navigation elsewhere. Recovery-only
fallback types merely make the semantic model representable; strict `check`
continues to reject the program and typed HIR and Wasm lowering are never built
from those fallbacks. Exact identifier segments connect recovered resolutions
to the same source and compiler-catalog definition targets as checked source.
The sibling `format` query reuses the cached strict parse and canonical
formatter, returning shared formatted text or the same syntax diagnostics.
This is the document-formatting boundary intended for the future LSP.

After inference, `check` materializes a `TypedProgram` body HIR. Every
`TypedExpression` contains its stable `ExprId`, source span, and semantic
`TypeId`. Its expression kind owns literal data, operators, and child `ExprId`s;
typed blocks likewise own variables, assignments, branches, returns, suspensions,
and expression statements. Match arms own their pattern, optional guard, and
result expression. Type-directed resolutions are attached to the relevant node:
value-path roots and member chains, user or standard-library calls, record
field order, and enum constructors. Assignments and match patterns similarly
carry their resolved targets, while choice settings carry resolved default and
option variants. Node enumeration is stable-ID ordered for deterministic LSP
and documentation consumers. Implicit conversions live on typed operand edges;
for example, a non-string template interpolation carries an explicit
`ToString` conversion and retains the operand's original `TypeId`.

Focused front-end snapshots render declaration identities, inferred function
signatures, typed body edges, expression types, semantic resolutions, and
implicit conversions without byte offsets or backend indices. A sibling
diagnostic snapshot records ordered, source-located error messages. These
fixtures make intentional semantic changes reviewable while keeping backend
tests centered on validated Wasm and observable runtime behavior.

[`examples/compiler_baseline.rs`](../examples/compiler_baseline.rs) measures
release-mode one-shot compiler latency and deterministic output size across the
minimal, Lunistice, cancellation, and settings fixtures. Results and the exact
methodology live in [`docs/BASELINES.md`](BASELINES.md); timings remain trend
signals rather than platform-sensitive test thresholds.

The backend consumes typed HIR for all user body work: local and async-frame
layout discovery, match scratch planning, string/signature discovery,
statement and expression emission, global constant expressions, and async
polling. Calls, paths, assignments, constructors, and match patterns therefore
arrive with semantic targets already selected; the backend no longer descends
through syntax expressions or re-queries expression-resolution side tables.
Parsed declarations still provide source-level state-pointer and settings
metadata.

Public language metadata is also backend-independent. `StandardLibrary`
catalogs callable APIs, while `LanguageCatalog` catalogs keywords, lifecycle
actions, settings forms, and syntax such as `retry expression`. Their stable
IDs remain distinct, but both use the shared `catalog::Documentation` and
`catalog::Example` model. Catalog validation checks identity and documentation
integrity, and the compiler test suite parses, checks, and lowers every embedded
example before editor or generated-documentation clients can expose it.

The public standard library is authored hierarchically in
[`src/stdlib/catalog.rs`](../src/stdlib/catalog.rs). Each namespace, nominal
type, capability, and type constructor owns its fields, variants, associated
functions, and methods. The declaration macro derives owners, qualified names,
stable symbol IDs, intrinsic IDs, and the flat normalized tables consumed by
the compiler and tooling. Those tables are a compatibility view, not a second
authoring surface. Architecture tests reject reintroduction of the retired
parallel type, field, variant, and item registries.

That Rust macro is intentionally an interim producer for the normalized graph.
The long-term producer will load bundled standard-library modules written in
SplitScript once the language can express modules, generics and capability
bounds, private runtime representation, effect declarations, and reusable
library bodies. Such modules are trusted compiler inputs, not ordinary project
files: only a compiler-created privileged loading path may declare intrinsic
or host bindings, representation hooks, runtime-private fields, or trusted
effects. There is no source directive, CLI switch, import, or shadowable module
name that can grant this authority to user code. A small Rust registry remains
the trust boundary and must validate every privileged binding's signature,
effects, availability, suspension and cancellation behavior, and physical
representation before any user program is checked.

The lower-level host boundary has a separate `AbiCatalog`. It is intentionally
not an LSP or public standard-library input. Each host import records its stable
ID, module and field name, Wasm parameters/results, memory or handle ownership,
lifetime contract, effects, and internal documentation. Wasm import types and
function indices are generated by iterating this catalog; backend call sites
retain only the resulting ID-to-index binding. A conformance test parses the
emitted module and verifies `docs/ABI.md` against the catalog-rendered table, so
the runtime contract cannot silently drift across code and prose.

The first physical backend split follows that boundary:
[`src/codegen/imports.rs`](../src/codegen/imports.rs) owns host-import type
emission and the catalog-order-to-function-index mapping. It returns the import
section, concrete ABI indices, imported-function count, and next free type
index to the final module orchestrator. This keeps catalog interpretation out
of the larger code generator without exposing backend indices to semantic or
editor layers.

[`src/codegen/expression.rs`](../src/codegen/expression.rs) owns structured
block and assignment emission, resolved receiver/path access, every Wasm-IR
expression variant, casts and comparisons, Result/Option control flow, match
lowering, and standard-library intrinsic dispatch. Its parent-facing surface is
the expression context plus entry points for a block, assignment, expression,
or receiver. Action, state-read, and async encoders can therefore reuse
ordinary expression semantics without reaching into their implementation or
re-querying typed HIR.

[`src/codegen/async_state.rs`](../src/codegen/async_state.rs) owns `onAttach`
state-machine emission. It discovers the already-numbered Wasm-IR poll and
resume states, traverses nested continuation blocks, emits builtin suspension
polls and arbitrary `retry T!` polling, stores live values in the GC frame, and
applies the process-lifetime cancellation region. Only the completed
`compile_async_attach` entry point is visible to the final orchestrator;
ordinary values and receivers are delegated through the expression module.

[`src/codegen/gc_types.rs`](../src/codegen/gc_types.rs) owns deterministic GC
layout construction. It emits one recursive group containing the state and
built-in runtime types, the async continuation frame, and only reachable
nominal records, enums, inferred arrays, Options, and Results. `GcLayout` owns
their compact deterministic order and the encoder consumes that same order.
The resulting plan contains the completed type section and first free type
index, which feeds host-import and generated-function type assignment without
making the final orchestrator understand individual GC fields.

[`src/codegen/settings.rs`](../src/codegen/settings.rs) owns the complete
settings lifecycle: host widget registration, nested titles, choices, file
filters and tooltips, settings-map acquisition/freeing, String decoding,
default fallback, and atomic current/old value rotation. It also emits the
start routine that initializes enum globals, source-level state defaults, and
the async frame before registration. Final assembly sees only generated bodies
for String decoding, refresh, and start rather than individual setting kinds.

[`src/codegen/update.rs`](../src/codegen/update.rs) owns the exported per-tick
runtime behavior. It emits process attachment and one-shot detached callbacks,
process-close cancellation/reset, settings refresh, transactional state-field
polling, snapshot rotation, lifecycle action ordering, nullable loading and
game-time handling, and timer start/split/reset commands. Final assembly passes
only resolved read/action function indices and receives one generated update
body, keeping host lifecycle policy out of the section orchestrator.

[`src/codegen/runtime_helpers.rs`](../src/codegen/runtime_helpers.rs) owns the
generated helper library: host/GC String conversion and formatting, structural
String/source-record/catalog-record/enum/Option/Result equality, signature
scanning, address following, relative reads, managed UTF-16 decoding, and
Unity/IL2CPP discovery. A single interface
receives resolved function indices plus discovered String/u64 array layouts and
returns ordered core and equality body plans. The orchestrator inserts settings
bodies between those groups to preserve its already-planned function-index
order without knowing how any helper is implemented.

[`src/codegen/function_plan.rs`](../src/codegen/function_plan.rs) owns the
single generated-function index space. Starting after the catalog-driven host
imports, it declares every helper, settings adapter, structural equality body,
user function, state reader, lifecycle action, and exported entry point in
body-emission order. Optional String-array and u64-array helpers are discovered
there as dependencies. Emitters receive named indices and cannot advance the
type or function counters themselves.

[`src/codegen/script_functions.rs`](../src/codegen/script_functions.rs) emits
ordinary source-defined bodies: transactional pointer and expression state
reads, user functions, and non-async actions. It also owns deterministic
Wasm-local assignment for values, match storage, fallback values, intrinsic
scratch space, and suspension scratch space; expression and async lowering
share that assignment routine without owning index policy.

[`src/codegen/data_plan.rs`](../src/codegen/data_plan.rs) discovers and interns
all static UTF-8 text and parsed signature needles/masks before body emission.
It exposes immutable lookup pools to emitters and alone encodes their linear
memory data segments, keeping memory offsets deterministic and preventing the
final assembler from rediscovering source dependencies.

[`src/codegen/dependencies.rs`](../src/codegen/dependencies.rs) scans resolved
standard-library calls in Wasm IR and closes their generated-helper
dependencies transitively. The first consumer is static-data planning: Unity's
module name and built-in IL2CPP signatures are absent unless `Unity.il2cpp` is
used. Function planning and body generation traverse the same canonical,
filtered helper order; checked helper-index lookups catch missing transitive
edges, and no placeholder signatures or bodies remain. Settings-free programs
also omit their two map-decoding adapters and refresh calls. Tests assert
observable data-segment and function-count behavior rather than exact indices.

The same dependency plan owns required host imports. Fixed process lifecycle
behavior, present action kinds, pointer-backed state fields, resolved
intrinsics, generated helpers, and individual settings kinds contribute stable
`AbiImportId`s. `codegen/imports.rs` filters the declarative ABI catalog by
that set while retaining catalog order, and all body emitters use checked ID
lookups. Consequently a minimal script imports only timer-state inspection and
the three process-lifecycle operations, while richer scripts pull in precisely
the timer, memory, logging, and settings APIs their emitted bodies call.

[`src/codegen/reachability.rs`](../src/codegen/reachability.rs) computes the
source-level live graph before backend dependencies are selected. Lifecycle
actions, expression-backed state fields, and global initializers are roots;
resolved user-function and user-method calls add bodies transitively, including
recursive call cycles. Function planning/body emission and static-data/helper/
import analysis all consume the same result. Dead functions therefore cannot
retain their strings, signature literals, generated helpers, or host imports.
Reachable equality operands additionally close over nested source or catalog
record fields, enum payloads, and Option/Result values; Result errors retain
String equality. Thus
structural-equality signatures/bodies and the String equality helper are emitted
only when a live comparison can call them. The same analysis
collects GC-type roots from state and poll storage, globals, settings, emitted
function signatures/locals, async frames, and live expressions, then closes
over every aggregate member. Compiler-generated layouts such as interpolation's
`Array<String>` are explicit roots rather than accidental side effects.

[`src/codegen/global_plan.rs`](../src/codegen/global_plan.rs) assigns runtime,
source-global, and current/old settings storage. Its result contains the global
section plus the value-to-index/type maps used by lowering, so body generators
cannot allocate globals opportunistically.

GC type construction also returns an explicit `GcLayout`. The map is the sole
source of physical indices and reference/storage types for recursive GC fields,
globals, generated function signatures, ordinary/async locals, aggregate
instructions, memory decoding, settings initialization, runtime helpers, and
failure propagation. The fixed built-in type conversion rejects dynamic GC
types, preventing semantic IDs from being mistaken for physical Wasm indices.
`GcLayout` compactly orders only the reachable records, enums, arrays, Options,
and Results and exposes that same order to type encoding. Removing an earlier
semantic layout therefore remaps all physical Wasm indices centrally without
changing individual emitters.

[`src/codegen/module_assembly.rs`](../src/codegen/module_assembly.rs) is the
final binary boundary. It receives completed type, import, function, global,
code, and static-data plans; adds the fixed memory, `_start`/`update` exports,
and SplitScript metadata; and serializes sections in canonical Wasm order.
The top-level encoder is now a deterministic orchestrator of these plans.

The checker borrows syntax immutably: inferred array layouts are returned as
part of `CheckedProgram` and passed explicitly to code generation rather than
being appended to the parsed AST.

[`src/wasm_ir.rs`](../src/wasm_ir.rs) is the first backend-owned lowering
product. Its bodies contain structured statements plus explicit
`Fallthrough`, `Return`, and `Suspend` terminators. A suspend records whether
its source operation is `await` or `retry` and owns its continuation, so
`onAttach` state-machine construction no longer rediscovers suspension
boundaries from syntax or typed HIR. The same product plans declared
values, match inputs and payload bindings, and numeric-intrinsic scratch locals
with semantic `TypeId`s. Code generation assigns their concrete Wasm local or
GC-frame indices.

Async storage is selected by backward liveness over the lowered suspension
segments. Every `Suspend` records, in deterministic declaration order, the
source locals required by its continuation. The generated GC frame contains
the union of those per-suspension sets; values used only before a suspension,
defined after it, or overwritten before their next use stay in ordinary Wasm
locals. Await bindings are frame-backed only when a later segment reads them.
Keeping the per-suspension sets in the IR also permits future physical slot
coalescing without making the encoder reconstruct data flow.

An `onAttach` body also owns a `ProcessLifetime` cancellation region. Awaited
catalog operations whose normalized `OperationSemantics` cancel on process
close attach their `Suspend`
terminator to that region. The generated runtime consumes the body-level region
when the process closes: it detaches the host handle and atomically resets both
attach readiness and the entire continuation frame. A later process therefore
enters the first suspension segment with fresh values. This ownership is
represented before encoding rather than being rediscovered separately by each
process or Unity intrinsic.

The checker consumes that same normalized catalog query to decide whether an
operation is awaitable and whether it is valid in the current lifecycle. For
example, a process read in `onDetached` is rejected because the catalog requires
an attached process. After typed HIR is built, `OperationAnalysis` computes a
fixed point over resolved user-function and method calls. The process
requirement therefore propagates through forward calls, methods, and recursive
call graphs, and the action boundary rejects the outer invalid call. The result
is exposed by `CheckedProgram::effects` for future hover and navigation. The
public catalog rendering query similarly prevents documentation and LSP clients
from reconstructing these rules independently.

Async control flow is no longer encoded from source statement indices. Lowering
assigns every suspension a stable poll state and a distinct continuation state,
then the encoder emits one dispatcher loop over those states. Reaching a
suspension sets
the poll state before invoking the host operation, so a pending poll resumes at
the operation itself rather than replaying preceding assignments, conditions,
or side effects. A successful poll selects its continuation and redispatches in
the same runtime tick. Conditional branches use continuation-style lowered
tails, allowing a suspension inside either branch to rejoin code after the
conditional without source-level special cases.

`nextTick()` is a lowering intrinsic rather than a host call. Reaching it stores
its poll state and returns pending immediately; dispatching that poll state on
the next update selects the continuation without replaying the preceding
segment. Its catalog cancellation behavior still attaches it to the same
process-lifetime region as process-backed futures.

`retry expression` uses the same state machine but is not a catalog intrinsic.
The checker requires the expression to have type `T!`, the Wasm IR records the
`Retry` suspension mode and plans a typed Result scratch local, and polling
re-evaluates the ordinary typed-HIR expression. An error returns pending; a
success extracts the Result payload into the continuation frame when the
binding remains live. This makes user-defined Result-returning functions
retryable without giving the backend special knowledge of those functions.
Immediate process-memory APIs consistently expose this path: generic reads,
pointer following, relative-address decoding, and managed-string decoding all
return `T!`. Their low-level helpers may still use zero or null sentinels at the
ABI boundary, but expression lowering converts those sentinels exactly once
into the standard Result GC representation. Discovery APIs such as module,
signature, and Unity metadata lookup instead remain intrinsic suspensions:
temporary absence means pending, while process closure cancels their region.

The Wasm IR now owns a stable-ID expression plan alongside its control-flow and
storage plans. Scalar literals, resolved value/member paths, unary and binary
operations, explicit casts, result `TypeId`s, and implicit Option/Result lift
edges are copied into backend-owned nodes. Ordinary emission consumes those
nodes without asking typed HIR which operation or path was selected. String
literals and interpolation, compile-time signatures, arrays, and resolved
record/enum constructors have moved to the same plan; static string/signature
data collection consumes it as well. Calls now carry their argument IDs and
complete resolved target in this plan: user-function IDs, method receiver
paths, standard-library item IDs, and inferred generic arguments are no longer
recovered from typed HIR by ordinary or suspending emission. Expression-valued
`if` uses a native branch node with typed result arms. `else` fallback nodes
preserve value versus returning branches, while postfix `?` nodes carry the
exact inferred Result boundary used for failure transfer. `match` uses
resolved Wasm-IR arms: enum patterns retain stable variant and payload
binding IDs; `None`/`Some` patterns retain Option state plus the present-value
binding; `Ok`/`Err` patterns retain Result state plus the corresponding
value or error binding. Literal/wildcard patterns retain guards and result
expressions. Binding extraction and guard evaluation are nested behind the
pattern condition, so an inactive enum variant or wrapper state is never
accessed. The temporary `TypedHir` expression variant is gone, calls are
encoded directly from their Wasm-IR target, and ordinary expression emission
no longer receives typed HIR. This completes the expression-plan migration
without introducing a second expression identity or a general SSA optimizer.

Process-read failure will remain expressed through the language's real `T!`
result type and ordinary result control flow rather than through a new
backend-only failure channel. This keeps semantic facts and backend physical
types separate.

`T?` and `T!` are already first-class constructed type annotations. Parsing
assigns stable layout identities, inference preserves their value types, and
the semantic store exposes `TypeKind::Option` and `TypeKind::Result` for the
backend and future editor tooling. The backend emits a monomorphized Wasm GC
struct layout for each used wrapper. `None`, `Some(value)`, `Ok(value)`,
`Err(message)`, and implicit optional/successful lifts are ordinary typed
values: every lift is recorded as
a source/target `TypeId` conversion during checking, copied onto its Wasm-IR
expression edge, and then lowered to GC construction. Payload-bearing `Some`
and `Ok` constructors allocate provisional layouts when no expected wrapper is
available. After inference resolves their payload types, equivalent provisional
and declared layouts canonicalize to one stable nominal Wasm GC identity.
Payload-free `None` and success-type-free `Err` still require expected-type
context. Wrapper match patterns are resolved to stable semantic Option/Result
layout IDs before lowering; the backend does not rediscover their meaning from
source spelling. Result-aware control flow remains a separate semantic step;
none of these physical layouts introduce a private failure protocol.

## Syntax traversal

[`src/visit.rs`](../src/visit.rs) is the single exhaustive definition of AST
child traversal. `Visitor` provides immutable preorder traversal for analysis
and collection. `Folder` provides the equivalent in-place mutable traversal
for syntax rewrites. Both cover top-level declarations, blocks, statements,
expressions, match arms and patterns, and written type references.

Each hook descends by default. When overriding a hook, call its corresponding
`walk_*` or `walk_*_mut` function to continue into children. Deliberately omit
that call to establish a boundary. For example, an analysis interested only in
statements can override expression traversal with an empty method, while an
async pass can stop below a suspension statement.

Compiler passes should use these utilities when they need ordinary structural
traversal. A bespoke recursive walker remains appropriate when traversal alone
does not express the algorithm. Definite-return analysis is one example: an
`if` returns on every path only when both branches return, so it needs explicit
control-flow composition rather than a flat node visit.

The typed-HIR traversal users include:

- string and signature literal collection;
- async continuation-frame local collection;
- ordinary Wasm local collection;
- match and numeric-intrinsic temporary planning;
- backend body emission through stable child IDs.

Typed HIR has a sibling `TypedVisitor` whose root traversal covers global
initializers, expression-backed state fields, function bodies, and action
bodies exactly once. AST visitors must not be extended to inspect semantic side
tables implicitly; passes that need types or resolutions should carry the
appropriate checked product explicitly. That keeps syntax traversal useful to
the formatter and early LSP features, which run before successful type checking.

## Formatting

[`src/formatter.rs`](../src/formatter.rs) exposes `format_source` as a reusable
library operation. It first invokes the strict compiler parser, then lays out
the parser's lossless token and trivia stream; there is no formatter-specific
lexer or grammar to drift from the compiler. Literal spellings, ordinary line
and block comments, and `///` setting documentation remain source-backed while
whitespace, indentation, and operator spacing become canonical. Invalid input
returns the compiler's structured parser diagnostics instead of producing a
potentially destructive rewrite.

Parsed header spans distinguish block braces from other braces. A one-line
`if`, `while`, function, or lifecycle header keeps its opening brace on that
line. When the header wraps, continuation lines receive one extra indentation
level and the opening brace moves to its own line at the header indentation;
the body therefore remains visibly nested beneath the brace. Formatter tests
require idempotence and reparsing for the main language showcases.

Formatting metadata is derived from parsed syntax rather than guessed solely
from punctuation. Settings, state fields, and ordinary statement spans
establish continuation regions; these stop at nested block braces so block
bodies are not indented twice. Match, choice, and file blocks identify commas
that terminate domain-specific entries. A delimiter pass separately records
line breaks made directly inside each parenthesized or bracketed expression.
This keeps
multiline process-read arguments one level inside their call without
over-indenting a call merely because a nested argument is multiline. Braces
in the settings DSL keep each label and `=>` on the same line, including
boolean, choice, file, choice-option, and file-filter entries; nested block
contents and closing braces are then anchored from that line. Interpolated
string chunks remain byte-for-byte source text, while expressions inside
`{...}` use the ordinary spacing rules.

The command-line frontend exposes the same operation as `splitc fmt <file>`.
It writes only after parsing and formatting succeed and skips unchanged files.
`splitc fmt <file> --check` performs no write and returns failure when the
canonical text differs, making it suitable for CI.

## Language server

[`src/lsp.rs`](../src/lsp.rs) owns transport-independent LSP state, while
[`src/bin/splitls.rs`](../src/bin/splitls.rs) provides standard
`Content-Length`-framed JSON-RPC over stdin and stdout. This remains a module
and sibling binary in the compiler crate for now, following the architecture
rule that crate boundaries should be introduced only after interfaces have
multiple proven consumers.

Each open URI owns one revisioned `CompilerDatabase`. Full-sync open and change
notifications update that database, publish diagnostics tagged with the
document version, and convert byte spans to LSP's UTF-16 line/character
positions. Diagnostic codes, severities, label messages, notes, and structured
fix edits come from the compiler's existing diagnostic model. Formatting
requests call the cached database formatter and return one whole-document edit;
syntax-invalid documents retain their diagnostics and receive no destructive
formatting edit. The server advertises only implemented capabilities during
`initialize` and observes the standard `shutdown`/`exit` lifecycle.

Semantic highlighting is a compiler query rather than an LSP-specific AST
walk. [`src/highlight.rs`](../src/highlight.rs) merges lossless lexer tokens,
syntax declarations, and semantic resolutions into a sorted index of byte
spans, token kinds, and modifiers. This gives domain constructs—including
settings titles, state fields, lifecycle blocks, signatures, enum variants,
and debug-erased regions—stable classifications that future editor clients can
share. Recoverable syntax errors retain lexical and declaration highlighting;
valid syntax with type errors additionally uses the recovering semantic model.
The LSP layer only splits multiline spans and delta-encodes UTF-16 positions
according to the advertised semantic-token legend.

Completion follows the same ownership rule. [`src/completion.rs`](../src/completion.rs)
returns editor-neutral candidate kinds, snippets, documentation, and byte-span
replacements. Root candidates combine the `LanguageCatalog`, `StandardLibrary`,
source declarations, and bindings visible at the cursor. The lexical walk
includes function parameters, preceding ordinary and suspension bindings,
enclosing-block locals, and active match-pattern bindings while preserving
shadowing and excluding later or sibling declarations. Member completion
handles snapshots and nominal enum names directly, then uses a temporary source
probe with the unfinished member suffix removed to recover the receiver's
semantic `TypeKind`. That supports
record fields, compiler-provided fields, user methods, and catalog methods even
for the normal mid-edit spelling `receiver.`. The LSP adapter converts the
replacement span to UTF-16 and emits standard snippet text edits; it does not
reimplement candidate or documentation rules.

Source navigation is likewise a compiler query. `DefinitionIndex` records the
exact declaration identifier and every exact identifier-token reference by
stable `ValueId`, `FunctionId`, `RecordId`, `RecordFieldId`, `EnumId`, or
`EnumVariantId`. It covers declarations, annotations, patterns, paths, calls,
constructors, record literals, and assignment targets, including recovered
semantic models. `CompilerDatabase::definition_at` resolves the token under the
cursor, while `CompilerDatabase::references_at` filters the same index by
identity and optionally excludes its declaration. The LSP adapter emits
single-document `Location` values and performs only byte-to-UTF-16 conversion.
Standard-library and language-catalog definitions deliberately return no source
location; future generated or virtual documentation can provide a separate
navigation target without weakening source identity.

Rename builds directly on that navigation boundary. `rename_target_at` returns
the exact selected occurrence for LSP `prepareRename`, while `rename_at`
validates the proposed ASCII identifier and rejects language or standard-library
root names. It then applies the proposed edits to an in-memory candidate,
requires the candidate to type-check, and maps every old reference span through
the edit deltas. Each mapped reference must retain its original stable source
ID. Consequently a rename cannot silently capture a global, parameter, local,
field, type, or callable even when the captured program would still be valid.
The LSP layer only converts the returned spans into one same-document
`WorkspaceEdit`; catalog symbols are not renameable.

[`src/symbols.rs`](../src/symbols.rs) owns the editor-neutral document outline.
It converts the recovered syntax tree into source-ordered symbols even though
the AST stores declarations by category. State and settings are domain
containers, setting-title spans recover their nested hierarchy, records and
enums contain fields and variants, and methods and lifecycle blocks retain
distinct kinds. `CompilerDatabase::document_symbols` caches this result; the
LSP adapter recursively converts its byte ranges and selection ranges to UTF-16
`DocumentSymbol` values.

Code actions require no parallel repair model. Compiler diagnostics already
carry titled `DiagnosticFix` values with applicability and one or more exact
`TextEdit`s. The LSP code-action endpoint filters current compiler diagnostics
to the requested source range, honors `context.only`, and maps those edits into
same-document quick-fix workspace edits. The diagnostic attached to an action
is rendered through the same conversion used by publication, keeping its code,
notes, labels, and fix metadata consistent.

[`src/insight.rs`](../src/insight.rs) provides the corresponding hover and
signature-help queries. Hover first uses the shared definition and position
analysis to select a stable source, standard-library, or language-catalog
identity. Source definitions are joined with semantic type facts, so globals,
locals, parameters, state and setting fields, record fields, functions and
methods, records, enums, and variants show their inferred source-level types.
User-function and method hover additionally consumes the interprocedural
`OperationAnalysis`, exposing transitive catalog effects, attachment
constraints, synchronous behavior, and debug-only build availability. Recovery
retains this analysis when type inference succeeds but a later semantic rule,
such as calling an attached-process helper from `onDetached`, rejects the
program.
For a resolved generic call, semantic `TypeId` arguments are rendered back to
source-level names and substituted into the catalog signature. The resulting
Markdown includes parameter documentation, effects and availability, and the
same compiler-validated examples used by generated documentation. Signature
help finds the active unmatched call delimiter and counts commas only at its
own nesting level. If an unfinished method call cannot yet be checked, a
temporary probe removes the partial call, infers its receiver, and selects the
applicable method from `StandardLibrary::methods_for_type`. The LSP adapter only
performs UTF-16 position conversion and JSON shaping.

Standard-library prose is normalized one step earlier in
[`src/documentation.rs`](../src/documentation.rs). A
`StandardLibraryDocumentation` entry contains the canonical generic or
substituted signature, summary and details, parameters, effects, runtime
behavior, deprecation, and compiler-validated examples. It can render the
compact completion description, the signature-help body, or the full hover
payload. An end-to-end JSON-RPC test requests numeric `.clamp` completion,
updates the same open document to a resolved call, and verifies both completion
and hover exactly against the corresponding generated entries. The future HTML
and machine-readable documentation exporters should consume this model instead
of rebuilding catalog presentation.

## VS Code client

[`editors/vscode`](../editors/vscode) is a thin TypeScript extension around the
same stdio server. Its language client selects file and untitled SplitScript
documents and discovers `splitls` from an explicit machine-overridable setting,
a future packaged `server` directory, this repository's debug target, or
`PATH`. Changing the server path or arguments restarts the client. Formatting
and every other dynamic editor feature remain LSP registrations rather than
parallel VS Code providers. Compilation remains a direct CLI operation: the
editor discovers or uses a configured `splitc` and offers two fixed workflows.
Debug watch owns a long-running `splitc watch --profile debug` child process,
rebuilds after saves, and exposes lifecycle state through an editor action and
status-bar stop command. Build Release saves the active file and invokes a
one-shot `--profile release` build. It stops an active watcher before building,
so a subsequent debug rebuild cannot overwrite the release artifact. Both
stream stdout and stderr to a dedicated output channel. Successful builds
produce a neighboring `.wasm` through the CLI's atomic replacement path;
failures leave the previous module untouched.

The extension manifest owns the `.split` association, declarative language
configuration, and fast-startup TextMate grammar. Semantic highlighting is
enabled by default and the compiler's custom `setting`, `settingTitle`,
`stateField`, `lifecycle`, `signature`, and `debug` token types plus the `debug`
modifier are contributed with standard supertypes and fallback theme scopes.
An integration test compares those manifest IDs against
`SemanticTokenKind::ALL` and `SEMANTIC_TOKEN_MODIFIERS`, preventing protocol and
extension drift. Root VS Code tasks build the Rust and TypeScript halves,
compile or watch the active script, and launch an Extension Development Host.
The packaged client additionally contributes fixed debug-watch and release-build
commands to the editor title and context menu, so users do not need the
repository tasks or understand compiler profile flags.
