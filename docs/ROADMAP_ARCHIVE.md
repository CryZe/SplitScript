# SplitScript roadmap

This roadmap is ordered by dependency and impact, not merely implementation
size. Language semantics should settle before editor tooling treats them as a
stable public contract.

Priority meanings:

- **P0** — correctness or foundational design needed by real autosplitters.
- **P1** — important language and API ergonomics once the foundations exist.
- **P2** — tooling, migration, build profiles, and ecosystem scaling.
- **Ongoing** — work that should continuously validate every other priority.

## P0 — Maintainability audit and architectural convergence (2026-07-30)

This audit covers the current compiler, standard library, backend, language
server, extension, tests, and roadmap after the first hierarchical
standard-library migration. The migration was worthwhile, but it exposed
several boundaries that are still nominal rather than enforced. Address these
before another large language or standard-library expansion.

Measured hotspots at the time of the audit:

- `parser.rs`: 3,207 lines / 109 functions;
- `typeck.rs`: 3,055 lines / 72 functions;
- `codegen/runtime_helpers.rs`: 2,621 lines;
- `database.rs`: 1,935 lines / 86 functions;
- `codegen/expression.rs`: 1,725 lines;
- `lsp.rs`: 1,645 lines;
- `wasm_ir.rs`: 1,516 lines;
- `stdlib.rs`, `language.rs`, `formatter.rs`, `hir.rs`, `stdlib/catalog.rs`,
  and `codegen.rs`: each over 1,000 lines;
- `tests/compiler.rs`: 7,414 lines / 129 tests;
- nine codegen submodules import their parent with `use super::*`;
- compiler stages directly construct the global `StandardLibrary` 89 times;
- `IntrinsicId` is matched 48 times in expression emission, 33 times in
  dependency planning, 21 times in Wasm IR, and 15 times in async lowering.

File size is a symptom, not the primary refactoring criterion. Split a file
only after its responsibilities communicate through a named input/output
boundary; do not turn one shared mutable context into many mutually coupled
files.

The intended dependency direction is:

```text
catalog schema -> validated catalog graph -> compiler context
source text -> syntax -> declarations/resolution -> typed HIR
typed HIR -> complete backend IR -> backend plans -> Wasm encoding
editor protocol -> compiler database/query snapshots -> compiler stages
```

No arrow should point backwards, and catalog producers must be replaceable
without changing their consumers.

### 1. Catalog graph, compiler context, and future stdlib loader

- [x] Replace the zero-sized, globally reconstructed `StandardLibrary` with an
  immutable validated graph owned by `CompilerContext` and passed through
  parsing, resolution, checking, tooling, documentation, and backend lowering.
  `StandardLibrary` now owns an `Arc` graph, products are cloneable rather than
  copyable, algorithms borrow the graph, and an independently constructed graph
  is injected through the complete pipeline in tests. Source-loaded declaration
  storage remains active future work rather than a hidden global-lifetime
  assumption.
- [x] Thread the first compiler-context seam through the active pipeline:
  parsed, lowered, checked, and recovered products retain one
  `CompilerContext`; inference, typed HIR, semantic validation, database-backed
  tooling, documentation queries, Wasm IR, backend planning, and emission all
  consume its standard-library handle. The 89 audit-time catalog
  reconstructions are now limited to compatibility entry points,
  context-free AST formatting, and tests. The parent item remains open until
  alternate/source-loaded graphs can be constructed and exercised.
- [x] Make the physical Wasm GC layout a catalog-derived plan. `GcLayout` now
  owns standard and inferred type indices, standard field slots, enum variant
  indices, value representations, and the async-frame position. Emitters no
  longer reconstruct the default catalog, so semantic and physical phases
  cannot silently disagree about the selected standard library.
- [x] Break the former dependency cycles between authored catalog data,
  declarations, public queries, and compiler semantic types. The layers are
  now one-way: one raw hierarchy in `stdlib/source.rs`; independently generated
  opaque IDs in `ids.rs`; dependency-light declaration and callable schemas;
  normalized data in `catalog.rs`; validation and indexed graph consumers;
  then the compiler-specific `stdlib_semantic` adapter.
- [x] Break the compiler-semantic half of that cycle. The standard-library
  schema, authored catalog, declarations, and graph no longer depend on
  `BuiltinType` or semantic `TypeKind`; `stdlib_semantic` is now the one-way
  adapter that owns typed call candidates and receiver applicability for the
  checker, completion, and hover. An architecture test rejects future
  semantic-type imports in backend-neutral catalog modules.
- [x] Split the declaration producer without creating a parallel registry.
  `source.rs` contains the hierarchy once and invokes independent ID and
  normalized-data consumers. `declarations.rs` no longer imports authored
  catalog tables, callable schema lives in `schema.rs`, and catalog-wide
  checks live above both in `validation.rs`. Architecture tests enforce these
  directions.
- [x] Make the authored hierarchy survive as a real normalized graph with
  owner-to-children and name/path indices. The current macro is hierarchical
  only at its input and immediately emits flat slices; most queries repeatedly
  linearly scan those slices. Flat deterministic iteration views may remain,
  but ownership and lookup must be indexed graph facts.
- [x] Decide and implement the ID model required by a source-loaded standard
  library. Rust enum variants are convenient well-known handles but cannot be
  the identity of declarations loaded from SplitScript. Use data-backed stable
  IDs/newtypes plus an explicit table of well-known compiler contracts, or a
  build-time source compilation scheme; do not make every consumer depend on
  generated Rust variants. Catalog symbol identities are now opaque `u32`
  newtypes with generated well-known constants and an internal loader
  constructor; their debug names preserve existing diagnostics. Only
  `IntrinsicId` remains a closed enum because compiler implementations must be
  exhaustively trusted.
- [x] Replace the positional `function_item!` / `method_item!` invocation
  grammar with one named callable declaration nested under its owner. Kind,
  generic parameters, value parameters, result, effects, availability,
  documentation, intrinsic binding, and the focused public example now live
  together in `stdlib/source.rs`; `catalog.rs` contains only the normalizing
  consumer and shared full-program example-validation fixtures. An
  architecture test rejects the retired callable factories and verifies that
  every bundled intrinsic has the complete named metadata shape.
- [x] Generalize the catalog type-expression model. `stdlib::TypeRef` now has
  atoms for core/nominal/parameter identities plus one recursive
  `Application { constructor, arguments }` form keyed by an open
  `StdlibTypeConstructorId`; Array, Option, and Result are declared unary
  constructors rather than closed expression variants. Rendering, inference,
  receiver applicability, hover substitution, and expected-Result inference
  consume that shape, while catalog validation checks constructor identity,
  arity, recursive arguments, and parameter scope. Future generic catalog
  constructors no longer require another `TypeRef` variant, although their
  semantic/runtime implementation still needs an explicit trusted contract.
- [ ] Use `StdlibCapabilityId` directly in generic bounds and replace the
  closed `TypeConstraint::{Numeric, MemoryReadable}` adapter. Replace
  inference's fixed `Requirements` mapping and `capabilities.rs` hard-coded
  capability switch with one extensible capability solver/registry that can
  describe marker, structural, and custom privileged capabilities.
  - [x] Generic bounds now contain catalog capability IDs directly; catalog
    validation rejects missing IDs and the retired `TypeConstraint` enum is
    gone.
  - [x] Inference requirements are a deduplicated capability-ID set rather
    than a fixed eight-bit mirror, so source-loaded IDs cannot collide or be
    silently ignored.
  - [x] Each capability declaration now selects `Declared`, structural
    equality, or structural memory-layout behavior. Inference admissibility,
    method discovery, and final semantic validation dispatch through that
    descriptor instead of matching the Equatable/MemoryReadable IDs in each
    phase.
  - [ ] Add the trusted custom-capability handler registry when the first
    capability cannot be expressed by declared membership, structural
    equality, or structural memory layout. Validate handler bindings exactly
    like intrinsic call bindings rather than adding another ID switch.
- [x] Normalize effects into a validated effect set rather than an arbitrary
  slice. Reject contradictions such as `Pure` plus writes/allocation, derive
  suspension/cancellation facts once, and cross-check intrinsic/helper/host
  effects so a public declaration cannot understate its implementation.
  - [x] Public callable effects now use a canonical deduplicated `EffectSet`
    with deterministic iteration. Catalog validation rejects empty sets,
    `Pure` combined with any observable effect, retry/suspend conflicts,
    impossible cancellation, process reads without attachment, and invalid
    onAttach availability; `OperationSemantics` derives suspension,
    attachment, and cancellation from that normalized set.
  - [x] Cross-check each intrinsic's declared effects against its trusted
    lowering/helper/host descriptor once the intrinsic registry below owns
    those implementation facts.
    The trusted intrinsic contract owns and exactly validates public effects
    and availability. The runtime-helper registry now recursively derives the
    observable timer, process, and runtime effects of every direct helper and
    ABI root, rejects unsupported ABI effect categories, and requires exact
    agreement with the trusted contract before Wasm emission.
- [ ] Give all standard-library symbols one documentation/link identity and
  validate examples for namespaces, types, fields, variants, capabilities,
  constructors, and callables. Callable docs currently use `StdlibItemId`
  links while other symbols use `StdlibSymbolId`, and only callables require
  examples.
  - [x] Use `Documentation<StdlibSymbolId>` for callables as well as every
    other library symbol, and validate related links against the complete
    symbol graph. Cross-kind links no longer need a callable-only adapter.
  - [ ] Author and compile focused examples for non-callable symbols, then
    require examples uniformly without padding documentation with artificial
    shared fixtures.
- [x] Generate the ABI and language-catalog IDs and tables from their
  declarations as well. `AbiImportId` currently has a manual enum, manual
  `COUNT`, and order-sensitive table; `LanguageItemId` likewise precedes a
  separate item table. Reuse common catalog validation/index/documentation
  infrastructure without pretending ABI imports or syntax keywords are
  standard-library functions.
  - [x] Make the ABI declaration list generate `AbiImportId`, `ALL`, `COUNT`,
    indices, and the normalized import table together. The public ID API and
    deterministic import order are unchanged, but adding an import can no
    longer omit its ID or require a manually synchronized count.
  - [x] Normalize the heterogeneous language-item, built-in-type, compiler
    symbol, and lifecycle-action declarations behind one authored source, then
    generate their identities and item views without erasing the useful
    `BuiltinType`/`ActionKind` relationships. One grouped declaration macro now
    accepts ordinary syntax, built-in types, compiler-provided snapshot roots,
    and lifecycle actions, then generates `LanguageItemId` and the ordered item
    table together. Payload identities remain typed rather than being flattened
    into strings or pretending syntax is standard-library API.

### 2. Intrinsic and generated-runtime architecture

- [x] Introduce a small trusted intrinsic registry independent of the public
  stdlib declaration source. Each intrinsic contract should own its expected
  signature shape, effects, availability, lowering class (ordinary, retry,
  suspension, host boundary, or representation primitive), scratch needs,
  helper roots, and host-import roots. Validate privileged stdlib bindings
  against it before checking user code.
  - [x] Generate an exhaustive `IntrinsicId::ALL` view and require one closed
    Rust `IntrinsicContract` per ID. Contracts now own callable shape,
    generic/value arity, exact `EffectSet`, availability, and lowering class;
    standard-library validation rejects mismatched or orphaned public
    bindings.
  - [x] Move direct generated-helper and host-import roots into the contracts.
    Backend dependency planning now interprets those roots and no longer
    matches `IntrinsicId`; adding a call implementation cannot silently omit
    its direct runtime dependencies.
  - [x] Move synchronous and suspension scratch policy into the contracts.
    Wasm-IR local planning now interprets typed scratch policies (core,
    expression, or Result payload plus slot count) instead of maintaining two
    more `IntrinsicId` switches.
  - [x] Move complete parameter/result shape into the contracts. Trusted
    signatures now describe generic parameters by ordinal, capability bounds,
    typed-function selection, method receivers, recursive constructor
    applications, every parameter type and literal rule, and result type.
    Catalog validation ignores cosmetic generic names but rejects any
    implementation-relevant mismatch. Transitive helper/ABI effects are also
    checked against the exact public effect set.
- [x] Lower a resolved stdlib call to an explicit backend call operation once.
  Wasm IR now owns `CallTarget` and converts semantic `ResolvedCall` at its
  input boundary; a standard-library target stores both stable item ID and
  trusted intrinsic ID plus concrete type/receiver facts. Codegen no longer
  imports or rematches front-end `ResolvedCall`, dependency analysis consumes
  contract roots, and local planning consumes contract scratch policies. The
  final expression/async emitter dispatch remains deliberately exhaustive
  until emitter strategies move into the contract registry.
- [x] Replace the parallel generated-helper enum/order, transitive dependency
  matches, signature match, and body-emission match with one helper descriptor
  registry and a body plan. Function signature order and code-body order must
  be represented by the same plan rather than reproduced by separate loops.
  - [x] Give every helper one descriptor containing its stable identity,
    deterministic order, symbolic Wasm signature, direct helper dependencies,
    direct ABI imports, and body builder. Intrinsic contracts should refer to
    that same identity rather than maintaining an `IntrinsicHelper` mirror.
  - [x] Build one dependency-closed, ordered `RuntimeHelperPlan` per backend
    compilation. Allocate function indices from that plan and emit bodies by
    iterating the exact same entries, including settings helpers; a missing or
    extra body must become structurally impossible rather than an ordering
    convention between `function_plan.rs`, `runtime_helpers.rs`, and
    `codegen.rs`.
  - [x] Validate the descriptor graph for duplicate identities, dependency
    cycles, forward references that violate body availability, and ABI roots.
    Keep focused tests for the Unity dependency closure and emitted
    signature/body parity. `runtime_helper_registry.rs` is now the only
    canonical helper iteration order and owns symbolic signatures, direct
    helper/ABI roots, and body-builder callbacks. Intrinsic contracts use the
    same `RuntimeHelperId`; dependency closure interprets descriptors; function
    planning resolves their signatures into one ordered `RuntimeHelperPlan`;
    and body emission iterates those exact entries, including settings
    adapters. Registry tests reject missing/duplicate identities, duplicate
    roots, and dependencies ordered after callers, while architecture checks
    reject the former per-phase helper matches.
- [x] Add an explicit linear-memory layout plan. `LinearMemoryLayout` now packs
  typed scratch roles into validated primary/companion alias classes, sizes
  ABI output from the largest readable layout and scan overlap from the actual
  signature set, page-aligns immutable data after scratch, and derives initial
  pages plus growable host-string staging from the complete layout. String and
  signature pools store relative offsets until this plan relocates them, so
  scratch growth cannot invalidate embedded pointers. Stress tests cover
  multi-page static strings, scratch growth from a readable record larger than
  one page, long planner-level signatures, bounds, and alias placement.
- [x] Complete the collision-safety slice of that plan: reserve the first Wasm
  page for centralized runtime scratch regions, place immutable data after it,
  derive minimum pages from the collected payload, validate wasm32 bounds, and
  stress-test a string large enough to require three pages. Typed scratch
  handles and workload-derived packing are completed below.
- [x] Replace fixed runtime global indices and scratch-address constants with
  typed plans (`RuntimeGlobals`, `LinearMemoryLayout`, and planned scratch
  handles). Emitters should not know that `current` happens to be global 1 or
  that string scratch happens to begin at 32 KiB.
  - [x] Have `global_plan` allocate and return a typed `RuntimeGlobals` value
    for process, current/old snapshots, attach readiness, async frame, and the
    detached-entry latch. Expression, async, settings, and update emitters now
    receive those named roles; the six parent-module numeric constants are
    gone and changing global declaration order cannot silently retarget them.
  - [x] Replace remaining numeric scratch addresses with typed regions from
    `LinearMemoryLayout`, including alignment, size, ownership/lifetime, and
    checked accessors for each helper/settings operation.
    - [x] Centralize the formerly named constants as `ScratchRegion` values for
      settings length/string decoding, signature scanning, C strings, and
      managed UTF-16/UTF-8. Their bounds and required non-overlaps are checked
      when the layout is planned and consumers receive regions explicitly.
    - [x] Place unbounded host String staging at the first page after immutable
      data and grow memory before writes. Long `print`/`setVariable` values can
      no longer overwrite static strings or signatures beginning at page 1.
    - [x] Replace raw address-zero ABI read destinations across process, Unity,
      state, and async emitters with one aligned `AbiReadScratch` role. Every
      read proves its complete output size fits, all decoding uses named bases,
      and the aliasing contract requires callers to materialize values before
      a nested synchronous read. An architecture test scans every process-read
      emitter and rejects anonymous destinations or address-zero loads.
    - [x] Replace the remaining fixed first-page coordinates with a packed
      region planner and explicit primary/companion alias classes. Capacity is
      derived from readable layouts and collected signature lengths; fixed API
      limits such as the existing 255-byte signature diagnostic remain
      source-facing. Settings decoding, scanning, C-string checks, ABI reads,
      and managed UTF input share the primary class only when their synchronous
      phases are mutually exclusive, while settings length/managed UTF output
      occupy the disjoint companion class.
- [x] Centralize Unity/IL2CPP version layouts and discovery signatures in a
  validated domain descriptor. `codegen/unity_layout.rs` now owns the accepted
  version rows, pointer width, versioned field-count/static-table offsets, the
  invariant assembly/image/class/field memory schema, module name, discovery
  scan windows and displacements, and every built-in signature. Static-data
  collection, attachment, synchronous helpers, and async static-table polling
  consume those identities/facts. Descriptor validation rejects duplicate
  versions/signatures, malformed signatures, bad alignment/scalar sizes, and
  inconsistent object strides; an architecture test prevents the former
  repeated literals from escaping back into Wasm emitters. Adding a supported
  64-bit layout is now one validated version-table row rather than a numeric
  hunt through instruction builders.
- [x] Split `codegen/runtime_helpers.rs` by genuine runtime domain after the
  registries/plans exist: strings/UTF conversion, equality, process memory and
  signatures, settings adapters, and Unity metadata. Give each module explicit
  inputs and returned bodies; no `use super::*`. The descriptor callback layer
  is now a 145-line orchestrator. String/formatting, structural equality,
  process-memory/signature/managed-string, Unity metadata, and Unity attachment
  implementations live in explicit-import modules, all below 1,000 lines;
  settings remains in its existing dedicated adapter module. This split was
  performed only after `RuntimeHelperPlan` made function identity, dependencies,
  signatures, and body order an enforced interface.
- [x] Remove all codegen `use super::*` imports. Move shared backend types into
  deliberately named modules and make each encoder depend only on its plan and
  narrow emission context. The current physical file split does not enforce
  the architectural boundaries described in `docs/COMPILER.md`.
  - [x] Remove every production wildcard parent import and make each codegen
    module state its crate, sibling, and parent dependencies explicitly. This
    also removed the accidental `codegen.rs` prelude: symbols no longer need to
    be imported by the root merely so children inherit them.
  - [x] Give shared backend concepts deliberate owners. `backend_type.rs` owns
    physical Wasm value categories; `gc_layout.rs` owns deterministic GC index
    assignment; `equality_plan.rs` owns equality function indices;
    `runtime_helper_registry.rs` owns `RuntimeHelperPlan`; `async_frame.rs`
    owns suspension-frame storage; expression lowering owns match-local
    storage; `global_plan.rs` owns setting globals; and `context.rs` owns the
    immutable post-plan emission and attachment contexts. `codegen.rs` is now
    an orchestrator rather than a miscellaneous backend type warehouse.
  - [x] Replace uses of the complete `EmissionContext` where an encoder only
    needs a smaller plan-specific view. Settings and per-tick update now expose
    `SettingsContext` and `UpdateContext`; runtime-helper construction receives
    only semantic facts, settings inputs, its helper plan, data pools, GC plan,
    and linear-memory plan. Script-body, expression, and async emission retain
    the complete context because they genuinely span locals, globals, calls,
    equality, process memory, state snapshots, and suspension storage.

### 3. Front-end and semantic stage boundaries

- [x] Make parsing purely syntactic. `parser.rs` currently pre-scans top-level
  declarations, allocates semantic declaration/layout IDs, knows the standard
  library, resolves record/enum constructors, and emits some redeclaration
  diagnostics (sometimes with a default span). Move declaration collection,
  nominal lookup, reserved-name checks, and constructor resolution into a real
  resolution/lowering stage; syntax type paths should retain source names.
  - [x] Establish an enforceable diagnostic boundary: parser recovery now
    contains grammar diagnostics only, while post-syntax declaration
    validation owns core/standard/duplicate nominal conflicts, precise name
    spans, and secondary labels. Parsed, lowered, recovered, and database
    products retain those resolution diagnostics independently; formatting
    remains available for syntactically valid but semantically conflicting
    source, and strict checking reports the conflict. Record/enum declaration
    identities are assigned while parsing rather than borrowed from the token
    pre-scan. Source type annotations now retain arbitrary nominal names and
    their source spans; unknown-name diagnostics belong to resolution, and
    recovering checking uses a total error placeholder instead of relying on a
    parser-enforced `unreachable!`. Record literals now retain their nominal
    spelling/span in syntax and publish the resolved `RecordId` through the
    semantic model; parsing recognizes their `field:` grammar shape without a
    declaration lookup.
  - [x] Remove the parser's declaration/catalog pre-scan and enum
    classification completely. Parsed enum match/settings references retain an
    `EnumReference::Named` spelling/span, and ordinary two-segment paths/calls
    remain ordinary syntax. `resolution::resolve_program`, invoked by `lower`,
    resolves source and catalog enums, rewrites enum constructors, validates
    choice-setting restrictions/constructor arity, and publishes resolved
    references. Typed HIR owns a separate `TypedPattern` with concrete
    `EnumTypeId`s, so unresolved syntax cannot leak into Wasm lowering. Source
    record, enum, and constructed-layout IDs now start in their own typed
    identity spaces without a token-count offset scan. The final substep below
    records why parser-owned constructed IDs remain syntax rather than layout.
  - [x] Classify parser-assigned declaration, expression, binding, and
    constructed-type IDs explicitly as stable syntax identities. Array,
    Option, and Result tables intern source type-expression structure only;
    they do not allocate inferred semantic types, memory layouts, or Wasm GC
    types. Resolution may replace nominal references in the lowered copy, and
    checking/backend stages retain independent identities. This keeps visitors,
    diagnostics, and editor queries stable without moving syntax-node identity
    into a later semantic pass.
- [x] Replace the minimal declaration-only HIR with a clear resolution product
  or remove that stage. Today `lower` mostly indexes declaration names while
  `typeck` repeats declaration-environment construction and performs most name
  resolution, making the advertised `syntax -> resolved HIR -> typed HIR`
  pipeline misleading.
  `LoweredProgram` is now the explicit resolution product: it owns syntax with
  nominal type names, enum constructors/patterns/settings, and record literal
  identities resolved to stable source/catalog IDs, plus resolution
  diagnostics. The former ambiguously named `hir::Program` is now the
  deliberately narrow `hir::DeclarationIndex`; it supports pre-check tooling
  but no longer pretends to be a resolved body IR. Type checking consumes the
  resolved syntax, does not rebuild the nominal type-name environment, and
  publishes a distinct typed body HIR with resolved calls, members, patterns,
  conversions, and types. Type-directed call/member/binding inference remains
  in the checker by design rather than being mislabeled as syntactic lowering.
- [x] Split `typeck.rs` around explicit products: declaration/signature
  collection, body checking, expression constraints, call/member resolution,
  exhaustiveness/control-flow, and finalization. Replace the large `Checker`
  state machine's interacting booleans (`in_function`, `checking_suspension`,
  `checking_state_source`, `allowing_null`, and others) with scoped context
  enums/guards so invalid mode combinations are unrepresentable.
  - [x] Extract the first declaration product from the flat checker state.
    `typeck/declarations.rs` now owns source nominal declarations, named-type
    bindings, state/settings/global bindings, user function/method signatures,
    and debug-callable identity as one `DeclarationEnvironment`; lexical scopes
    and transient body modes remain separate.
  - [x] Replace the mutually stale `in_function`, `current_action`, and
    `current_callable` fields with one `CallableContext`, and replace the
    independently combinable `checking_suspension` / `checking_state_source`
    flags with one `ExpressionMode`. Replace the independent optional failure
    boundary and `used_propagation` flag with `FailureContext`, so propagation
    use cannot exist without the result boundary it targets. `NonePolicy` now
    names the distinction between ordinary optional construction and the two
    domain-nullable action results instead of carrying an unexplained
    `allowing_null` flag.
  - [x] Make all transient checker modes scoped. Debug-only code, loop depth,
    suspension/state-expression mode, nullable-return policy, failure
    propagation, and callable/return contexts now enter through restoring
    helpers; checking one field or callable cannot leak state into the next.
    `DebugContext` and `LoopContext` replace the remaining boolean/counter
    conventions, while `FailureContext` returns propagation evidence from its
    exact boundary.
  - [x] Extract real passes rather than merely distributing one state machine.
    A 95-line driver initializes the checker and sequences
    `declaration_pass`, `body_pass`, and `finalization`; declaration and
    signature collection, global/state/function/action bodies, statements and
    lexical scopes, expression constraints, call/member resolution,
    syntax-level control-flow facts, and semantic publication each have a
    named owner. The audit-time 3,055-line root is now 576 lines and no
    type-checking module exceeds 1,000 lines. The complete 219-test Rust suite
    and warnings-denied Clippy pass across this boundary.
- [x] Move post-type-check semantic validation out of `lib.rs::check` into a
  named validation stage that owns effect, detached-call, equality, memory,
  and generic-capability diagnostics. Strict and recovering checks should
  share the same stage boundaries and publish the same available facts.
  `validation.rs` now returns one `ValidationOutput` containing derived
  capabilities, operation effects, and diagnostics. Strict checking consumes
  it before publishing `CheckedProgram`; recovering checking runs the same
  stage whenever typed HIR is available, retains effects even when validation
  rejects the program, and reports the same detached/capability diagnostics.
- [x] Reconcile the repeated type universes deliberately. Core primitives are
  manually repeated in `CoreTypeId`, syntax `TypeRef`, `BuiltinType`, and the
  backend physical `Type`; constructed types are converted through several
  parallel matches. Generate mechanical primitive mappings from one core
  declaration and document the genuinely necessary syntax/inference/semantic/
  physical distinctions. `with_core_types!` now authors each magical core
  primitive once and generates `CoreTypeId`, ordered metadata (canonical name,
  capabilities, and memory layout), and the backend's deliberately physical
  variants/conversion. Syntax stores `TypeRef::Core(CoreTypeId)` rather than
  repeating thirteen variants; semantic `BuiltinType` is a descriptive alias
  of that same ID rather than another enum. Constructed syntax, semantic, and
  physical types remain distinct because they carry stage-specific layout and
  representation facts.
- [x] Reassess expression duplication across syntax `ExprKind`, typed
  `TypedExpressionKind`, and Wasm `ExpressionKind`. Typed HIR should retain
  semantic source shape, but the backend IR should contain completed backend
  operations rather than a third mostly source-shaped copy. Add a Wasm-IR
  visitor/folder so reachability, dependencies, data collection, and local
  planning do not each grow another recursive switch for every expression.
  The lowered IR remains a stable-ID expression DAG because it owns backend
  call targets, conversions, failure boundaries, and match layouts that syntax
  cannot represent. `wasm_ir::Visitor` now owns recursive program/block/
  statement/terminator/expression traversal, while
  `visit_expression_children` is the one exhaustive direct-edge definition
  for worklist analyses. Reachability consumes that edge query; local and
  intrinsic-scratch planning and suspension-frame liveness consume the lowered
  visitor rather than reopening typed HIR. Declaration stores are explicit IR
  facts, so storage planning no longer guesses from an assignment-shaped node.
  Dependency and static-data planning deliberately scan the flat reachable
  expression table and match only payloads they consume; they do not duplicate
  recursive child traversal.
- [x] Make `lower_wasm` produce the complete backend input. It now returns a
  `codegen::BackendProgram` that owns the profile-specific Wasm IR and borrows
  the matching syntax, semantic model, constructed-type layouts,
  memory layouts, equality analysis, and standard-library identity. The
  product dereferences to its Wasm IR for staged inspection, while binary
  encoding accepts only the complete product, preventing callers from mixing
  unrelated earlier-stage results. Once global constants, local/scratch
  planning, and frame liveness migrated to Wasm IR, the backend product also
  stopped retaining typed HIR merely as an escape hatch.
- [x] Replace the code generator's eight positional entry arguments with a
  named product boundary. The temporary `codegen::Inputs` migration seam was
  superseded by `BackendProgram`; new inputs cannot be silently reordered or
  independently threaded into encoding.
- [x] Reduce cloning and duplicated recovery paths. `CompilerDatabase` rebuilds
  products by cloning syntax and repeats “strict check or recovering check” in
  many queries. Publish a shared semantic snapshot/view that exposes whichever
  facts survived, while keeping strict compilation incapable of consuming
  recovery placeholders.
  `SemanticSnapshot` now owns the strict/recovered choice once and provides
  shared syntax, source-document, semantic-model, enum, context, effect, and
  optional typed-HIR views. Position analysis, navigation, highlighting,
  hover, signature help, and definition indexing consume that product. Hover
  retains the shared snapshot rather than cloning whole syntax and semantic
  models; strict compilation and the typed reference index still require a
  checked program deliberately.

### 4. Tooling, tests, and repository-scale maintenance

- [x] Split `database.rs` into the revision/query cache, semantic snapshot
  access, definition/reference indexing, and rename validation. Completion,
  insight, highlighting, and symbols should consume stable query interfaces
  rather than reach through the database into stage internals.
  `database/queries.rs` owns source revisions and stage/query orchestration,
  `cache.rs` owns one invalidated-per-revision cache product, `snapshot.rs`
  owns the editor-safe semantic view, `position.rs` owns cursor analysis,
  `references.rs` owns typed value references, and `rename.rs` owns
  identity-preserving edits. The remaining 932-line database root owns the
  stable source-definition index and resolution mapping; no database module
  exceeds the soft 1,000-line threshold, and production modules use explicit
  imports rather than parent glob coupling.
- [x] Replace the monolithic raw-`serde_json::Value` LSP handler with typed
  protocol DTOs (prefer the maintained `lsp-types`/`lsp-server` ecosystem if
  its cost is acceptable), a request router, document store, and conversion
  modules. Malformed parameters should produce consistent protocol errors
  rather than ad hoc `None` paths. Keep transport framing separate.
  The transport remains isolated in `bin/splitls.rs`; `lsp/protocol.rs`
  deserializes the JSON-RPC envelope and every supported incoming parameter
  shape, the root routes named methods and lifecycle state,
  `lsp/documents.rs` owns open buffers and their compiler databases, and
  `lsp/conversion.rs` owns byte/UTF-16 and compiler-product serialization.
  Request decoding consistently returns `-32600` for malformed envelopes and
  `-32602` for malformed method parameters. The production root fell from
  1,645 audit-time lines (including tests) to 574 lines; its 750-line protocol
  suite now has a dedicated module. A direct `serde` DTO layer was sufficient
  for the current small method surface, so adopting a larger protocol crate is
  deferred until it removes more code than it adds.
- [x] Split the VS Code extension into language-client discovery/lifecycle and
  compiler build/watch task management. Model build/watch state with one task
  controller instead of module-level booleans and process handles, and add
  extension tests for completion of builds, watcher exit races, and disposal.
  `languageClient.ts` now owns server discovery plus start/stop/restart,
  `compilerTasks.ts` owns release/watch UX and processes, and `extension.ts` is
  activation wiring only. One discriminated compiler-task controller replaces
  the independent release boolean, watcher handle, and status globals.
  `ExclusiveTaskState` uses task identity so a delayed close/completion event
  cannot clear a newer owner; Node tests cover exclusion, stale watcher events,
  and idempotent completion, while TypeScript strict checking covers controller
  disposal and command wiring.
- [x] Split `tests/compiler.rs` by stable subsystem (parsing/recovery,
  inference/checking, catalogs/tooling, lowering/codegen) and extract shared
  source/Wasm assertions. Keep cross-stage tests, but make failures point to
  one architectural area.
  The 7,805-line integration-test root is now a 47-line shared-fixture/module
  index. Ten named modules own compiler queries/navigation, parser recovery,
  catalogs/types, migration diagnostics, failure semantics, profiles/codegen,
  expressions/control flow, inference/language, async/runtime behavior, and
  snapshot rendering. Shared fixtures remain defined once, the complete 138
  tests retain one integration binary, and failure names now include their
  subsystem module.
- [x] Add one repository verification command (`cargo xtask check`, `just
  check`, or equivalent) that runs formatting, Clippy with warnings denied,
  all Rust tests, VS Code TypeScript checks, both release examples, Wasm
  validation, and every Node runtime harness. The Node harnesses are currently
  documented/manual and there is no CI configuration, so ordinary `cargo test`
  does not protect runtime behavior.
  `cargo xtask check` now owns that exact matrix. Its runner uses an isolated
  target directory so Windows can execute nested Cargo commands without trying
  to replace the running `xtask.exe`; generated modules live only under the
  ignored `target/verify`. Lunistice is compiled once as a publishable release
  artifact and once in debug for the harness that deliberately asserts debug
  attachment messages. The complete command passes locally, including base and
  DLC Lunistice host simulations and every Option/Result/async/settings/profile
  runtime fixture.
- [x] Add CI using that exact verification command and cache only disposable
  build outputs. Never commit generated `.wasm`/`.wat` files.
  `.github/workflows/check.yml` runs the same `cargo xtask check` entry point on
  Windows, caches only npm's disposable download data, and rejects tracked
  `.wasm`/`.wat` artifacts before verification. Toolchain setup is explicit;
  the workflow does not maintain a second hand-written test matrix.
- [x] Keep compile-time, warm-query, generated-Wasm-size, and LSP latency
  baselines. The one-shot runner records compiler/Wasm size, while the tooling
  runner generates a 500-function source and measures cached database and
  in-process JSON-RPC queries. Large-catalog scaling remains paired with the
  alternate-graph ownership task in the active roadmap.
- [x] Narrow the crate's public surface after internal interfaces stabilize.
  `compiler` and `tooling` facades now classify the public products; root
  implementation modules are private and integration tests consume the same
  facades. Keep one crate until two real consumers need independently versioned
  APIs.
- [x] Split this roadmap into a short active `TODO.md` and this archived design/
  completion history. The former 1,800-line roadmap is preserved here while
  the root file contains only active and deliberately deferred work.
- [x] Establish a soft 1,000-line module review threshold and responsibility
  checks, not a hard mechanical limit. The audit-time priorities now have
  named boundaries: the parser root is 410 lines with declarations,
  statements, expressions, types, and recovery modules; the checker root is
  576 lines with explicit passes; runtime helpers are split by host domain;
  the database root is 932 lines with cache/snapshot/navigation/query modules;
  the LSP root is 574 lines with protocol/document/conversion modules; and the
  compiler integration-test root is a 47-line subsystem index. A file crossing
  the threshold prompts an ownership review, but cohesive tables/visitors and
  test suites are not split solely to satisfy a line counter. Remaining large
  modules are listed in `docs/COMPILER.md` as candidates for the next
  interface-led change rather than treated as an emergency mechanical pass.

### Audit baseline and immediate repair

- [x] Inventory module/file sizes, catalog construction, dependency cycles,
  standard-library and intrinsic fan-out, stage inputs, test entry points, and
  the current runtime/compiler verification surface.
- [x] Restore a warning-free `cargo clippy --all-targets -- -D warnings`; the
  dependency/toolchain update currently reports an unnecessary lazy boolean
  closure in `database.rs`.
- [x] Begin with the validated catalog graph/compiler-context seam, while
  fixing the linear-memory overlap as an independent correctness slice. These
  two changes unblock the future stdlib loader and make backend refactoring
  safe; large-file decomposition follows their interfaces.

## P0 — Unify the standard-library declaration and type model

Do this before any further substantial standard-library expansion. This work
turns `StandardLibrary` from the former callable-only catalog into the source
of truth for namespaces, nominal types, fields, enum variants, capabilities,
and runtime representations. The remaining unchecked items finish unifying
source type resolution, inference, and derived capabilities around that graph.

Preserve the current language and generated behavior while establishing this
invariant:

> Adding an ordinary standard-library nominal type requires one declaration.
> Intrinsic behavior may additionally require one deliberately scoped
> implementation, but must not require parser, inference, checker,
> documentation, LSP, or physical-layout declarations.

### Hierarchical standard-library authoring model

The compiler and tooling now consume one normalized symbol graph, but the Rust
source that creates that graph is still fragmented: types, namespaces, fields,
variants, callable items, owner links, qualified names, and intrinsic IDs live
in parallel blocks. The architectural goal is not complete until the authoring
model mirrors the API hierarchy as cleanly as the consumer model does.

- [x] As the immediate migration, replace the parallel `declare_standard_types!`,
  `declare_standard_namespaces!`, `declare_standard_fields!`,
  `declare_standard_variants!`, and `declare_standard_items!` inputs with one
  hierarchical declarative Rust macro. This is an authoring adapter for the
  normalized symbol graph, not the intended permanent source format.
- [x] Make each nominal type declaration contain its documentation,
  capabilities, value-usage policy, runtime representation, public and
  runtime-private fields, enum variants, associated functions, and instance
  methods. Opening `Module`, `Duration`, or `UnityClass` must reveal its whole
  public and physical API in one place.
- [x] Give root functions, namespaces, nested namespaces, core-type extension
  methods, capabilities, and type constructors equally explicit owner blocks.
  `process.read`, numeric methods, address methods, and array methods must not
  remain a flat exceptional list.
- [x] Derive owners and qualified names from declaration nesting. A member
  declaration must not repeat `StdlibOwner::Type(...)`, `Module.scan`, and
  `Module` independently.
- [x] Generate `StdlibNamespaceId`, `StdlibTypeId`, `StdlibFieldId`,
  `StdlibVariantId`, `StdlibItemId`, `IntrinsicId`, and the flattened
  `NAMESPACES`/`TYPES`/`FIELDS`/`VARIANTS`/`ITEMS` tables from the hierarchical
  source. Generated flat tables remain an internal compatibility layer for
  generic consumers, not an authoring surface.
- [x] Keep intrinsic implementation bodies deliberately separate, but bind
  their generated intrinsic key alongside the owning function or method.
  Validation must prove that every declared intrinsic has exactly one backend
  implementation and that no implementation is orphaned.
- [x] Migrate every existing declaration, including the test-only ordinary
  catalog record, then delete the retired macros, duplicated owner/name data,
  and manual intrinsic-ID list.
- [x] Add architecture tests showing that representative types with fields,
  variants, associated functions, and methods are declared in owner blocks yet
  remain resolvable, documentable, completable, and code-generatable through
  the normalized graph.
- [ ] Long term, define the standard library in SplitScript source and make its
  loader produce the same normalized symbol graph as the interim Rust macro.
  Add the prerequisite language features deliberately: modules/namespaces,
  generic declarations and capability bounds, declaration-only intrinsic and
  host functions, effect metadata, private runtime fields, attached
  documentation, and ordinary reusable library bodies.
  Organize those bundled sources as domain modules (`core`, process memory,
  timer/runtime, Unity, and future engines) rather than recreating the interim
  single-file token stream, which is already close to 1,000 declarative lines.
- [ ] Compile bundled standard-library sources in an explicit privileged mode,
  never as ordinary project code. Only that mode may declare intrinsic or host
  bindings, representation hooks, runtime-private fields, trusted effects, and
  other low-level implementation details. User files must be unable to enable
  the mode, import its private surface, shadow the bundled library, or call raw
  intrinsic entry points.
- [ ] Keep a small Rust intrinsic/host registry as the trust boundary. Loading
  the SplitScript standard library must resolve every privileged declaration
  against that registry and verify its signature, effects, availability,
  suspension/cancellation behavior, and representation contract. Reject
  unknown, duplicate, orphaned, or understated bindings before compiling user
  code.
- [ ] Once the SplitScript source loader covers the complete library, delete
  the interim Rust declaration macro. Rust should retain only core primitive
  definitions, backend-neutral representation primitives, the host ABI
  catalog, and deliberately scoped intrinsic lowering implementations.

### Library declaration graph

- [x] Extend `StandardLibrary` from a callable catalog into a complete,
  backend-independent symbol graph with stable IDs for namespaces, nominal
  types, fields, runtime-private slots, enum variants, callables, capabilities,
  and intrinsic implementations.
- [x] Give every callable an explicit owner—root, namespace, nominal type, or
  capability—instead of deriving namespaces from string path prefixes.
  Declare `process`, `timer`, and `Unity` as namespaces; keep `Duration` and
  the Unity value types as nominal types with associated or instance members.
- [x] Replace catalog `Builtin(BuiltinType)` and `Named("...")` references with
  stable core-type, standard-type, type-parameter, and constructed-type
  identities. Names are lookup/display data, never semantic identity.
- [x] Describe public fields and runtime-private storage in the owning type
  declaration. Runtime metadata must be backend-neutral: scalar, GC struct,
  GC array, enum, compile-time-only, and derived record representations rather
  than `wasm_encoder` types or numeric heap/field indices.
- [x] Generate namespace, standard type, field, variant, and callable IDs
  together with their declaration rows, so adding an ordinary symbol cannot
  leave a parallel ID enum or inverse owner list out of sync.
- [x] Add catalog validation for unique IDs and names, resolvable owners and
  type references, valid representation dependencies, field/variant identity,
  complete public documentation, capability consistency, and intrinsic
  signature/implementation agreement.

### One semantic type universe

- [x] Restrict compiler-core types to genuine language primitives and
  constructors: `void`, `bool`, fixed-width numbers, `address`, inference
  variables, arrays, `T?`, and `T!`. Model `String`, `Duration`, `Module`,
  `TimerState`, and the Unity family as declared nominal library types;
  represent `Signature` as a single declared compile-time intrinsic type.
- [x] Preserve unresolved nominal type paths in syntax and resolve them against
  one environment containing core, standard-library, and source declarations.
  The parser must not enumerate future standard-library type names.
  - [x] Keep source-written standard-library type names nominal in syntax and
    resolve them to catalog identities when entering inference/semantics.
  - [x] Apply the same name-resolution boundary to source record/enum names:
    source annotations now carry an interned nominal-name identity and resolve
    alongside catalog names only when entering semantics.
  - [x] Consolidate constructor, enum-pattern, choice-setting, and nominal-type
    lookup into one declaration environment rather than parallel parser maps.
- [x] Simplify inference to known semantic `TypeId` values plus inference
  variables and the minimum temporary constructor terms needed while a
  constructed type's element/value remains unresolved. Remove the parallel
  nominal variants and conversion tables in `ast::TypeRef`, `inference::Type`,
  and `BuiltinType` as their migrations complete.
  - [x] Build one semantic `TypeStore` before inference and represent every
    standard-library nominal inference case as its canonical `Type::Known(TypeId)`.
    Checked publication preserves that identity directly; no library type has
    a dedicated inference or semantic variant or a post-inference conversion
    table.
  - [x] Represent source record and enum inference types with the same known
    semantic `TypeId` values used by checked programs; nominal inference no
    longer has separate standard/source variants.
  - [x] Represent core primitives with their canonical semantic `TypeId` as
    well; inference no longer has parallel primitive or nominal variants.
  - [x] Intern resolved array, Option, and Result terms during inference while
    retaining only the minimum temporary constructor terms needed while their
    element/value types are still unresolved.
- [x] Introduce well-known type/variant handles for genuine language and ABI
  contracts such as string literals, interpolation, `gameTime`, signature
  literals, and timer-state conversion. A well-known handle references the
  catalog declaration; it does not redeclare its name, fields, variants,
  capabilities, nullability, or representation.
- [x] Generalize equality, process-memory layout, interpolation/string
  conversion, and future traits into capability queries over semantic
  `TypeId`. Derive record/enum capabilities from their members where
  appropriate rather than matching concrete library types.
  - [x] Declare core and standard-type capabilities once in the catalog and
    make inference constraints, callable applicability, casts,
    interpolation, memory-read eligibility, and equality query them instead
    of maintaining concrete-type lists.
  - [x] Treat inference checks for source records, enums, and wrappers as
    conservative admissibility only, then prove them through one recursive
    semantic-`TypeId` capability query. Preserve precise equality failures and
    process-memory layouts as capability evidence for diagnostics and backend
    planning; validate catalog type-parameter constraints generically.

### Generic members, layouts, and tooling

- [x] Resolve standard and source fields through one declaration query and one
  stable member identity. Remove `BuiltinFieldId` and the type checker’s
  `Module`/Unity field-name tables.
- [x] Make completion, hover, signature help, go-to-definition, semantic
  highlighting, rename reservation, and generated documentation traverse the
  same symbol graph. Remove standard types and fields from `LanguageCatalog`;
  that catalog should contain only keywords, syntax, lifecycle/actions, and
  other genuinely language-defined concepts.
- [x] Plan reachable Wasm GC layouts from semantic type declarations. Backend
  code must query type and field layout IDs rather than use fixed constants
  such as `UNITY_IMAGE_TYPE` or numeric field indices.
- [x] Make intrinsic lowering request its actual temporary values and query
  declared representations. Remove unconditional Unity scratch locals and
  helper signatures that reconstruct library types independently.
- [x] Keep exact-type branching only inside behavior that is intrinsically
  type-specific. Such code may reference stable standard type/field/variant
  IDs, but it must not restate their source names, semantic shapes, storage
  layout, or documentation.

### Vertical migration and removal order

- [x] Establish catalog declarations and adapters without changing source
  syntax or runtime behavior; add characterization tests for compiler queries,
  docs/LSP results, GC layouts, and generated Wasm before deleting old paths.
- [x] Migrate explicit `process`, `timer`, and `Unity` namespaces and switch all
  editor/documentation discovery away from inferred callable path prefixes and
  hard-coded namespace lists.
- [x] Migrate `TimerState` first as the enum/variant proving case. Remove its
  synthetic AST enum, checker injection, name-based backend lookup, and
  duplicate `LanguageCatalog` entries.
- [x] Migrate `Module` as the field and nominal-GC-record proving case. Its
  `address` and `size` members and physical slots must come from one
  declaration.
- [x] Migrate `UnityModule`, `UnityImage`, `UnityClass`, and `UnityField`
  together, including public fields, runtime-private ownership references,
  methods, async temporaries, generated helpers, and GC layout planning.
- [x] Migrate `Duration`, then `String`, then `Signature`, accounting for their
  lifecycle/ABI contracts, non-nullability, literal/interpolation behavior,
  array representation, and compile-time-only behavior without duplicating
  their declarations.
- [x] Delete the retired standard-type variants, conversion matches, fixed GC
  indices, numeric field indices, special member tables, and duplicate
  language/tooling documentation after each vertical slice. Do not retain
  compatibility aliases; the language is not yet in production.
- [x] Finish with full compiler, formatter, generated-documentation, LSP,
  extension, example-autosplitter, Wasm validation, and runtime regression
  coverage. Add a test proving that a new ordinary catalog record with fields
  becomes resolvable, documentable, completable, and code-generatable without
  adding a concrete-type match elsewhere.
  - [x] A test-only `CatalogRecordProbe` declaration exercises nominal
    resolution, semantic `TypeId` identity, public fields, go-to-definition,
    hover documentation, completion, derived process-memory layout,
    structural equality helpers, and valid Wasm GC generation. Its ID is not
    referenced by production compiler or tooling code.
  - [x] Re-run the full Rust suite, formatter check, VS Code TypeScript check,
    release compilation of both example autosplitters, and Wasm validation.
    The existing Node 24 Lunistice harness still reaches its previously
    characterized null-dereference in the unchanged attachment runtime; the
    generated module validates and this refactor does not alter that runtime
    path.

## Completed foundation — Conditional state expressions

### Expression-valued `if`

- [x] Make `if` an expression with type inference across its branches.
- [x] Evaluate only the selected branch. This is essential for memory reads:
  an inactive branch must not touch the target process.
- [x] Use ordinary expression syntax in `state` rather than inventing a
  state-only conditional around individual fields.
- [x] Allow a state field to combine edition-specific representations into one
  enum value. The Lunistice state should become conceptually:

  ```text
  levelOrScene = if isDlcDemo {
      LevelOrScene.Scene(process.readManagedString(
          process.read(gameManagerInstance.offset(levelOrSceneOffset)),
          128
      ))
  } else {
      LevelOrScene.Level(process.read(
          gameManagerInstance.offset(levelOrSceneOffset)
      ))
  }
  ```

  This fixes the current bug where both `level` and `currentScene` are read from
  the same union-like location even though only one representation is valid.
  Implemented in [`examples/lunistice.split`](examples/lunistice.split); its
  runtime test verifies a 4-byte base-game read versus an 8-byte DLC pointer
  read at the shared location, never both.

## P0 — Compiler and standard-library architecture checkpoint

Do this checkpoint before adding `Option`, `Result`, generic reads, traits, or
another substantial batch of standard-library APIs. The original prototype
was useful for discovering the language, but did not scale: its parsed AST was
mutated in place with inferred types, calls remained string paths, type
checking and code generation resolved built-ins independently, and
[`src/codegen.rs`](src/codegen.rs) owns ABI declarations, runtime generation,
async lowering, layout, helper selection, and expression emission in one file.
`min`, `max`, and `clamp` make the problem especially visible: their names,
typing rules, temporary-local requirements, and lowering are manually
recognized in several unrelated functions.

This is an architectural refactor, not a language redesign. Preserve the
current source syntax and generated behavior with regression tests while
introducing the following boundaries.

### Staged compiler pipeline and semantic model

- [x] Replace the single mutable AST pipeline with explicit products:
  `syntax AST -> resolved HIR -> typed HIR -> lowered Wasm IR -> WebAssembly`.
  Syntax nodes should describe what the user wrote; inferred types, resolved
  symbols, coercions, and selected calls belong in semantic IR.

  - [x] Add an inspectable declaration-level HIR at `lower`, keep syntax
    immutable during type checking, and return inferred backend layouts in the
    checked product instead of appending them to the parsed AST.
  - [x] Materialize checked statement and expression resolution into typed body
    HIR nodes. Paths, calls, members, constructors, assignments, patterns, and
    choice settings carry their stable semantic targets directly; resolutions
    that depend on inference are intentionally attached after checking.
  - [x] Publish typed HIR with `TypeId` results and explicit coercions, then
    make the backend consume it instead of syntax plus semantic side tables.

    - [x] Give every checked expression a stable-ID-ordered typed HIR node with
      its `TypeId`, span, and optional type-directed resolution. Migrate backend
      path, assignment, constructor, and match-pattern identity lookup to it.
    - [x] Move expression/statement shape and child ownership into typed HIR,
      including match arms and async statements. Add typed-HIR traversal and
      migrate backend string/signature discovery to it as the first structural
      consumers.
    - [x] Represent interpolation-to-string conversion explicitly on the typed
      operand edge, migrate ordinary statement/expression emission and async
      lowering together, and remove backend syntax walks and expression
      resolution side-table lookups from user bodies.
- [x] Give declarations, types, standard-library items, and call targets stable
  IDs. A typed call must identify a user function/method or a standard-library
  item directly; the backend must never resolve `Vec<String>` paths again.

  - [x] Assign stable per-program `ExprId` values during parsing and key
    resolved standard-library calls by expression identity rather than source
    spans.
  - [x] Assign stable `FunctionId` values to user functions and methods, resolve
    every user call to its callable ID, and make the backend dispatch through a
    single ID-indexed Wasm function table rather than function/method names.
  - [x] Assign stable `ValueId` values to globals, parameters, locals, await
    bindings, state fields, and settings. Expose ordinary and snapshot-aware
    path roots for go-to-definition and use ID-keyed local, async-frame, state,
    settings, and global storage for backend reads.
  - [x] Assign stable `AssignmentId` values, publish assignment targets as
    `ValueId`, and make local, async-frame, and global writes consume semantic
    targets rather than resolving names again in the backend.
  - [x] Assign `ValueId` values to match payload bindings, include the resolved
    receiver root and semantic receiver type in method-call facts, and lower
    receivers directly through ID-keyed storage without backend name maps.
  - [x] Assign typed IDs to records, enums, record fields, enum variants, and
    built-in fields. Publish semantic member chains for paths and method
    receivers, and make record literals and enum constructors expose their
    resolved field/variant IDs to the backend.
  - [x] Assign `PatternId` and `SettingChoiceOptionId` values, publish resolved
    enum variants for match arms and choice defaults/options, and remove their
    backend variant-name lookup.
- [x] Separate syntactic type references, private inference types, semantic
  `TypeId` values, and backend physical value categories. Constructed semantic
  types retain the stable declaration/layout identities needed by tooling and
  Wasm GC lowering.

  - [x] Introduce the inference-free `TypeId` / `TypeKind` interner used by
    checked semantic call resolutions and editor-facing queries. Move catalog
    built-ins and typed-path arguments to the independent `BuiltinType` model.
  - [x] Store every resolved expression type as a semantic `TypeId`, expose it
    by `ExprId`, and make the Wasm backend consume the semantic facts.
  - [x] Keep omitted global/local/await/state/parameter/function-result
    annotations absent in checked syntax. Publish every inferred declaration
    type by `ValueId` (and each function result by `FunctionId`) in the semantic
    model, and make the Wasm backend consume those facts.
  - [x] Publish record-field, enum-payload, and array-element layouts as
    ID-keyed semantic `TypeId` facts, give arrays a dedicated `ArrayTypeId`, and
    make the Wasm backend consume those facts instead of AST layout types.
  - [x] Introduce inference-free `ast::TypeRef` values for every source
    annotation, cast target, and integer suffix. Keep parser-owned array type
    references separate from checker-owned inferred array layouts.
  - [x] Remove the temporary `Expr::ty` slot and synthetic-expression escape
    hatch. Record pending expression types directly by `ExprId` during checking
    and resolve them into the semantic model without mutating or revisiting the
    syntax tree.
  - [x] Move temporary types and inference variables completely out of `ast`.
    A dedicated inference context now owns union/find unification, requirement
    composition, literal bounds, defaulting, and inferred array layouts.
  - [x] Remove `TypeStore`'s parallel legacy representation. Wasm storage/value
    selection now lowers semantic `TypeId` / `TypeKind` values into
    backend-local physical categories without importing inference types or
    reading source annotations.
- [x] Keep bidirectional inference and expected-type propagation, but express
  them through a reusable constraint/unification layer. Standard-library
  overloads and methods must submit the same constraints as ordinary language
  constructs instead of having one-off branches in `Checker::call`.

  - [x] Extract the reusable inference context and make ordinary expressions,
    declarations, and standard-library generic constraints share it.
  - [x] Replace the remaining procedural overload/method selection branches
    with declarative candidates that submit constraints to the same context.
  - [x] Defer record-member resolution when a receiver is still an inference
    variable, then solve it from later call sites or a unique combination of
    accessed fields. `fn levelTimeText(parts) { parts.minutes }` now infers
    `LevelTimeParts` from `levelTimeText(current.levelTimeParts)` without an
    annotation; genuinely ambiguous shared field names receive a focused
    diagnostic instead of an arbitrary nominal-type guess.
- [x] Add shared AST visitor and folder utilities. String/signature collection,
  await discovery, local collection, and later tooling passes should not each
  need another exhaustive recursive walker whenever a new expression kind is
  added. Give resolved/typed HIR sibling traversal utilities when those IRs are
  introduced rather than pretending the syntax visitor can traverse them.
- [x] Expose the stages through a compiler facade (`parse`, `lower`, `check`,
  and `codegen`) while retaining `compile` as the convenient one-shot API.
  Stage results must be inspectable without invoking the WebAssembly backend.

### One standard-library catalog

- [x] Replace name-based `Builtin::resolve` and method-name special cases with
  a declarative, backend-independent `StandardLibrary` catalog. Each item needs
  one canonical record containing:

  - a stable symbol ID and qualified name;
  - item kind, receiver (for methods), parameters, return type scheme, generic
    variables, and capability constraints;
  - availability and effects such as pure, process-reading, suspending,
    lifecycle-restricted, or debug-only;
  - summary, full documentation, parameter documentation, examples,
    deprecation/migration information, and links to related items;
  - an implementation key: ordinary SplitScript body, compiler intrinsic, or
    host ABI operation. The catalog may name an intrinsic, but backend code is
    the only layer that knows how that intrinsic becomes Wasm.

- [x] Provide read-only catalog queries for exact lookup, method lookup by
  receiver/capability, symbol enumeration, signature rendering, and
  documentation retrieval. Type checking, diagnostics, generated docs, LSP
  completion, hover, and signature help must all call these APIs rather than
  maintain parallel lists.
- [x] Model language-only concepts—keywords, action/lifecycle blocks, the
  settings DSL, and syntax forms—in a sibling `LanguageCatalog` using the same
  documentation/example model. They are not fake functions, but the docs and
  editor still need a uniform way to discover them. First-class control flow
  such as `retry expression` belongs here rather than in `StandardLibrary`.
- [x] Validate the catalogs at test time: IDs and canonical names are unique,
  references resolve, every intrinsic has a backend implementation, every
  public item has documentation, and every example parses and type-checks.
- [x] Migrate `min`, `max`, and `clamp` first as the proving case. They should
  be numeric methods with a shared constrained type variable and stable
  intrinsic IDs (or ordinary generic library bodies once those are
  expressible). Remove all raw checks for those names from type checking,
  temporary-local collection, and expression emission.
- [x] Then migrate every existing built-in and type-directed method. Adding a
  normal library API after this point should require one catalog declaration
  plus either a source body or one deliberately scoped backend intrinsic—not
  edits across parser, checker, code generator, documentation, and LSP code.
- [ ] Once modules and generic library functions exist, allow ordinary
  standard-library declarations and bodies to be authored in SplitScript and
  compiled into the same validated symbol graph. Keep most future library
  functionality there; reserve compiler intrinsics for representation
  primitives, host boundaries, and suspension/control-flow operations that
  cannot be ordinary source code. The interim hierarchical Rust declaration
  macro must feed the same graph so this later source migration replaces only
  the producer, not every compiler and tooling consumer.

### ABI, lowering, and code-generation boundaries

- [x] Describe host imports in a declarative ABI catalog containing their Wasm
  signature, ownership/lifetime contract, effects, and documentation. Generate
  import declarations and host-backed standard-library bindings from it, and
  make [`docs/ABI.md`](docs/ABI.md) a generated/verified view rather than a
  second hand-maintained source of truth.
- [x] Introduce a small Wasm-oriented lowering IR with explicit locals,
  blocks/terminators, coercions, and suspension points. This
  should replace direct typed-HIR-to-encoder emission and the remaining ad hoc
  scratch-layout planning, and prepare for
  `else return`, transactional `Result` handling, profile erasure, and a real
  async state-machine pass; a general optimizer or target-independent SSA IR is
  not required yet.

  - [x] Expose an inspectable `lower_wasm` stage with structured statements,
    `Fallthrough`/`Return`/`Suspend` terminators, and explicit suspension
    continuations. Make ordinary action/function emission and `onAttach`
    state-machine construction consume it.
  - [x] Plan value locals, async-frame fields, match inputs/payload bindings,
    and numeric-intrinsic scratch locals in the lowering product using semantic
    `TypeId`s. Assign concrete Wasm indices only in the encoder.
  - [x] Lower expression operations and typed coercion edges into the Wasm IR.
    Do not add a backend-only failure channel while doing this; result-aware
    control-flow edges belong after the language's real `T!` semantics.

    - [x] Add a stable-ID expression plan and migrate None/bool/integer/float
      literals, resolved value/member paths, unary operations, binary
      operations, explicit casts, semantic result types, and implicit
      Option/Result lift edges. Ordinary encoding consumes these nodes without
      re-querying their typed-HIR operation or path resolution.
    - [x] Migrate String literals/interpolation with explicit String-conversion
      edges, signature literals and their data collection, arrays, resolved
      record-field constructors, and resolved enum-variant constructors.
    - [x] Migrate call arguments and resolved call targets, including user
      function IDs, method receivers/member chains, standard-library item IDs,
      and inferred generic type arguments. Ordinary and suspending call
      emission no longer queries typed HIR for semantic call resolution.
    - [x] Migrate expression-valued `if`, including nested `else if` branches
      and value-producing GC/reference branches.
    - [x] Migrate `else` fallback branches and postfix `?` propagation,
      preserving value, `return value`, bare `return`, and the exact inferred
      Result boundary in Wasm IR.
    - [x] Migrate `match` with resolved enum variants, payload binding IDs,
      literal/wildcard patterns, guards, and result arms. Remove the temporary
      `ExpressionKind::TypedHir` boundary and the expression encoder's
      typed-HIR context.
- [x] Split code generation by responsibility only after the typed-HIR and
  lowering interfaces exist: ABI/imports, GC type/layout planning, ordinary
  expression lowering, async/state-machine lowering, generated runtime,
  standard-library intrinsic lowering, and final Wasm encoding. Moving the
  existing functions into many files before establishing those data flows
  would only redistribute the coupling.

  - [x] Extract host-import type emission and stable ABI function-index
    assignment into `codegen/imports.rs`, driven solely by `AbiCatalog`.
  - [x] Extract deterministic GC type/layout construction into
    `codegen/gc_types.rs`. Return the completed recursive type section and next
    free type index as an explicit backend plan covering state, built-ins,
    async frames, records, enums, arrays, Options, and Results.
  - [x] Extract final section assembly after generated functions, globals,
    exports, and data dependencies are represented as explicit backend plans.
  - [x] Extract ordinary block/assignment/expression emission and
    standard-library intrinsic dispatch into `codegen/expression.rs`, around
    the completed Wasm-IR expression plan and a narrow set of entry points used
    by actions, state reads, and async polling.
  - [x] Extract `onAttach` async/state-machine emission into
    `codegen/async_state.rs`, around Wasm-IR suspension states, continuation
    blocks, `retry`, and process-lifetime cancellation regions. Keep polling
    and state traversal private behind one orchestrator entry point.
  - [x] Extract settings registration, host-map refresh, value decoding,
    current/old rotation, tooltips/filters, and start-time initialization into
    `codegen/settings.rs`, exposing only its three generated function bodies.
  - [x] Extract the per-tick update runtime into `codegen/update.rs`: process
    attach/detach, cancellation cleanup, settings refresh, transactional state
    reads, snapshot rotation, lifecycle action ordering, and timer dispatch.
  - [x] Extract generated String, formatting, equality, scanning, address,
    managed-memory, and Unity/IL2CPP helpers into the
    `codegen/runtime_helpers/` domain modules. Generate runtime bodies from one
    descriptor-backed `RuntimeHelperPlan` and structural equality bodies from
    its separate type-directed plan.
  - [x] Extract generated-function signature and index planning into
    `codegen/function_plan.rs`. Allocate helper, settings, equality, user,
    state-read, action, start, and update functions in one deterministic pass;
    body emitters only consume its named indices.
  - [x] Extract state-read, user-function, and ordinary action body generation
    plus Wasm-local assignment into `codegen/script_functions.rs`.
  - [x] Extract global/data planning and final section assembly so
    `codegen::compile` becomes only the deterministic module orchestrator.
- [x] Track dependencies between selected library items, generated helpers,
  strings/signatures, and host imports so the backend can eventually emit only
  what a program uses. Deterministic output remains required.

  - [x] Introduce a backend dependency analysis over resolved standard-library
    calls and generated-helper edges. Use it to omit Unity module-name and
    IL2CPP signature data from scripts that never call `Unity.il2cpp`.
  - [x] Make generated function planning and body emission consume the helper
    set so unused helper signatures and placeholder bodies are removed. Helper
    calls use checked dependency-index lookups, and settings-free scripts omit
    both settings adapter functions and their update/start calls.
  - [x] Derive the required host-import set transitively from emitted helpers,
    state sources, settings, lifecycle behavior, and direct intrinsics. Emit
    the filtered subset in ABI-catalog order and use checked import lookups;
    setting registration/value imports are filtered by individual kind.
  - [x] Compute source-function and expression reachability from actions,
    state expressions, and global initializers, following user-function and
    user-method calls transitively. Filter user function signatures/bodies,
    source strings/signature literals, helpers, and host imports by that set.
  - [x] Filter generated structural-equality signatures/bodies from reachable
    `==` / `!=` operand types, recursively retaining nested record/enum and
    String equality dependencies. A minimal script now emits no equality body.
  - [x] Introduce an explicit `GcLayout` plan returned by GC type construction;
    use it for recursive field storage, globals, generated function signatures,
    and expression/action local types while preserving the current layout set.
  - [x] Replace emitter-side semantic-ID index arithmetic with `GcLayout`
    lookups for aggregate construction/access, defaults, memory reads, settings,
    generated helpers, and failure propagation. Dynamic GC types now fail fast
    if they reach the fixed built-in type conversion path.
  - [x] Filter inferred GC layouts by reachable storage, signatures, and
    expressions, with transitive closure through record fields, enum payloads,
    arrays, Options, and Results. Compiler-generated helper layouts are
    explicit roots, and `GcLayout` owns both compact ordering and encoding.

### Refactor safety and scope

- [x] Capture compile-pass and runtime behavior for every currently supported
  feature before changing the pipeline, including both Lunistice layouts,
  settings changes, await cancellation, transactional failed reads, numeric
  methods, matches, strings, and GC values across suspension.
- [x] Add focused snapshots for resolved/typed HIR and diagnostics. Backend
  tests should assert observable Wasm behavior instead of depending on
  incidental function/type indices.
- [ ] Keep this as internal modules in the existing crate initially. Split into
  syntax, HIR/type-system, standard-library, compiler, Wasm backend, CLI, docs,
  and LSP crates only when the new interfaces are stable and at least two
  consumers need them. Crate boundaries should enforce proven architecture,
  not be used to discover it.
- [x] Record compile-time and generated-Wasm-size baselines. Correctness and a
  clean semantic API come first, but the catalog/query design must not require
  repeatedly scanning all symbols or regenerating all helpers for each editor
  request.

## P0 — `Option`, `Result`, and explicit failure

The semantic type representation, catalog type schemes, typed HIR, and basic
lowering IR from the architecture checkpoint are prerequisites for this work.

### `Option` and `Result`

- [x] Add `T?` as the spelling of an optional constructed type. Preserve its
  value type and monomorphized Wasm GC layout in semantic queries.
- [x] Use `None` to construct the empty option and `Some(value)` as an optional
  explicit present-value constructor. A plain `T` automatically lifts into
  `T?` when the expected type is optional. Require an annotation or other
  expected-type context when a bare `None` has no constraint on `T`.
- [x] Add `T!` as a result constructed type with a standard language error
  payload. Preserve its value type and monomorphized Wasm GC layout in semantic
  queries; the error type is deliberately not generic initially.
- [x] Use `Err("message")` to construct failure and `Ok(value)` as an optional
  explicit success constructor. A plain `T` automatically lifts into the
  successful result when `T!` is expected. Require an annotation or other
  expected-type context when `Err` has no constraint on its success type.
- [x] Reject adjacent repeated postfix constructors (`T??`, `T!!`, `T?!`, and
  `T!?`) with a focused diagnostic. Nested constructed types remain possible
  through an enclosing type, such as `[T?]!`, without giving adjacent
  punctuation an ambiguous meaning.
- [x] Record optional/successful lifts explicitly on typed-HIR expression edges
  with source and target `TypeId`s, and lower empty, successful, and failed
  values to their WebAssembly GC representations.
- [x] Define structural equality for both wrappers when `T` supports equality:
  Options compare empty/present state and present values; Results compare their
  success/error state, successful values, or standard error strings.
- [x] Define exhaustive matching for both wrapper types. Options use `None` and
  `Some(value)`; Results use `Err(error)` and `Ok(value)`. `_` remains a full
  wildcard, and guarded arms do not satisfy exhaustiveness.
- [x] Extend bidirectional inference so payload-bearing `Some(value)` and
  `Ok(value)` constructors work without expected-type context. Canonicalize
  provisional wrapper layouts after inference so later annotated uses share one
  nominal Wasm GC type. Payload-free `None` and success-type-free `Err(error)`
  still require context because their missing type cannot be inferred.
- [x] Decide which standard-library operations return `T?` versus `T!`.
  Immediate process operations whose attempt can fail return `T!`: fixed-layout
  reads, pointer following, relative-address decoding, and managed-string
  decoding. Suspended module/signature/Unity discovery yields `T` because
  temporary absence is its pending state and process closure is cancellation.
  Future one-shot lookup APIs where absence is an expected completed outcome
  return `T?`; none of the current catalog operations have that contract.
- [x] Preserve transactional state polling: an unhandled failed state read must
  skip the entire snapshot rather than commit partially updated fields.
- [x] After `T!` is represented in typed HIR, make result propagation explicit
  in the lowering IR. Replace the hidden `READ_FAILED_GLOBAL` writes emitted by
  individual process-read call cases with ordinary `Result` construction and
  handling. The state transaction boundary may initially preserve the same
  all-or-nothing commit behavior, but it must consume the language-level result
  path rather than a second backend-only error channel.

### `else` unwrapping and control flow

- [x] Support a low-precedence `else` operation for `T?` and `T!`, serving the
  common roles of `unwrap_or` and Rust's `let ... else` without method noise.
- [x] Permit a value fallback:

  ```text
  let name = optionalName else "Unknown"
  ```

- [x] Permit `return` as diverging control flow in the fallback:

  ```text
  let module = findModule() else return Err("module not found")
  ```

- [x] Add `while condition { ... }` statement loops as the foundation for loop
  control flow. Conditions are checked before every iteration and loop bodies
  are lexically scoped.
- [x] Add statement-form `break` and `continue` with nearest-loop behavior,
  including from nested conditional blocks. They are currently rejected
  outside loops.
- [x] Add direct `else break` and `else continue` fallback branches. Expression
  lowering carries structured branch targets through nested expression `if`,
  match arms, and short-circuit expressions. `else` is otherwise
  the lowest-precedence, right-associative expression operation; expression
  `if` owns its braced `else`, while `else return` is a diverging fallback.
- [x] Produce a useful diagnostic when `else` is applied to a non-optional,
  non-result value.
- [x] Add postfix `?` for `T!`. It unwraps success and propagates the original
  error to the nearest typed failure boundary. A state-field assignment and a
  function returning `T!` are currently boundaries.
- [x] Add `throw error` as the underlying error control-flow primitive for
  `T!` functions. Thrown errors are typed independently from `Err(error)`, which
  constructs an ordinary result value. Explicit `throw` and the failure arm of
  `value?` share one failure-transfer lowering operation rather than separate
  implicit-return implementations.
## P0 — Typed process deserialization

Start this only after standard-library calls resolve through the catalog and
typed HIR. Record layout/deserialization should be a reusable semantic service,
not another collection of `process.read` branches inside code generation.

### One generic `process.read`

- [x] Replace primitive-specific calls such as `process.read.i32(address)` with
  `process.read(address)` and infer the result from its expected type.
- [x] Keep suffix paths such as `process.read.i32(address)` as the explicit type
  escape hatch for contexts where inference has no constraint.
- [x] Make synchronous and retried reads share the same type-directed API and
  clear failure semantics.
- [x] When inference is ambiguous, explain which annotation would resolve it
  and show an example in the diagnostic.

### Records as readable memory layouts

- [x] Introduce a compiler-known `MemoryReadable` / `Deserializable` capability.
  Every primitive memory type implements it.
- [x] Automatically make a record readable when all its fields are readable.
  The initial layout is field order with explicit, documented size and padding
  rules.
- [x] Read a record with one host `process_read` call, then deserialize its
  fields locally. Besides being faster, this gives a coherent snapshot.
- [ ] Once real target layouts require it, add declarative layout controls for
  exact field offsets, explicit padding/packing, per-record or per-field
  little-/big-endian decoding, and eventually custom decoding, without changing
  `process.read` call sites. Keep natural layout as the zero-annotation default.
- [x] Add a record for Lunistice's adjacent clock fields and replace three
  reads with one:

  ```text
  record LevelTimeParts {
      minutes: f32
      seconds: f32
      hundredths: f32
  }

  levelTimeParts: LevelTimeParts = process.read(
      timerInstance.offset(levelTimeVectorOffset)
  )
  ```

### Managed strings

- [x] Remove the naming inconsistency of the standalone
  `process.readManagedString` function.
- [x] Initially place specialized readers in the same namespace, for example
  `process.readManagedString(address, maxUtf16Units)`, because a managed string
  is a pointer-based runtime object rather than a fixed inline memory layout.
- [ ] Explore representing Unity managed strings as a `Deserializable` wrapper
  once custom deserialization exists. Do not pretend they are ordinary inline
  `String` values merely to force them through the generic reader.

### Traits / interfaces

- [x] Establish one compiler-known structural equality capability shared by
  semantic diagnostics and Wasm helper generation. Records and enums derive it
  recursively from their fields and payloads; this service can later feed LSP
  availability and hover information without reconstructing backend rules.
- [ ] Design a trait or type-class system compatible with bidirectional
  inference. It should support standard-library constraints such as
  `Deserializable` without forcing routine scripts to spell generic bounds.
- [ ] Start with compiler-known traits needed by memory reading, formatting,
  equality, and string interpolation.
- [ ] Decide later whether users can define and implement their own traits.
  Avoid committing to a full Rust-like trait system before real splitter ports
  demonstrate the necessary surface.
- [ ] Put trait/capability declarations and implementations in the same
  semantic catalog used by standard-library signatures. Completion and hover
  need to explain why a method is available for an inferred type, including
  which bound supplied it.

### Anonymous records

- [ ] Add structural anonymous record values and types after named-record
  deserialization is stable.
- [ ] Infer their field types bidirectionally and allow ordinary field access,
  nesting, matching, and GC storage.
- [ ] Decide whether anonymous records can implement `Deserializable` from a
  type annotation or remain value-level conveniences only. Named records should
  remain the recommended form for documented target-process layouts.

## P1 — Core language and lifecycle polish

### Compound assignment

- [x] Support `+=`, `-=`, `*=`, `/=`, `%=` and the applicable bitwise/shift
  assignment operators.
- [x] Reuse the normal operator typing and cast rules. The left side must be an
  assignable location and must be evaluated exactly once.
- [x] Replace verbose code such as
  `runTimeSeconds = runTimeSeconds + old.levelTime` with
  `runTimeSeconds += old.levelTime` in the Lunistice port.

### Timer state as an enum

- [x] Replace the integer returned by `timer.state()` with an exhaustive
  `TimerState` enum.
- [x] Name variants after the actual ASR states and document host integer
  conversion only at the ABI boundary.
- [x] Support structural `==` / `!=` for enums, including active payload
  comparison, and update the Lunistice port to compare named `TimerState`
  variants directly instead of matching solely to emulate equality.

### Small global runtime functions

- [x] Make frequently used, unambiguous operations global:
  `setVariable(key, value)` and `setTickRate(hz)`.
- [x] Keep namespaced APIs where the namespace provides real disambiguation or
  discoverability. Do not flatten the entire standard library as a blanket
  rule.
- [x] Remove the prototype spellings `timer.setVariable` and
  `runtime.setTickRate`; the language has no compatibility burden yet, so one
  canonical spelling is preferable to aliases or migration diagnostics.
- [x] Rename the implementation-shaped `Duration.saturatingSecondsF32` API to
  the scripting-oriented `Duration.fromSeconds`. Keep range handling as a
  documented safety property, not part of the function name, and retain no
  compatibility alias while the language is still unpublished.

### Lifecycle vocabulary

- [x] Rename `update` to `whileAttached` so its execution condition is visible
  in the source.
- [x] Audit all lifecycle names together. The former detached block runs once
  on entry, so `onDetached` is more accurate than `whileDetached`.
- [x] Reserve `whileDetached` for a future block that genuinely runs on every
  detached polling tick, if a real use case needs it.
- [x] Use the coherent set `onAttach`, `whileAttached`, and
  `onDetached`, with the async and cancellation behavior documented beside the
  names.

### General async and cancellation lowering

- [x] After the lowering IR and `Result` semantics are stable, replace the
  current `onAttach` statement-index special case with a dedicated
  state-machine transformation over lowered control flow. Every await has a
  stable poll state and continuation state; nested conditional branches lower
  through the same dispatcher without replaying preceding statements while a
  poll remains pending.
- [x] Compute which locals actually live across each suspension point rather
  than storing every attach local in one continuation frame. Backward liveness
  is recorded on each lowered `Suspend`; the physical GC frame contains the
  deterministic union, while locals killed before use remain ordinary Wasm
  locals.
- [ ] Once real-world frame sizes justify it, coalesce non-overlapping
  suspension-live ranges into shared physical frame slots without changing the
  per-suspension liveness exposed by the lowering IR.
- [x] Model process lifetime as a structured cancellation region so library
  futures can offer the equivalent of ASR's `until_process_closes(...)`
  without every operation hard-coding `onAttach` checks. The lowered body owns
  the region, cancellable `Suspend` terminators reference it, and process exit
  resets readiness plus the complete continuation frame in one runtime action.
- [ ] Let the standard-library catalog describe whether an operation suspends,
  can be cancelled, or requires an attached process. The checker, async
  lowering, docs, and LSP signature/hover output should expose the same facts.

  - [x] Add `RequiresAttachedProcess` and `CancelsOnProcessClose` alongside the
    existing suspension/retry effects, and make async lowering derive its
    cancellation edge from the resolved catalog item.
  - [x] Normalize raw effects into a public `OperationSemantics` query shared
    by the checker and lowering. Validate contradictory catalog declarations,
    render the same facts for hover/documentation consumers, and reject direct
    process operations in `onDetached`.
  - [x] Infer operational requirements through user-function call graphs so a
    helper that reads process state cannot be called transitively from
    `onDetached`. Publish the fixed-point result through `CheckedProgram` for
    ordinary functions, methods, and recursive call graphs without manual
    function annotations.
  - [ ] Surface those effects through the future machine-readable docs and LSP
    hover/signature protocol rather than inventing editor-specific flags.
- [ ] Broaden suspending control flow and the future library incrementally:

  - [x] Allow awaits in nested `if` / `else if` / `else` control flow inside
    `onAttach`, preserving the selected branch and its continuation.
  - [x] Add `await nextTick()` as the first reusable suspension primitive. It
    resumes on the following attached-process update without replaying prior
    statements and is cancelled with its process-lifetime region.
  - [x] Add first-class `retry expression` control flow for arbitrary `T!`
    expressions. It re-evaluates the expression once per update, yields `T` on
    success, and uses the assignment as its suspension/Result boundary. This
    works through ordinary user functions rather than a hard-coded builtin.
  - [x] Lower `while` loops containing `await` or `retry` through explicit async
    header and exit states. Resumed bodies preserve nearest-loop `break` and
    `continue` targets, including fallback forms and nested suspending loops.
  - [ ] Add suspending user functions and reusable race combinators after their
    inference, cancellation, and frame-ownership rules are specified.

## P2 — Later control-flow extensions

Explicit catches are intentionally deferred. Ordinary autosplitters can already
handle failures with `T!`, `else`, postfix `?`, `throw`, function boundaries,
and transactional state-field boundaries; catch syntax does not currently
unblock a representative port.

- [ ] Design explicit `catch` boundaries and their expression syntax. An
  uncaught throw leaves a `T!` function as its error result; state-field
  assignments catch into their poll result. Ensure nested catches compose
  without losing the original error or forcing equal success types.
- [ ] Once explicit catches exist, allow `throw` anywhere their boundary is in
  scope, including actions and expression-oriented state DSL contexts where a
  statement block becomes available.

## P2 — Debug and release profiles

Implement this on typed HIR/lowering IR and catalog effects. Profile erasure
must be a semantic lowering pass, not conditionals scattered through AST walks
and Wasm emission.

- [x] Add explicit debug and release compiler profiles to the CLI and compiler
  library API. `--profile debug|release` is shared by one-shot and watch builds,
  debug is the default, and the selected profile is retained by Wasm lowering
  for the upcoming semantic erasure pass.
- [x] Add the first `debug` modifier for expression statements and calls such
  as `debug print(...)`, assignments, `if`, `while`, and unbound suspension
  statements.
- [x] Extend `debug` to function and method declarations. Debug functions are
  checked normally but omitted from release Wasm IR.
- [x] Add debug-only local bindings, suspended bindings, and globals. Retained
  code cannot use their names, and release lowering removes global storage and
  initialization as well as local statements.
- [ ] Extend `debug` to remaining declarations only when a real use case
  establishes their erasure and dependency rules.
- [x] Remove supported debug-only statements from release WebAssembly before
  reachability, and verify their strings and imports are eliminated too.
- [x] Enforce the release name-resolution rule for functions and bindings:
  ordinary retained code cannot use a debug-only name, while debug statements
  and debug functions can. Apply the same rule when more declaration kinds
  become debug-capable.
- [x] Initially restrict `debug` to statements whose removal has a
  clear type (`unit`). A value-producing debug expression needs either a
  release fallback or another explicit rule; silently inventing a default value
  would be unsafe. Debug-only bindings now have explicit lexical visibility
  and erasure rules; terminating statements remain rejected.
- [x] Type-check debug-only code in release builds so debug paths do not
  silently rot, while removing it before release code generation.
- [x] Add profile-aware compiler and runtime tests and document current
  diagnostic and debug logging behavior.

## P2 — Formatter, LSP, and editor support

### Watch builds

- [x] Add `splitc watch <input.split> [-o <output.wasm>]` with an immediate
  initial build and content-based change detection that survives editor file
  replacement and coarse filesystem timestamps.
- [x] Publish each successful module through a same-directory temporary file
  and rename, so debugger reloaders do not observe partial Wasm. Preserve the
  last successful output when reading or compilation fails.

### Tooling-ready syntax and compiler database

Do this immediately before formatter/LSP implementation. It builds on the
compiler facade and typed HIR, but it does not block the intervening language
and standard-library work.

- [x] Add a lossless source document and token/trivia layer. The compiler uses
  the same lexer pass for parsing and for an ordered lexeme stream that retains
  whitespace, ordinary comments, documentation comments, exact token spelling,
  and byte spans across parsed, lowered, and checked products. Formatting must
  consume this layer rather than pretty-printing the semantic AST.
- [x] Add an editor-facing recovering parse API with a partial AST, multiple
  diagnostics, and explicit missing/error recovery nodes at top-level
  declaration boundaries. Batch parsing uses the same pass but remains strict.
- [x] Recover independently inside function, action, and nested statement
  blocks. Synchronize at semicolons, closing braces, and plausible statements
  on later lines without consuming a valid boundary token.
- [x] Recover invalid record fields and enum variants independently while
  retaining later valid members and their stable IDs.
- [x] Recover invalid state fields independently in both supported state
  syntaxes, retaining other pointer paths and state expressions.
- [x] Recover neighboring settings independently in the simple settings block,
  nested documentation DSL, and older constructor-shaped syntax.
- [x] Recover invalid `choice` options and file filters while retaining their
  containing setting and later valid entries.
- [x] Recover invalid match arms independently while retaining the match
  expression, later arms, and enclosing function or action.
- [x] Recover invalid function parameters independently and retain function
  bodies when the parameter list is missing its closing parenthesis.
- [x] Recover malformed array elements and function-call arguments
  independently, retaining later expressions and their enclosing statement.
- [x] Recover malformed record-literal fields and template interpolations
  independently, retaining neighboring fields, later interpolations, and the
  enclosing expression.
- [x] Add a syntax-only error expression and use it for missing unary/binary
  operands and malformed parenthesized expressions. Preserve the following
  statement without allowing recovery placeholders into typed HIR.
- [x] Recover missing conditions, empty or malformed branches, and a missing
  `else` inside expression-valued `if`, retaining the complete conditional and
  following statements.
- [x] Recover malformed declaration and statement root expressions without
  discarding globals, state fields, locals, assignments, control-flow
  statements, suspensions, throws, or standalone expression statements.
  Missing `match` scrutinees likewise retain the enclosing match expression.
- [ ] When modules or another multi-source feature are actually introduced,
  add `FileId`, a source map, and file-aware spans as part of that feature.
  Single-file scripts keep file-local byte spans for now; line/column
  conversion remains at the presentation boundary.
- [x] Move diagnostics into a dedicated model with stable compiler-stage codes
  and severity. Lexical, syntax, type, and post-type semantic errors use
  `SS0001` through `SS0004`, and the CLI renderer exposes the same values that
  editor tooling can query.
- [x] Add primary and secondary labels, notes, and applicability-classified,
  multi-edit fixes to the shared diagnostic value. CLI rendering consumes this
  model, and the repeated wrapper-postfix diagnostic proves a real
  machine-applicable source edit. Eventual LSP conversion must use these same
  values rather than introducing a parallel diagnostic shape.
- [ ] Enrich individual diagnostics with focused labels, notes, and fixes as
  language features and editor code actions are implemented; do not block the
  compiler database on exhaustively annotating every existing error first.
- [x] Back the compiler facade with reusable single-source queries for syntax,
  lowering, name resolution, inference, references, and diagnostics. Begin
  with explicit caching/invalidation; adopt a framework such as Salsa only if
  measurement shows it is worthwhile.

  - [x] Add a revisioned `CompilerDatabase` that caches recovering/strict
    parsing, declaration lowering, checking, and diagnostics as shared query
    results. Identical source updates are no-ops; changed text invalidates all
    dependent stages without introducing a `FileId`.
  - [x] Expose declaration lookup, inferred expression/value/function-result
    types, semantic type shapes, resolved calls and paths, assignment targets,
    and a cached read/write reference index without forcing clients to know
    which compiler product owns each fact.
- [x] Preserve partial syntax/HIR results after errors and expose symbol/type
  lookup at a source position. Completion, hover, semantic tokens, navigation,
  and code actions must not invoke or depend on the Wasm backend.

  - [x] Lower declarations retained by the recovering parser into cached HIR
    even when strict parsing fails.
  - [x] Query the smallest checked expression at a byte position with its
    inferred `TypeId`, semantic `TypeKind`, and resolved path/call/constructor
    information.
  - [x] Expose exact lossless token lookup at a byte position and align the
    identifier components of typed paths and call targets with their precise
    source spans, excluding arguments and nested child expressions.
  - [x] Resolve identifier segments in checked expressions to exact
    source-definition spans for values, functions, record fields, enums, and
    enum variants. Represent standard-library calls and compiler-provided
    fields as catalog targets rather than inventing source spans.
  - [x] Add a syntax-reference index for source-defined types in annotations,
    method receivers, return types, enum payloads, and casts, plus record
    literal type/field labels and enum-pattern type/variant labels. Declaration
    and pattern-binding identifiers navigate to themselves.
  - [x] Navigate source-spellable built-in types, array/option/result syntax,
    wrapper constructors and patterns, keywords, lifecycle names, setting
    documentation, and choice/file DSL tokens to stable `LanguageCatalog`
    items. Choice option enum/variant labels navigate to their source.
  - [x] Catalog and navigate snapshot roots, standard-library value fields,
    and the `TimerState` type and variants. Their semantic IDs resolve to the
    `StandardLibrary` symbol graph rather than editor-specific special cases.
  - [x] Preserve useful semantic facts when other expressions fail type
    checking so navigation and hover remain available in unaffected regions.
    The recovering checker publishes its diagnostics alongside a partial
    semantic model; database type, resolution, position-analysis, and
    definition queries fall back to that model without constructing typed HIR
    or invoking the Wasm backend.

### Formatter

- [x] Build a canonical formatter first, sharing the compiler lexer/parser.
- [x] Preserve ordinary comments and `///` setting documentation comments.
- [x] Cover settings DSL indentation, match arms, interpolated strings, state
  expressions, and multiline process reads.
- [x] Expose formatting through `splitc fmt` and a cached
  `CompilerDatabase::format` query for the future LSP.

### Language server

- [x] Create the `splitls` LSP server module and stdio binary inside the
  existing crate, backed by one reusable `CompilerDatabase` per open document
  rather than reparsing independently for each feature. Keep it internal until
  the crate-splitting criteria above are met.
- [x] Implement diagnostics and formatting first, including full document
  synchronization, UTF-16 positions, document versions, structured diagnostic
  metadata, and cached whole-document formatting edits.
- [x] Add semantic highlighting, including settings titles, state fields,
  action/lifecycle blocks, types, enum variants, signatures, and debug-only
  code. A cached compiler-owned highlight index combines lossless lexical
  tokens, syntax declarations, and recovered semantic resolutions; the LSP
  only converts its byte spans into delta-encoded UTF-16 semantic tokens.
- [x] Add completion for keywords, action blocks, standard-library symbols,
  settings, state snapshots, record fields, enum variants, and inferred methods.
  The compiler owns candidate kinds, snippets, documentation, and replacement
  spans. An incomplete `receiver.` is probed without its unfinished suffix so
  receiver types and record/user/standard-library members stay inferable. Root
  completion follows lexical scope and includes parameters, preceding ordinary
  and suspension bindings, nested-block locals, and match bindings while
  excluding declarations after the cursor.
- [x] Add standard-library hover and signature help directly from
  `StandardLibrary` catalog queries. Completion already consumes catalog
  signatures and documentation; hover and signature help additionally show
  inferred substitutions, effects/availability, parameter docs, and the
  compiler-validated catalog examples without importing the Wasm backend.
  Signature help counts nested delimiters correctly and probes the inferred
  receiver when a method call is still syntactically incomplete. Source hover
  renders inferred types for globals, locals, parameters, state and setting
  fields, record fields, functions and methods, records, enums, and variants.
  Function and method hover also renders transitive operational effects,
  attachment constraints, synchronous behavior, and debug-only availability.
- [x] Drive keyword, settings DSL, and lifecycle hover documentation from the
  sibling `LanguageCatalog`; completion already consumes the catalog and the
  extension must not duplicate prose or syntax lists.
- [x] Add an integration test that asks the LSP for a catalog symbol such as
  numeric `.clamp`, then verifies that completion and hover expose the same
  signature and documentation as generated standard-library docs. The
  renderer-independent `StandardLibraryDocumentation` payload is now shared by
  completion, hover, and signature help; the test compares their JSON-RPC
  responses against both its generic and inferred `T = i32` forms.
- [x] Add go-to-definition and find-references for functions, types, globals,
  state fields, settings, record fields, and enum variants. Both features use
  stable source declaration IDs and exact identifier-token references from the
  compiler database; the LSP only maps those spans into locations for the
  current single-file URI. References distinguish same-spelling declarations
  and honor the protocol's `includeDeclaration` flag.
- [x] Add identity-safe rename after the core navigation features are stable.
  `prepareRename` selects the exact occurrence under the cursor, and the
  compiler query validates identifier syntax, reserved catalog names, and a
  rebuilt candidate document. It additionally verifies that every existing
  source reference retains the same stable declaration ID, preventing captures
  that could still type-check. Rename currently requires a semantically valid
  document so newly introduced conflicts can be distinguished reliably.
- [x] Add document symbols and code actions. A cached, editor-neutral compiler
  query restores source order and models state/settings as domain containers,
  nested setting titles as outline groups, record/enum children, methods, and
  lifecycle events. LSP quick fixes are derived from the compiler's structured
  diagnostic fixes, honor requested ranges and `context.only`, and preserve
  applicability metadata rather than duplicating repair logic in the server.

### VS Code extension

- [x] Package the LSP client, language configuration, file association, basic
  fallback grammar, formatter integration, and build/debug tasks. The
  TypeScript client uses `vscode-languageclient` 10, discovers configured,
  bundled, repository-development, or `PATH` copies of `splitls`, and restarts
  when server settings change. The repository launch/task configuration builds
  both halves and also exposes compile/watch tasks for the current `.split`
  file. The extension itself exposes two explicit editor-title, context-menu,
  and Command Palette workflows: a status-bar-managed debug watcher that
  rebuilds after saves, and a one-shot release build that cannot race with the
  watcher. Both share compiler discovery, automatic initial saving,
  notifications, and an output channel; the extension package passes TypeScript
  checking and a dry-run pack.
- [x] Use semantic tokens from the LSP as the authoritative highlighting layer;
  keep TextMate highlighting only as a fast startup fallback. The extension
  enables semantic highlighting for SplitScript, contributes every custom
  domain token type and modifier with standard supertypes/theme scopes, and has
  a Rust integration test that prevents its manifest from drifting from the
  server legend.
- [ ] Add snippets for state, settings, lifecycle blocks, match, records, and
  common process/Unity attachment patterns.

## P2 — Documentation and migration

### Generated language and standard-library documentation

- [ ] Build the browsable rustdoc-like renderer as a consumer of
  `StandardLibrary` and `LanguageCatalog`; the structured source of truth and
  compiled-example validation are established in the P0 architecture
  checkpoint, not recreated here. Reuse the renderer-independent
  `StandardLibraryDocumentation` entry model already consumed by editor tools.
- [ ] Generate canonical signatures from semantic type schemes and link types,
  traits, methods, related items, source definitions, and host capabilities.
- [ ] Publish machine-readable catalog data for editor tooling that cannot link
  the compiler library directly, with a schema/compiler version handshake.
- [ ] Test that rendered pages, machine-readable output, and LSP hover identify
  the same catalog item and use the same documentation payload.

### Guides for existing communities

- [ ] Write “Coming from old ASL / C#”, “Coming from TypeScript / JavaScript”,
  and “Coming from Rust” guides.
- [ ] Include syntax maps, lifecycle differences, numeric and address types,
  nullability/results, process reads, async attachment, settings, and complete
  small ports.
- [ ] Explain semantic differences, not just token substitutions—especially
  transactional state, inference, cancellation, and WebAssembly sandboxing.

### Familiarity-oriented diagnostics instead of broad aliases

- [x] Add recovery diagnostics with preferred machine-applicable fixes for the
  first unambiguous foreign spellings:
  - C#/JavaScript declarations: `const` and `var` → `let`; `func` and
    `function` → `fn`;
  - JavaScript absence and C# library names: `null` → `None`, `string` →
    `String`, and `TimeSpan` → `Duration`;
  - C# numeric keywords: `sbyte`/`byte`, `short`/`ushort`, `int`/`uint`,
    `long`/`ulong`, and `float`/`double` → the corresponding
    `i8`/`u8` through `i64`/`u64` and `f32`/`f64` types.
  Recovery must produce canonical syntax immediately so one familiar spelling
  does not cause a cascade of unrelated parser or type errors.
- [x] Add context-sensitive “did you mean” diagnostics for unresolved catalog
  and user-defined function/method calls. Compare names across camelCase,
  PascalCase, and snake_case before applying ordinary edit-distance matching,
  so `Duration.FromSeconds`, `Duration.from_seconds`, and small typos all point
  to `Duration.fromSeconds`. Filter methods by the inferred receiver type,
  replace only the exact name segment, and suppress ambiguous or unrelated
  guesses. LSP code actions expose unique suggestions as machine-applicable.
- [ ] Move the growing foreign-spelling table and its source-language,
  replacement, explanation, and applicability metadata behind a migration
  catalog consumed by diagnostics and LSP code actions. Keep context-sensitive
  recognition in the parser/checker: catalog data must not make ordinary
  bindings such as `let double = ...; double.clamp(...)` look like type names.
- [ ] Add the remaining unambiguous token and delimiter fixes first:
  - JavaScript `===`/`!==` → `==`/`!=` (there is no coercing equality), and
    `${value}` → `{value}` inside SplitScript backtick interpolation;
  - TypeScript `boolean` and CLR `Boolean` → `bool`, plus CLR primitive names
    such as `Int32`, `UInt32`, `Single`, `Double`, and `System.Int32` → their
    canonical SplitScript types;
  - C#/Rust `IntPtr`, `UIntPtr`, `nint`, and `nuint` → `address` only in
    target-process-address contexts where that nominal conversion is correct;
  - Rust `let mut name` → `let name`, because SplitScript `let` bindings are
    already mutable.
- [ ] Add type-aware fixes only after name and type resolution, so they are not
  offered for shadowed user symbols:
  - JavaScript/TypeScript `value ?? fallback` → `value else fallback` when the
    left side is an Option or Result and the fallback has the unwrapped type;
  - `.toString()`/`.ToString()` → `as String`, and `.Length` → `.length`, when
    the receiver and result make the rewrite equivalent;
  - `console.log(value)` and Rust `println!(...)` → `print(...)` only for call
    shapes whose formatting behavior is preserved;
  - C# `Math.Min`/`Math.Max`/`Math.Clamp` and Rust `min`/`max` forms → the
    type-directed `.min(...)`, `.max(...)`, and `.clamp(...)` methods.
- [ ] Provide focused explanatory diagnostics without an automatic replacement
  when the foreign type has no unique equivalent: TypeScript `number` and
  `bigint`, C# `decimal`, and generic error-bearing `Result<T, E>` all require a
  width, representation, or error-model decision from the author.
- [ ] Recognize common structural syntax and show a canonical example, but leave
  multi-token rewriting to `splitc migrate` unless equivalence is proven:
  - Rust `Option<T>`/`Result<T, E>` versus `T?`/`T!`, postfix `.await`,
    `unwrap_or`, `vec![]`, `loop`, and `&str`;
  - C# casts `(T)value`, `new` expressions, `$"..."` interpolation, and
    `switch`; JavaScript ternaries, optional chaining, arrow functions, object
    literals, and `switch`;
  - explicit `async onAttach` should explain that `onAttach` is inherently
    suspending; `async fn` must not be presented as equivalent until reusable
    suspending functions exist.
- [ ] Add old-ASL-specific lifecycle migration diagnostics:
  - `update` → `whileAttached` is a direct block-name migration;
  - `init` should point to `onAttach` and explain suspension and automatic
    process-close cancellation;
  - `exit` should point to `onDetached` while warning that `onDetached` also
    runs once for the initial detached state;
  - `startup`, `shutdown`, and `onStart` need guidance rather than blind renames
    because their lifetime boundaries do not all have one-to-one replacements.
- [ ] Teach `splitc migrate` the old-ASL shapes that require coordinated AST
  rewrites: `state("process")` declarations, `vars`, `settings.Add`/
  `AddDropdown`/`SetToolTip`, refresh-rate assignment, timer APIs, C# memory-read
  helpers, and action/lifecycle blocks. Preserve comments and report every
  construct that needs manual review.
- [ ] Test every migration rule with a positive case, a shadowing/ambiguity
  negative case, the advertised applicability, and—where a fix is marked
  machine-applicable—a test that applying all edits yields compiling canonical
  source. Build the eventual migration-command fixtures from real splitter
  ports rather than synthetic syntax alone.
- [ ] Prefer diagnostics and automated code actions over accepting foreign
  spellings as permanent aliases. Multiple equivalent syntaxes fragment style,
  complicate the formatter and documentation, and make search/completion less
  predictable. Only add a compatibility alias when porting data shows that it
  removes substantial friction without changing semantics; the formatter must
  still emit one canonical spelling.

## Ongoing — Port-driven language development

- [ ] Maintain a representative corpus of real autosplitters covering native
  games, Unity Mono, Unity IL2CPP, Unreal, emulators, pointer-heavy games,
  settings-heavy splitters, load removal, game-time calculation, and process
  restarts.
- [ ] Port additional splitters incrementally and record every missing feature,
  awkward pattern, generated-Wasm issue, and diagnostic failure.
- [ ] Promote repeated game-independent patterns into the standard library;
  keep game-specific signatures, offsets, and policies in scripts.
- [ ] Add a runtime conformance test for every promoted feature, including
  cancellation, failed reads, settings changes, process closure, and memory
  handle cleanup.
- [ ] Use the port corpus as formatter fixtures, LSP integration projects,
  documentation examples, compile-time benchmarks, and release regressions.
- [ ] Do not declare the language generally usable based only on Lunistice. The
  corpus—not speculative feature count—is the readiness criterion.

## Recommended execution order

1. Keep the completed expression-valued `if` and Lunistice union-state behavior
   locked down with compile and runtime regression tests.
2. Establish the compiler facade, semantic `TypeId`/symbol model, constraint
   layer, resolved typed HIR, and shared visitors without changing language
   behavior.
3. Build the standard-library/language/ABI catalogs and their validation API.
   Migrate `min`/`max`/`clamp` first, then all existing built-ins; make a small
   catalog documentation query testable before adding more library APIs.
4. Add the Wasm-oriented lowering IR and backend boundaries, initially without
   inventing a separate failure protocol. Split the code generator internally
   only as those interfaces make the split natural.
5. Specify and implement `T?`, `T!`, and `else` fallback/control flow on those
   foundations, including their typed-HIR and Wasm GC representations.
6. Lower process-read failures through ordinary `T!` values and result-aware
   control flow, then implement generic typed reads, named-record
   deserialization, the minimal
   capability/trait machinery, and Lunistice's single `LevelTimeParts` read.
7. Land compound assignment, `TimerState`, global convenience functions, and
   lifecycle renaming as one language-consistency pass through the catalog.
8. Generalize async lowering and structured process cancellation before adding
   a larger future/combinator library.
9. Add lossless syntax, structured diagnostics, source-aware compiler queries,
   and the formatter; then build LSP and VS Code support. Standard-library
   completion/hover/signature help must already have catalog data to consume.
10. Build the browsable documentation renderer and machine-readable catalog
    export from the same API used by the LSP.
11. Add debug/release profiles as typed-HIR/lowering passes and expand them
    using real debugging workflows.
12. Continue porting diverse autosplitters and measuring compile time/Wasm size
    throughout every step; use the corpus to revise priorities rather than
    waiting until the architecture is “finished.”
