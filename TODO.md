# SplitScript active roadmap

This file contains only active or deliberately deferred work, ordered by
dependency and impact. The detailed implementation history and the
2026-07-30 maintainability audit are preserved in
[`docs/ROADMAP_ARCHIVE.md`](docs/ROADMAP_ARCHIVE.md).

Priority meanings:

- **P0** — correctness or foundational design needed before the next major
  standard-library or language expansion.
- **P1** — important language and API ergonomics once the foundations exist.
- **P2** — tooling, documentation, migration, and ecosystem scaling.
- **Ongoing** — evidence that must accompany changes in every priority.

## P0 — Extend the completed source-defined standard-library boundary

The bundled public surface now has one canonical authoring source:
`stdlib/standard.split`. The dependency-light syntax crate parses it in an
explicit privileged mode, the build-only loader validates and generates typed
catalog data and stable IDs, and the compiler validates the generated graph
against its closed intrinsic/runtime trust boundary. Ordinary compiler and
tooling consumers only see the normalized immutable graph.

- [x] Make `CompilerContext` own an immutable, shareable validated catalog
  graph rather than a copyable handle to a `'static` singleton. `StandardLibrary`
  now owns an `Arc` to its indexed graph, context/stage products are `Clone`
  rather than `Copy`, and algorithms borrow the library while explicit product
    boundaries clone the owner. A separately built validated graph is injected
  through context and tested across parsing, checking, typed HIR, Wasm IR, and
  binary generation.
- [x] Define the ordinary standard-library surface in privileged SplitScript
  source. Its declarations should co-locate namespaces, types, capabilities,
  fields, variants, methods/functions, documentation, and focused examples.
  - [x] Establish the dependency-light privileged source parser and migrate the
    complete bundled surface. Structs, enums, fields,
    variants, static functions, parameter documentation, focused examples,
    representations, value-usage rules, capabilities, and intrinsic binding
    names now have an owned source model.
    - [x] Move spans, tokens, lexical rules, and the privileged declaration
      grammar into `splitscript-syntax`. Ordinary and standard-library parsing
      now select explicit modes of one lexer; the build-time loader owns only
      catalog/ID generation rather than a second parser stack.
    - [x] Move token position, lookahead, exact/variant matching, EOF behavior,
      and documentation-comment collection into one shared `TokenCursor` used
      by both grammars. Parser recovery retains policy, not token mechanics.
    - [x] Move the ordinary program AST, parser cursor, recovery,
      and grammar into `splitscript-syntax`, then expose ordinary and privileged
      parsing as modes of that shared parser. First replace parser-time
      dependencies on compiler semantic/catalog identities with unresolved
      syntax identities; the syntax crate must not depend back on the compiler.
      - [x] Move `SettingDecl::value_type`, `ActionKind::return_type`, and
        resolved type display out of the syntax AST and into semantic adapters.
      - [x] Keep state-provider names and process syntax immutable. Lowering now
        records `StdlibStateProviderId` in `ProgramResolutions`, checking
        publishes it in `SemanticModel`, and backend attachment derives the
        provider process list from that semantic fact.
      - [x] Keep source-written nominal type references immutable. Lowering maps
        `TypeNameId` to source/catalog identities in `ProgramResolutions`, and
        type checking consumes that side table without replacing `TypeRef::Named`
        nodes in annotations, casts, or constructed type syntax.
      - [x] Stop rewriting enum-qualified paths, calls, match patterns, and
        choice settings into resolved AST forms. Their source/catalog enum
        identities are keyed by stable expression, pattern, and setting IDs in
        `ProgramResolutions`; checked variants flow through `SemanticModel` and
        typed HIR. The synthetic `ExprKind::Enum` and
        `EnumReference::Resolved` forms have been deleted.
      - [x] Split source `TypeRef` from resolved semantic/layout type
        references. Primitive spellings now live in `splitscript-syntax`;
        parsed references contain only those spellings, `TypeNameId`, and
        constructed-syntax IDs. `ResolvedTypeRef`, checked constructed layouts,
        and `EnumTypeId` live in the semantic type layer, and `ast.rs` no longer
        imports `crate::stdlib`.
      - [x] Move stable syntax IDs and nodes, structured syntax diagnostics,
        recovery nodes, the lossless source document, and the complete ordinary
        parser into `splitscript-syntax` without maintaining conversion copies.
        The compiler crate now re-exports these products and only adapts lexer
        failures into the common diagnostic model.
      - [x] Move the generic immutable visitor and mutable folder beside the
        AST. Raw stable-ID constructors are crate-private; nominal type-name
        iteration and the shared constructed-type allocator are the explicit
        APIs used by later stages instead of fabricating syntax IDs.
  - [x] Generate the normalized typed catalog and every ordinary stable symbol
    ID directly from that source during the Cargo build. Capabilities,
    providers, constructors, namespaces, types, fields, variants, callables,
    signatures, and documentation now have one canonical authoring source.
  - [x] Represent privileged type expressions structurally rather than as
    strings reparsed by the generator. The loader validates nested constructor
    references and arity, nominal field types, capabilities, representations,
    value-usage rules, selectors, literal rules, providers, attributes, and
    focused callable examples before emission.
  - [x] Remove the compiler-side 1,200-line source mirror validator and the
    runtime reparse of `standard.split`. Source-shape errors are build-time
    loader diagnostics; generated graph integrity and trusted intrinsic
    contracts are checked directly by the compiler. The loader is now a build
    dependency only.
- [x] Add an explicit standard-library compilation mode. It may use reserved
  declaration forms and intrinsic bindings that user programs cannot spell;
  loading untrusted source must never grant host or compiler privileges.
  - [x] Reserve `@representation`, `@valueUsage`, `@capabilities`, and
    `@intrinsic` to the bundled loader. Ordinary parsing rejects the decorator
    syntax, and tests prove that changing an intrinsic name cannot select a
    different trusted implementation.
- [x] Keep a small Rust trust boundary for core primitives, compiler-provided
  representations, intrinsics, runtime helpers, and ABI imports. Validate every
  privileged source binding against that registry before the graph is usable.
  - [x] Move `IntrinsicId` and name resolution into a closed Rust-owned trust
    registry. Source bindings resolve through the registry and are then checked
    against its exact callable shape, signature, effects, availability, lowering,
    dependencies, and scratch requirements.
- [x] Once the source loader covers the bundled library, delete the Rust
  authoring macro and generated declaration duplication in one change. Do not
  maintain two canonical sources or compatibility aliases. The loader now
  emits final typed arrays; the former 1,000-line declaration macro and its
  normalization macro have both been removed.
- [ ] Extend ordinary standard-library function bodies beyond the completed
  synchronous, effect-aware and catalog-nominal generic tiers.
  Intrinsics should be the minimal trusted leaf operations; composition,
  validation, convenience APIs, and other high-level behavior should normally
  be library source rather than compiler dispatch.
  - [x] Extend privileged callable syntax so a declaration has exactly one
    implementation: `@intrinsic(Name) fn ...;` or `fn ... { ... }`. Reject a
    body plus intrinsic binding, and reject a bodyless non-intrinsic function.
  - [x] Reuse the ordinary expression/statement AST, resolver, type checker,
    typed HIR, effect inference, and Wasm-IR lowering for library bodies. Do not
    create a smaller second body language or a parallel evaluator in the
    loader. The privileged parser should only add declaration/binding forms.
  - [x] Give body-defined functions stable catalog item IDs and a distinct
    implementation target such as `Implementation::LibraryBody`. Calls from
    user code and other library bodies should use the same overload,
    capability, documentation, and reachability machinery as intrinsic-backed
    items; only the final call target differs.
  - [x] Infer effects and dependencies transitively through the ordinary body
    call graph. Source bodies carry no authored or generated placeholder
    effects: a standalone compiler pass over the bundled library derives their
    canonical `EffectSet`, availability, attachment requirement, suspension,
    and cancellation metadata once per catalog graph. Every user compilation
    rechecks its injected bodies against that cached result.
  - [x] Publish compiler-derived source-body operation semantics through the
    same catalog queries used by checking, detached-call validation,
    documentation, completion, signature help, and hover. `timer.isRunning`
    and `Module.readRelative32` exercise timer-read and attached-process call
    graphs without adding compiler dispatch.
  - [x] Feed library bodies through ordinary reachability and dead stripping.
    The compiler-owned functions are hidden from public syntax, HIR, and
    semantic iterators even though the backend sees the complete unit.
  - [x] Establish the first bounded generic-body specialization tier as a
    bridge for single-parameter capability methods over eligible catalog
    nominal types. This proved source-defined `Numeric.clamp` and reachable
    concrete emission before ordinary inferred generics existed; the temporary
    eager instance generator has since been removed in favor of the shared
    demand-driven machinery below.
  - [x] Add demand-driven monomorphization for ordinary inferred generic
    functions. It supports source-owned types and constructed nominal GC
    layouts without recreating type spellings, recursive components, debug
    erasure, and deterministic diagnostics for excessively large instance
    graphs. Catalog-owned source bodies now use this same machinery instead of
    maintaining a second bounded specialization tier.
    - [x] Establish one concrete `FunctionInstance` identity at the semantic
      boundary and carry it through resolved calls, typed function bodies,
      Wasm IR, reachability, function-index planning, and emission. Ordinary
      monomorphic functions use an empty type-argument vector; backend data
      structures no longer assume that one `FunctionId` means one Wasm body.
    - [x] Introduce ordinary function type schemes. Infer and generalize
      declaration-local type variables without leaking inference variables
      into semantic products; instantiate a fresh copy at every call and
      retain both inferred type arguments and the exact concrete signature on
      `ResolvedCall`. Capability requirements are published with each semantic
      type parameter for diagnostics, hover, completion, and validation.
      - [x] Add a constraint-preserving signature-instantiation boundary.
        Per-call substitution preserves shared variables and recursively
        rebuilds `[T]`, `T?`, and `T!` layouts while copying capability
        and numeric-literal requirements. Resolved user calls now receive their
        type arguments from this path; monomorphic schemes supply none.
    - [x] Infer mutually recursive functions by call-graph strongly connected
      component. Calls within an SCC remain monomorphic until the component is
      solved; only the completed boundary is generalized. Diagnose polymorphic
      recursion explicitly rather than depending on source order. Recursive
      calls are patched to the component's generalized roots after solving.
      - [x] Build a deterministic source-function dependency graph, collapse it
        into strongly connected components, and check components in callee-first
        order. Free calls use exact declaration names; method calls add
        conservative edges to source methods with the written member name.
    - [x] Represent a checked generic body once, then substitute semantic types,
      constructed GC layouts, locals, conversions, and nested calls for every
      reachable `FunctionInstance`. Syntax-keyed semantic and typed-HIR facts
      remain a single editor-facing template; backend lowering carries the
      owning instance and specializes every fact through it. A backend-private
      cloned semantic/layout arena materializes local `Array`, `Option`, and
      `Result` layouts without corrupting source tooling identities.
    - [x] Make reachability discover instances demand-first from actions, state
      expressions, globals, and already-specialized bodies. Deduplicate by the
      structural function, type-argument, and concrete-signature key. Generic
      expansion has deterministic depth and total-instance limits with focused
      source diagnostics; ordinary monomorphic call depth does not consume
      those limits.
    - [x] Route catalog-owned generic bodies through the same inferred template
      and `FunctionInstance` pipeline. Each library body is parsed and checked
      once; the catalog call boundary supplies its exact concrete signature,
      including constructed and source-owned shapes, and backend reachability
      materializes only used instances. `[T].isEmpty()` exercises two array
      element types without an intrinsic or an eager catalog substitution set.
  - [x] Keep privilege capability-based: a library body gains no direct host,
    Wasm, runtime-helper, or memory access merely because it lives in
    `standard.split`; it can only reach those operations through validated
    intrinsic declarations. Ordinary user source must still be unable to spell
    privileged decorators or inject library declarations.
  - [x] Establish the pipeline with non-generic synchronous bodies and
    migrate every `Duration` constructor out of intrinsic dispatch. Catalog
    GC-record literals let `Duration` construct its own private representation
    in `standard.split` without exposing those fields to user code.
  - [x] Add generic array-like and source-type-compatible bodies after ordinary
    generic functions exist. `[T].isEmpty()` is the first constructed-type
    helper and uses the shared demand-driven specialization pipeline.
  - [ ] Add suspending standard-library bodies through an explicit coroutine
    calling convention; do not treat permission to write `await` as sufficient
    or disguise a compiler implementation as a source body.
    - [ ] Classify source-defined functions as synchronous or suspending from
      their transitive checked call graph. A suspending function must be called
      with `await`, and its attachment/cancellation requirements must remain the
      compiler-derived catalog facts consumed by diagnostics and tooling.
    - [ ] Define a backend poll ABI that distinguishes `Pending` from `Ready`
      and carries the completed value without changing the ordinary synchronous
      Wasm call ABI. Specify cancellation and failure behavior before emission.
    - [ ] Give every reachable suspending `FunctionInstance` a typed GC frame
      containing its program counter, parameters/receiver, locals live across
      suspension, nested callee frame, and result storage. Plan these layouts
      with the existing demand-driven specialization and GC-type reachability.
    - [ ] Let the caller retain and poll the child frame across ticks, then move
      the ready result into the source binding and release the child frame.
      Nested suspending source calls must compose; recursion needs an explicit
      diagnostic until frame allocation and recursion limits are designed.
    - [ ] Generalize the current `onAttach`-specific async emitter around a
      shared state-machine body emitter. Keep the process-lifetime region as the
      root cancellation owner and propagate it through awaited library calls.
    - [ ] Add type-checker, Wasm-validation, runtime, cancellation, generic-
      specialization, and editor-metadata tests with one real source-defined
      discovery helper. Continue migrating high-level helpers afterward; retain
      only representation, host-ABI, runtime, or lowering leaves as intrinsics.

### Catalog completeness

- [ ] Author focused, compiler-checked examples for namespaces, types, fields,
  variants, capabilities, and type constructors, then require examples for
  every public documented symbol. Do not pad symbols with shared test-like
  programs; the rendered snippet must teach that symbol specifically.
- [ ] Add a trusted custom-capability handler registry only when the first
  capability cannot be described by declared membership, structural equality,
  or structural memory layout. Bind and validate it like an intrinsic rather
  than adding another capability-ID switch.

## P0 — Preserve architectural boundaries under growth

The audit removed the largest accidental couplings: parsing is syntactic;
resolution and validation are named stages; checking has explicit passes;
typed HIR and complete backend IR are distinct; codegen consumes one
`BackendProgram`; the database publishes a shared semantic snapshot; the LSP
uses typed protocol boundaries; and one command verifies compiler, extension,
Wasm, and runtime behavior.

The remaining evidence-backed risks are scale visibility and API containment.

- [x] Add repeatable warm compiler-database query and end-to-end LSP latency
  baselines alongside the existing one-shot compile/Wasm-size baseline.
  `tooling_baseline` generates a 500-function source and measures cold checking,
  cached checking, hover, highlights, definitions, and full JSON-RPC hover.
- [ ] Add a generated large-catalog dimension after alternate catalog
  construction exists, so graph validation/indexing and catalog-backed queries
  are measured rather than repeated against the fixed bundled graph.
- [x] Classify the crate API into a small compiler facade, a tooling facade,
  and explicitly inspectable stage products. `src/lib.rs` now exposes only
  `compiler` and `tooling` modules plus the convenient root pipeline functions;
  parser/checker/backend passes, registries, and adapters are private. The
  integration suite imports the facades, so tests no longer force every
  implementation module into the package API. Do not split into crates until
  two real consumers need independently versioned APIs.
- [ ] Review modules above the soft 1,000-line signal when a related feature
  changes them: `codegen/expression.rs`, `wasm_ir.rs`, `hir.rs`, `language.rs`,
  `formatter.rs`, `codegen/async_state.rs`, and `completion.rs`. Split only at
  a named product/context or visitor boundary; line count alone is not a
  reason to scatter one mutable responsibility.
- [ ] Keep `cargo xtask check` as the only local and CI verification matrix.
  Extend it whenever a product surface is added rather than duplicating steps
  in workflow YAML. Generated `.wasm`/`.wat` files belong under ignored
  `target` directories and must never be committed.

## P0 — Add typed state providers after the Minish Cap port

The GBA port now uses the completed first state-provider slice. `state GBA`
owns emulator selection and discovery and exposes a read-only `gba` value of
type `GbaEmulator`; scripts never retain or attach the emulator themselves.

- [x] Add provider declarations and generic `state <provider>` resolution to
  the standard-library graph. `state GBA` now obtains its emulator process
  names, process type, attachment intrinsic, and documentation from one
  catalog declaration rather than parser or backend name switches.

- [x] Add declarative state providers to the standard-library graph so
  attachment families are library-owned declarations rather than parser or
  backend switches. Provider metadata must include its source name, typed
  value name and type, process list, attachment implementation, effects, and
  documentation.
- [x] Support `state GBA { ... }`. The GBA provider owns the complete emulator
  process list and discovery logic; individual autosplitters must not repeat
  executable or core names. Its catalog declaration also selects the typed
  direct-read operation used by concise fields such as `room: u8 at
  0x03000010`; computed addresses can still use `gba.read(...)`.
- [x] Give each selected provider one implicit typed value with a catalog-owned
  name. Under `state GBA`, `gba.read` accepts original GBA hardware addresses
  and performs emulator translation; native states expose a typed, read-only
  `process: Process` provider value. Completion hides the unavailable root, and diagnostics
  explain the correct one. Preserve generic typed reads and the same
  Result/failure-boundary behavior as native process reads.
- [x] Generalize ordinary native process attachment into the same provider
  model without reserving a parser keyword per platform. The catalog declares
  a source-process provider whose executable names come from `state "..."`,
  whose identity attachment exposes the raw handle as nominal `Process`, and
  whose direct-read operation drives ordinary `at` fields. `Process` owns its
  module/read/follow/scan/string-decoder methods, including inferred and
  explicitly selected generic reads; values can flow through locals,
  parameters, returns, and globals. GBA remains a transformed provider with a
  catalog-owned executable list and attachment intrinsic. Type checking,
  codegen, completion, hover, definitions, effects, and documentation consume
  these provider/callable facts rather than a `process` namespace exception.
- [ ] Use the same provider declaration shape for future emulator and engine
  families. Add another provider only when a real port supplies its process
  discovery, address domain, and direct-read semantics; do not add parser
  keywords or backend switches for each platform.
- [x] Remove the temporary explicit GBA attachment API. The Minish Cap port now
  uses only `state GBA` and `gba`; compiler, runtime, formatter, hover,
  completion, definition, highlighting, and documentation paths consume the
  provider declaration. No compatibility alias exists.

## P0 — Complete typed process deserialization

- [ ] Complete bounded native string reads without introducing ASL-style
  `string4`/`string32`/`string150` pseudo-types.
  - [x] Add `process.readUtf8(address, maxBytes) -> String!` and concise
    pointer-state sugar `name at ... as utf8(maxBytes)`. The decoder applies
    after the complete pointer path, performs one bounded host read, stops at
    NUL, validates strict UTF-8, and participates in transactional state
    failure. Infer the state field as `String`, cap reads at 4096 bytes, and
    document/highlight/hover/format the contextual syntax.
  - [ ] Add explicit ASCII and UTF-16 decoders when ports require them. Decide
    malformed-input policy per encoding rather than silently sharing the
    managed-string replacement behavior.
  - [ ] Add an explicit fixed-length mode for data where embedded NUL bytes are
    meaningful. Keep null-terminated and fixed-length semantics visibly
    distinct at the call site and cover missing terminators, invalid input,
    failed reads, and empty values in runtime fixtures.
  - [ ] Generalize decoder declarations into catalog/source-owned metadata if
    the family grows, so parser, checker, editor tooling, and codegen do not
    accumulate one handwritten branch per encoding.
- [x] Add exact-length `[T; N]` arrays for transactional indexed process state.
  `[T]` remains the general array type, while `[T; N]` carries its length,
  widens to `[T]`, checks literal element counts, and derives `MemoryReadable`
  recursively. Typed process/state reads fetch one contiguous byte range before
  constructing the GC array. Reject zero-length process reads, unsized `[T]`
  reads, non-readable elements, more than 4096 elements, and layouts over 65536
  bytes. The Minish Cap inventory now exercises one `[u8; 6]` GBA read in both
  mGBA and VBA runtime fixtures.
- [ ] Add declarative record-layout controls when a real target requires them:
  exact offsets, padding/alignment, packing, and per-field endianness. Preserve
  field-order native-endian layout as the ergonomic default and diagnose
  overlap, out-of-bounds fields, and unsupported combinations.
- [ ] Explore representing Unity managed strings as a readable wrapper or
  derived layout instead of a permanent special method, while preserving the
  pointer chasing, length validation, and UTF-16 conversion it requires.
- [ ] Design the user-facing trait/type-class model around capabilities already
  represented in the catalog. Begin with memory reading, formatting/
  interpolation, equality, and numeric operations; decide separately whether
  user code can declare or implement traits.
- [x] Make nominal standard-library `Display` implementations source-driven.
  A type declaring `Display` can tag one parameterless `String`-returning method
  with `@display`; generated catalog metadata owns the implementation identity,
  validates the contract, retains the ordinary hidden function through
  reachability, and dispatches `as String`, interpolation, `print`, and
  `setVariable` through the same body. `FileVersion.toString()` now renders the
  dotted four-part version without any type-specific formatting branch in
  codegen. Primitive formatting remains the built-in fallback; user-defined
  trait implementations remain part of the broader trait design.
- [x] Add source-defined standard-library capability inheritance. Declarations
  such as `Numeric<T: Equatable>` and `Integer<T: Numeric + Display>` now drive transitive
  satisfaction, method availability, inference, and minimal rendered bounds;
  catalog validation rejects unknown, duplicate, and cyclic relationships.
- [ ] Keep trait declarations, implementations, docs, and method lookup in the
  source-defined standard-library model rather than a parallel checker table.
- [ ] Add structural anonymous record values/types after named-record layout
  semantics settle. Infer their fields bidirectionally and decide explicitly
  whether anonymous records can be memory-readable.
- [x] Make `current`, `old`, `settings`, and `oldSettings` first-class read-only
  values rather than path-only compiler roots. Give state snapshots and settings
  views proper structural types so values such as `let snapshot = current`
  can flow through inference, parameters, and returns without losing field
  identity. Tooling already presents the roots as read-only variables and
  navigates them to their declaring block.
  - [x] Promote `current` and `old` to ordinary `StateSnapshot` references over
    the existing transactional state GC object. Locals, inferred function
    parameters, generic identity calls, and returns retain the snapshot without
    copying it; state-field resolution, documentation, completion, semantic
    highlighting, and navigation keep the original field identity. A lexical
    binding named `current` or `old` still shadows the lifecycle root.
  - [x] Promote `settings` and `oldSettings` to ordinary inferred
    `SettingsView` values. The backend represents a view as a one-word selector
    over the already-existing current/old setting globals, so binding, passing,
    and returning one performs no GC allocation or value copying; field access
    selects the corresponding storage, and direct `settings.field` reads remain
    allocation-free. The name deliberately says “view”, not “snapshot”: it is
    an immutable capability for observing one side of the runtime's current/old
    settings pair rather than a permanently retained copy.

## P0 — Support versioned and discovered native state

The 2026-07-31 manual ASL-port review shows that scalar, single-layout scripts
port cleanly. The most common fidelity blocker is not expression syntax but the
assumption that every process candidate shares one fixed state layout. ASL's
second `state("game", "version")` string is a version label, not another
executable name.

- [ ] Design typed state-layout variants from the legacy-ASL corpus rather than
  treating module size as part of the layout grammar. A 2026-08-01 survey of
  1,613 ASL files found 443 files with version-labelled states. Of 432 files
  parsed structurally, 342 keep the same order-independent field interface in
  every layout, 89 differ, 84 combine more than one executable, and individual
  scripts contain as many as 46 layouts. Module size is common (276 files), but
  file/product versions, executable hashes, signatures, process identity, and
  custom memory discovery are also real selectors.
  - [x] Preserve expression-backed state fields and expression `if` as the
    preferred escape hatch for small, irregular, or already-discovered layouts.
    Do not force every conditional read through a new declaration form. The
    corpus-derived `examples/abzu.split` port demonstrates the intended small
    case: `process.mainModule()` selects a typed `AbzuBuild` and one inferred
    address global, while a single ordinary state expression reads it. Both
    supported builds and the never-ready unsupported-build path have runtime
    coverage; no layout DSL improves this shape.
  - [x] Separate named memory layouts from layout detection. `layout Name { ... }`
    declarations co-locate concise `at` fields, documentation, and stable enum
    variant identities. The existing implicitly-suspending `onAttach` block is
    the selector: it uses ordinary typed SplitScript expressions, `await`/`retry`,
    matches, process metadata, module sizes, signatures, and reads, then returns
    exactly one generated layout variant. This keeps one attachment lifecycle
    phase instead of adding a separate `selectLayout` action and does not encode
    `when module(...).size == ...` as a privileged probe grammar.
  - [x] Generate a typed `StateLayout` enum and read-only `layout` value rather
    than exposing ASL's mutable string `version`. Lifecycle code can compare and
    exhaustively match it. Named layouts require `onAttach` to return a variant
    on every completing path; `await process.closed()` is recognized as the
    explicit non-completing unsupported-build path, so there is no silent first-
    layout fallback. `layout` is rejected inside `onAttach` before it exists.
  - [x] Implement the first vertical slice for layouts with an identical field
    name/type interface. Every layout is validated order-independently against
    one canonical snapshot shape, and the selected variant dispatches each
    transactional field read to the correct pointer source after `onAttach`
    completes. `current.field` and `old.field` remain unchanged. Parser,
    formatter, completion, hover, semantic highlighting, Wasm validation, and a
    Steam/GOG/unsupported runtime fixture cover the complete feature.
    The corpus-derived Martha Is Dead port additionally validates three real
    legacy-ASL layouts, module-size selection, module-relative pointer paths,
    load removal, and the unsupported-build lifecycle end to end.
    The Borderlands port validates typed `FileVersion` selection and a shared
    interface whose old build uses direct module offsets while its patched
    build uses expression-backed PE32 pointer paths.
  - [ ] Design non-uniform layouts separately. The survey found 61 differing
    interfaces with stable field types and 28 with at least one same-named type
    conflict. Evaluate common-field projection plus typed variant payloads;
    do not automatically make every missing field optional or erase conflicting
    types merely to mimic ASL's dynamic state object.
  - [ ] Add layout sharing/overrides only after real ports demonstrate the
    minimum useful rule. Large scripts often repeat nearly identical pointer
    paths across dozens of versions, but unrestricted inheritance would make
    the selected memory layout difficult to audit.
  - [x] Allow source-defined standard-library structs to contain constructed
    field types such as `[u64]`, `T?`, `T!`, and other catalog type
    applications. Fields now use the same recursive catalog `TypeRef` as
    signatures, receive stable per-program semantic types, participate in
    recursive Wasm-GC reachability/layout and structural capability validation,
    and render through the shared documentation/tooling path. Privileged
    standard-library bodies may construct any standard record while user code
    still cannot. `MemoryPath` and its width-aware `resolve()` implementation
    now live entirely in `stdlib/standard.split`; the Alan Wake port exercises
    Steam, GOG, Epic, and unsupported PE32 builds at runtime without a
    compiler-owned path type.
  - [x] Extend privileged catalog type syntax with exact array lengths so a
    standard-library field or signature can spell `[T; N]`. Ordinary source
    already had fixed arrays; privileged parsing now preserves the length in a
    dedicated catalog `TypeRef`, validates and generates it recursively, interns
    the same exact semantic array layout, renders it in documentation/tooling,
    and carries it through fixed process-memory and Wasm-GC planning. A
    source-defined catalog method and record field exercise `[u64; 2]` without
    erasing it to `[u64]`.
  - [x] Generalize calls so postfix methods work on every expression receiver,
    not only identifier paths. Alan Wake naturally needs
    `selectedLayout().level.resolve()`. Calls now distinguish resolved path
    receivers from arbitrary receiver expressions throughout syntax, semantic
    resolution, typed HIR, Wasm IR, tooling, and codegen. Postfix receivers are
    visited before arguments and emitted exactly once. Generic catalog calls use
    ordinary `<...>` type arguments without a turbofish or dotted pseudo-path.
    The Alan Wake port calls `MemoryPath.resolve()` directly
    and no longer needs its temporary `resolvePath(...)` wrapper.
- [ ] Complete the read-only process/module identity surface needed for safe
  version probes. `Module.address` and `Module.size` already cover common size
  checks.
  - [x] Expose the exact configured process candidate through which attachment
    succeeded as `process.name()`. Keep it distinct from a filesystem path.
  - [x] Expose `await process.mainModule()` using that matched candidate, so
    common `modules.First().ModuleMemorySize` selectors do not repeat executable
    strings or need a special module-size grammar.
  - [ ] Add module enumeration/search, product-version identity, and a
    deterministic executable fingerprint as corpus ports require them. Prefer
    host-provided metadata/fingerprinting over unrestricted filesystem access
    or reading and hashing an entire module from Wasm.
    - [x] Preserve the queried module name privately in each `Module` value and
      expose fallible `Module.path()`. It uses ASR's host-provided portable path
      ABI, bounds the returned UTF-8 payload, and does not grant general file
      access. Main-module values retain the actual matched process candidate;
      named-module values retain their requested module identity.
    - [x] Add source-defined `Module.fileVersion() -> FileVersion!` by porting
      ASR's bounded PE-resource traversal into `standard.split`. The typed
      `FileVersion` record exposes `major`, `minor`, `build`, and `privatePart`
      components and is equatable, avoiding legacy `FileVersionInfo` strings
      whose punctuation differs between games and APIs. Only typed process
      reads remain intrinsic; DOS/PE validation, resource-directory loops,
      `Result` propagation, and record construction are ordinary library code.
      The checked `v"major.minor.build.private"` literal produces the same
      typed value, validates exactly four decimal `u16` components at parse
      time, and keeps version comparisons concise without exposing record
      construction. The compiler lowers it through source-defined
      `FileVersion.fromParts` rather than a second backend implementation.
- [ ] Make the existing attach-time-discovered state-source pattern a documented
  first-class contract. Scripts can already scan/follow/resolve into globals in
  `onAttach` and use expression-backed state fields; add a canonical recipe,
  clearer typing/tooling, and specified behavior for temporarily unreadable and
  optional fields so authors do not recreate `MemoryWatcher` manually. Keep
  process-close cancellation and transactional snapshot rotation automatic.
  The Borderlands port is a focused syntax-pressure case: its PE32 layout must
  currently spell source-defined `MemoryPath` expressions because declarative
  pointer paths have one implicit pointer width. Use it to decide whether
  pointer width belongs on an individual path, a named layout, retained module
  metadata, or a more general discovered state-source value; do not add an
  `at32` pseudo-keyword.
- [ ] Extend the existing signature APIs only where the corpus proves a gap:
  reusable scan targets, multiple/fallback signatures, range and memory-page
  selection, capture/offset transforms, relative-address decoding, and concise
  pointer-follow composition. Basic `sig` literals, module/process scanning,
  `process.follow`, and `process.readRelative32` already exist and should be
  documented rather than reimplemented.
- [ ] Add provider-backed engine discovery beyond current Unity IL2CPP support.
  Prioritize Unity Mono object/class/field/string discovery, then assess an
  Unreal provider for `GWorld`/object/name traversal from representative ports.
  External `asl-help` wrappers should map to maintained typed providers rather
  than reflection-shaped compiler exceptions.
- [ ] Keep emulator families on the same provider model. Use real corpus ports
  to prioritize Dolphin/PCSX2/RetroArch/DOSBox-style address translation after
  GBA; do not add emulator-name conditionals to the parser or type checker.

## P1 — Make actionable warnings a first-class compiler product

- [x] Diagnose intentionally discardable values separately from values whose
  result carries an obligation.
  - [x] Let privileged catalog declarations mark a callable or constructed
    value type with `@mustUse("reason")`. Discarding such a value as a bare
    expression statement should produce a non-fatal warning whose note explains
    the obligation; binding, consuming, returning, or propagating it must not
    warn. Mark `Option<T>` and `Result<T>` globally, and give
    immutable transformations such as `String.replaceAll` a focused reason.
  - [x] Preserve successful compilation while publishing warnings through the
    compiler facade, incremental database, CLI, compile service, LSP, VS Code
    builds, and debug watch. Keep severity and source fixes structured rather
    than rendering warnings into strings at an inner layer.
- [x] Add an unused-code analysis over resolved semantic identities rather than
  spelling heuristics.
  - [x] Start with unread local bindings and parameters, including bindings in
    `for`, `match`, and failure-control constructs. An underscore-prefixed name
    should express intentional non-use; offer a machine-applicable rename when
    appropriate, and do not confuse a write with a read.
  - [x] Diagnose unreachable private globals, functions, records, and enums.
    Seed the transitive graph from lifecycle blocks and state/settings ABI
    declarations, include debug statements independent of the selected build
    profile, follow resolved calls and global reads by identity, and propagate
    reachable function signatures plus nested nominal layouts. References from
    one dead declaration do not keep another declaration alive; hidden
    standard-library source bodies remain catalog-owned.
  - [x] Extend the same graph to record fields and enum variants. Direct member
    access observes record fields; construction and memory deserialization do
    not. Construction and reachable patterns observe enum variants. Choice
    settings root their exposed variants, and structural equality observes the
    complete recursively nested record/enum shape. Members of a wholly dead
    type do not produce cascading diagnostics. Keep future compiler-invoked
    capability implementations and host callbacks as explicit roots rather
    than relying on apparent source calls.
  - [x] Validate the member policy against maintained ports before enabling it:
    exact member-name spans, underscore suppression, focused notes, equality
    regression coverage, and a warning-free example corpus cover the initial
    policy. Source-defined standard-library declarations retain their separate
    catalog/root policy rather than blanket source-level suppression.
  - [x] Publish stable warning-specific diagnostic codes independently from
    compiler-stage errors: `SS1001` for must-use results, `SS1002` for unused
    bindings, `SS1003` for unused declarations, and `SS1004` for unused
    members. Preserve focused labels/notes and the `_` suppression convention;
    the LSP publishes the same codes and machine-applicable local-binding
    rename actions as the compiler service and CLI.
  - [x] Reuse the identity-aware rename planner to offer safe multi-edit
    suppression actions for globals, functions, nominal types, fields, and
    variants, including references that live only in otherwise dead code. The
    planner tries additional underscores on collision and rechecks both the
    edited program and every stable reference identity before the LSP offers a
    preferred quick fix.
  - [x] Add a transport-neutral allow/warn/deny policy keyed by stable warning
    code. Apply it at compiler-product boundaries rather than inside parsing or
    type checking: `allow` filters a warning, `warn` preserves it, and `deny`
    rejects a configured build while retaining the original `SS100x` code and
    adding an explicit policy note. The staged semantic product remains valid;
    the incremental database can change policy without invalidating semantic
    caches, embedded compile requests carry the policy, and native compile/watch
    commands accept repeated `--allow`, `--warn`, and `--deny` selectors for one
    code or `warnings`. Defer choosing a persistent project-file syntax until a
    project/module model supplies a natural owner for it.

## P1 — Add port-driven collections, text, math, and time

- [x] Use bracket syntax as the sole source spelling for arrays: `[T]` when the
  length is not part of the type and `[T; N]` when it is exact. Both compose
  directly with wrapper postfixes and nesting; the unreleased `Array<T>`
  prototype syntax has no compatibility alias. The privileged
  `typeConstructor Array<T>` declaration remains an internal standard-library
  schema construct rather than public source syntax.
- [x] Add `for value in array` iteration over `[T]` and `[T; N]`, with a
  read-only inferred element binding, lexical scope, `break`/`continue`, and
  exactly-once iterable evaluation. Ordinary loops lower to structured Wasm;
  suspending `onAttach` bodies preserve iterable/index/binding state across
  `await` and `retry`. Formatter, hover, semantic highlighting, inlay hints,
  completion, navigation, checked catalog docs, Wasm validation, and runtime
  fixtures cover the feature. Generalize this protocol when the first
  non-array iterable collection is added.
- [ ] Add growable typed collections after the source-defined standard-library
  boundary is ready: `List<T>`/`Vec<T>`, `Map<K, V>`, and `Set<T>`, including
  indexing or lookup, insertion/removal, containment, clearing, and iteration.
  Derive key constraints from declared equality/hash capabilities. Named
  records should be the normal replacement for C# tuples; add tuple syntax only
  if ports show that records remain materially noisy.
- [ ] Fill out immutable `String` operations used by real split logic:
  - [x] Add allocation-free, exact, case-sensitive `contains`, `startsWith`,
    and `endsWith` predicates over valid UTF-8 strings. Empty needles match;
    runtime coverage includes multibyte text and longer-needle failures.
  - [x] Add fallible `slice(start, end)` with inclusive/exclusive UTF-8 byte
    offsets. Invalid, reversed, out-of-range, and non-code-point-boundary offsets
    return `String!`; the allocation is bounded by the source byte length.
  - [x] Add allocation-free `equalsIgnoreAsciiCase` with deliberately narrow
    ASCII folding; non-ASCII UTF-8 bytes compare exactly rather than implying
    incomplete Unicode case folding.
  - [x] Add fallible, exact, non-overlapping `replaceAll`. Reject an empty search
    string and an unrepresentable result length, precompute the exact output
    size, and allocate only once.
  - [x] Add focused catalog and guide documentation distinguishing bounded
    native-memory UTF-8 reads, Unity managed UTF-16 object reads, and the
    ordinary owned GC `String` value both produce.
- [x] Add catalog-owned floating-point helpers: intrinsic `abs`, `floor`, and
  `ceil`, plus source-defined `round`, `roundTo`, `isNaN`, and `isFinite` on the
  generic `Float` capability. `round` maps directly to Wasm `nearest`, and both
  it and `roundTo` use unbiased ties-to-even semantics. Exact integer-looking
  literals now satisfy contextual `f32`/`f64`
  expectations without a cosmetic `.0`; values that would lose integer
  precision produce a type diagnostic. Catalog validation, compile-time
  inference tests, Wasm validation, and a runtime fixture cover both widths.
- [ ] Complete `Duration` arithmetic and constructors, then add a monotonic
  `Instant`/elapsed-time API for debounce and delayed-split logic. Wall-clock
  calendar time should not be used where a monotonic clock is intended.
  - [x] Define `Duration.zero()` and signed integer
    `Duration.fromWholeMilliseconds(i64)` in standard-library source, alongside
    the existing source-defined frames, parts, and floating-seconds
    constructors.
  - [x] Add exact signed integer seconds as source-defined
    `Duration.fromWholeSeconds(i64)`, keeping the floating-point
    `fromSeconds<T: Float>` conversion visibly distinct.
  - [x] Generalize `Duration.fromSeconds` over the source-defined `Float`
    capability so both f32 and the C#/ASL-typical f64 work without overloads.
    Split milliseconds into generic `fromMilliseconds<T: Float>` and exact
    `fromWholeMilliseconds(i64)` constructors, all implemented in library
    source.
  - [x] Normalize every constructor to one signed `(seconds, nanoseconds)`
    representation, permit Duration values in ordinary source storage, and add
    source-defined component/total accessors plus `add` and `subtract`. Keep
    private representation fields visible only to source bodies owned by
    `Duration`; user completion and field lookup must not expose them.
  - [x] Make binary addition and subtraction catalog-declared language roles,
    analogous to source-driven `Display`: ordinary privileged methods use
    `@operator(add)` / `@operator(subtract)`, generated metadata indexes the
    binding, type checking resolves syntax to a normal catalog call, and
    lowering reuses its intrinsic or source body. Bind both the generic
    `Numeric` capability and source-defined `Duration.add` / `subtract`; prefer
    exact receiver implementations over capability defaults. Validate one
    binding per owner/operator and require receiver, operand, and result types
    to agree. Runtime coverage exercises normalized positive and negative
    Duration results without any Duration-specific checker or backend branch.
  - [x] Resolve `+=` and `-=` through those same catalog bindings. Typed
    assignments retain their resolved call, Wasm IR owns either a primitive or
    call-backed store operation, effects and reachability observe statement
    call targets, and source-defined implementations are retained even when a
    compound assignment is their only use. Duration compound assignment and
    numeric local/global storage share this path.
  - [x] Add exact signed `Duration.fromNanoseconds(i64)` rather than leaking
    C#'s `TimeSpan` tick unit into the permanent API. Straightforward ports may
    convert a C# tick count to its actual unit of 100 nanoseconds; negative
    fractional values normalize without passing through floating point.
  - [x] Add a monotonic `Instant` value for delayed splits and debouncing.
    `Instant.now()` is the single trusted WASI monotonic-clock boundary;
    saturating `durationSince` and `elapsed` are ordinary standard-library
    source bodies returning `Duration`. Clock reads carry an explicit
    `reads runtime state` effect, and runtime coverage fixes the clock readings
    to prove elapsed and backwards-time behavior deterministically.
    Source-defined `hasElapsed(Duration)` covers the common polling operation,
    including an `Instant?` persisted across ticks. Defer `Instant + Duration`
    until its overflow contract is chosen explicitly rather than inheriting an
    accidental Wasm wraparound.
  - [x] Extend catalog-declared operators to `<`, `<=`, `>`, and `>=`, with
    operator-specific signature validation. `Duration` implements all four in
    standard-library source using its normalized integer components, so delay
    logic does not need a lossy floating-point conversion; numeric comparisons
    retain their primitive fallback.
  - [ ] Extend catalog-declared operators to multiplication, division, and
    remainder when concrete non-numeric types require them.

## P1 — Model polling, settings, and timer integration explicitly

- [ ] Design a state-normalization/filtering layer for ASL patterns that mutate
  `current` or copy from `old` to suppress invalid transitions. Snapshots must
  remain immutable; express retain-last-valid, debounce, edge filtering, and
  derived fields declaratively or with persistent typed state.
- [ ] Decide and document the equivalent of a boolean-returning ASL `update`
  block. If it gates snapshot commit or lifecycle evaluation, represent that as
  an explicit polling/tick guard rather than giving the non-returning
  `whileAttached` block an ambiguous return value.
- [ ] Extend static settings for data-heavy ports: stable external keys that do
  not need to be SplitScript identifiers (including numeric ASL keys), typed
  keyed lookup when an index is genuinely dynamic, conditional visibility or
  enablement, and a maintainable way to declare repeated tables. Preserve the
  current label-first DSL, nested headings, choices, files, and `///`-only
  tooltips; do not restore legacy `settings.Add` or compact aliases.
- [ ] Design a typed, least-privilege timer/run API for the recurring host data
  that split logic actually needs: timing method, category and attempt
  metadata, current segment/split history, and run offset. Separate read-only
  metadata from mutations such as changing an offset, pausing, or selecting a
  timing method, and add ABI support only for operations LiveSplit can expose
  safely and consistently.
- [ ] Document and test the existing lifecycle mappings before adding APIs:
  `isLoading` controls game-time pausing, `onDetached` replaces process-exit
  cleanup, `timer.state()` replaces phase integers, and `setTickRate` replaces
  ASL `refreshRate`. Add new primitives only where these mappings cannot
  preserve behavior.
- [ ] Add structured async discovery combinators only as ports require them:
  timeout, race/select, bounded concurrent scans, and cancellation scopes.
  Preserve automatic process-close cancellation and avoid exposing threads or
  unconstrained background tasks as a scripting-language primitive.
- [ ] Write an explicit sandbox capability policy for process writes/code
  injection, arbitrary file reads, network/process launching, modal UI, custom
  audio, and host control. These appear in legacy ASL files but are not implied
  language requirements. Prefer safe host abstractions or documented non-goals;
  any dangerous opt-in capability needs visible consent and cleanup semantics.

## P1 — Async, failure, and control-flow extensions

- [ ] Coalesce non-overlapping async-frame slots only if real autosplitters
  make frame size material. Preserve liveness evidence and cancellation-safe
  cleanup; do not add allocator complexity for hypothetical savings.
- [ ] Let source-defined standard-library declarations express suspension,
  retry, cancellation, and attachment requirements through privileged metadata
  validated against intrinsic contracts.
- [ ] Broaden suspending control flow incrementally from real ports (nested
  loops, matches, and future combinators), adding a runtime conformance test
  for each new shape.
- [x] Add `await process.closed()` as the explicit “ignore this attachment for
  the rest of its lifetime” operation. It is available only in `onAttach`, never
  asks the host to detach, never completes normally, and relies on the existing
  process-close cancellation boundary to discard its async frame before
  `onDetached` and a later attachment. The Abzu fixture uses it for unsupported
  builds and verifies that state polling never begins. Do not expose a general
  detach operation until a separate use case defines how it could avoid an
  immediate automatic reattachment cycle.
- [ ] Design explicit `catch` boundaries later. `throw` should propagate to the
  nearest catch or return a `Result` from the function when uncaught; postfix
  `?` remains ergonomic propagation built on the same semantics.
- [ ] Define the payload-free result spelling and representation before exposing
  fallible procedures. `void!` currently parses and type-checks far enough to
  reach a backend layout assertion because `Result` assumes a value slot. Either
  support it as a proper unit result throughout inference, GC layout, matching,
  and propagation or reject it with a source diagnostic; user input must never
  reach the backend assertion.
- [ ] Extend `debug` to additional declaration kinds only when a concrete use
  case defines reachability, type-checking, and release-erasure behavior.

## P1 — Source-level debugging for debug builds

Debug compilation should produce a module that can be stepped in the original
`.split` source with readable stacks and variables. Release compilation must
remain stripped. This is separate from the `debug` source modifier: the
modifier controls which program constructs survive profile lowering, while
this work describes the surviving program to debuggers.

The ASL v2 prototype is a useful reference for the WebAssembly `name` custom
section: it names functions, locals, globals, GC types, and struct fields. It
does not contain a DWARF producer; Binaryen's `--debug-info` option only
preserves debug information that already exists. SplitScript must generate its
own metadata. Use the [WebAssembly name-section
conventions](https://webassembly.github.io/spec/core/appendix/custom.html#name-section),
the [DWARF for WebAssembly
conventions](https://yurydelendik.github.io/webassembly-dwarf/), and
[Wasmtime's native-debugging
contract](https://docs.wasmtime.dev/examples-debugging-native-debugger.html)
as the compatibility baseline.

Embedded DWARF is the source-level format. Do **not** generate a JavaScript
source map: it would duplicate a weaker line mapping and is not needed by the
Wasmtime host. Initially embed each `.debug_*` payload as its own Wasm custom
section. Revisit external debug files only if debug-module size becomes a real
problem.

### Establish the compatibility boundary first

- [ ] Build a minimal compatibility fixture against the exact Wasmtime version
  and configuration used by the LiveSplit host. It must contain scalar locals,
  a global, a source record represented by a Wasm GC struct, a GC-backed value,
  and several source lines. Run it with `Config::debug_info(true)` and the
  lowest practical Cranelift optimization level, then record which supported
  debugger/version combinations can set source breakpoints, step, show stacks,
  inspect scalar locals/globals, and inspect GC references and fields. Keep the
  fixture as a conformance test where automation is possible.
- [ ] Treat Wasm GC inspection as an explicit research result, not an assumed
  DWARF feature. `DW_OP_WASM_location` standardizes how a debugger finds a Wasm
  local/global/operand-stack value, but the current DWARF-for-Wasm convention
  does not specify how a debugger traverses a `structref`/`arrayref` or presents
  Wasm GC fields. Verify Wasmtime's native-DWARF transformation end to end
  before choosing a representation.
- [ ] If ordinary DWARF consumers cannot inspect GC aggregates, first expose
  the reference as an honest opaque value while retaining full stepping and
  scalar inspection. Then evaluate, in order: Wasmtime-supported GC debug
  metadata, a stable runtime/debugger visualizer API, and debug-only shadow
  storage. Do not describe Wasmtime's private moving-GC heap layout as a DWARF
  C-style struct or depend on internal native pointers. A custom debug adapter
  is justified only if the standard/native path cannot provide the required
  source-language view.

### Preserve source and finalized backend identities

- [ ] Add single-file debug source identity to the compilation input. The CLI
  supplies the `.split` path and source text; path-less library callers receive
  a deterministic synthetic filename. Decide and document whether the module
  stores an absolute path, a workspace-relative path plus compilation
  directory, or an explicit source-root mapping so debug artifacts are both
  usable and do not leak paths accidentally. This does not require general
  `FileId`/module support; extend the DWARF file table when a real multi-source
  feature arrives.
- [ ] Retain source provenance through typed HIR and Wasm IR for expressions,
  statements, terminators, lexical scopes, declarations, and async suspension/
  resume boundaries. `ExprId` already survives into Wasm IR, but statements
  and generated control flow need explicit origins so the backend never
  guesses spans by walking the AST again.
- [ ] Introduce one profile-aware `DebugArtifactPlan` after all type, import,
  function, global, GC-layout, local, and body plans are finalized. It owns the
  mapping from semantic identities (`FunctionId`, `ValueId`, source types and
  fields, lifecycle actions, and generated helper identities) to their actual
  Wasm indices and names. Name/DWARF emission must consume these existing plans
  instead of recreating index-assignment logic.
- [ ] Make encoded function bodies retain source marks at exact instruction
  boundaries. `wasm_encoder::Function::byte_len()` can capture body-relative
  positions while emitting; after every body is finalized, account for the
  Code-section function count, each body-size LEB, and local-declaration bytes
  to produce the Code-section-relative instruction offsets required by
  DWARF-for-Wasm. Verify every recorded address is an instruction boundary with
  `wasmparser`.
- [ ] Mark generated scaffolding as having no source location instead of
  attributing it to the nearest user statement. This includes transaction
  rotation, state polling, settings refresh, retry/cancellation machinery,
  async dispatch, wrapper construction, equality helpers, and ABI adapters.
  Stepping should stop at user statements, conditions, call sites, returns,
  and meaningful expression boundaries, not wander through compiler code.

### Emit readable names in debug modules

- [ ] Emit a `name` custom section only for `BuildProfile::Debug`. Name the
  module and every indexed entity for which the format has a useful namespace:
  imported ABI functions, runtime/equality helpers, user functions and
  methods, lifecycle and state-read functions, `_start`/`update`, parameters,
  source locals, deterministic compiler temporaries, source/runtime globals,
  memory, GC types, and GC struct fields. Use readable collision-resistant
  generated names such as `state.level.read` rather than raw numeric IDs.
- [ ] Drive type and field names directly from `GcLayout`, and function/global/
  local names from `FunctionPlan`, `GlobalPlan`, and each body's local plan.
  Add plan-level uniqueness/completeness checks so an added runtime helper or
  constructed GC type cannot silently remain `func[42]` or `type[17]`.
- [ ] Assert that release modules contain neither the `name` section nor any
  `.debug_*` custom section, and do not embed source paths or symbol strings as
  incidental debug metadata. Keep ordinary runtime-required strings and the
  SplitScript ABI/version marker unaffected.

### Emit DWARF incrementally

- [ ] Use `gimli::write` (which supports `DW_OP_WASM_location`) to emit a
  single compilation unit and the minimum interoperable embedded DWARF
  sections. Choose DWARF 4 or 5 from the Wasmtime/debugger compatibility
  fixture rather than from novelty. Parse the finished sections back with
  `gimli` in tests.
- [ ] Land source breakpoints and stepping first: emit the compilation-unit and
  subprogram DIEs, function address ranges, source file/directory metadata,
  and a `.debug_line` program mapping Code-section-relative instruction
  offsets to UTF-8-derived line and column positions. Represent lifecycle
  blocks and state-field readers with source-facing names and ranges even
  though they lower to generated Wasm functions.
- [ ] Add scalar type and variable inspection next. Emit exact signed/unsigned
  integer widths, `bool`, `f32`, `f64`, and `address`; function parameters;
  lexical local variables; and globals. Use DWARF location descriptions with
  `DW_OP_WASM_location` for Wasm locals and globals, location/range lists for
  real scope and lifetime, and an empty location when a value is genuinely
  unavailable. Never claim a stack temporary is a source variable.
- [ ] Model enums, records, arrays, `String`, `Option`, `Result`, `Duration`,
  async frames, and other GC-backed values only to the level proven usable by
  the compatibility fixture. Keep logical SplitScript types distinct from
  backend helper structs. If aggregate children cannot be inspected through
  standard DWARF, document that limitation rather than publishing misleading
  member offsets.
- [ ] Handle suspension explicitly. Before `await`/`retry`, a source binding
  may live in a Wasm local; after suspension it may live in the GC async frame
  and execute during a later `update` call. Emit correct disjoint line and
  variable ranges where representable, and otherwise mark the binding
  unavailable for the affected range. Stepping across suspension must resume
  at the next source statement rather than expose the async dispatcher.

### Integrate the runtime and VS Code workflow

- [ ] Add an explicit build-profile marker to the SplitScript metadata section
  so the host can reject a release module for a source-debug session. Configure
  the LiveSplit Wasmtime engine with debug information enabled and low
  optimization for debug sessions, while retaining the normal release engine
  configuration for published autosplitters. Measure startup, tick, and memory
  overhead rather than enabling native debug transforms for every release run.
- [ ] Prove a manual native-debugger workflow before building editor UI:
  compile/watch a debug module, load it in the real host, attach a supported
  debugger, set a breakpoint in the `.split` file, and step through attach,
  state polling, a lifecycle action, and an async retry/resume. Document source
  path mapping and hot-reload/restart expectations.
- [ ] Then add a VS Code debug workflow. Prefer a thin launch/attach integration
  with a supported native debug adapter (for example LLDB where Wasmtime
  supports it) over implementing another debugger. Only build a SplitScript
  Debug Adapter Protocol implementation if the GC/source-language inspection
  experiment demonstrates a concrete gap. Starting a debug session should own
  debug watch, wait for a successful module, start or attach to the host, and
  stop/restart cleanly without racing release builds.
- [ ] Add automated artifact tests for section presence/absence, name/index
  correctness, instruction-boundary and line-table round trips, type/location
  DIEs, deterministic output, and debug-versus-release size. Add an end-to-end
  Wasmtime trap/backtrace or debugger harness that proves a generated frame
  resolves to the expected `.split` function and line; keep a small documented
  manual matrix only for debugger behavior that cannot be made reliable in CI.
- [ ] Document the supported host, Wasmtime, debugger, and platform matrix;
  scalar and GC-variable inspection capabilities; async stepping behavior;
  source-path privacy; and the difference between `debug` declarations, debug
  build metadata, and release artifacts.

## P1 — Ship a self-contained portable VS Code toolchain

The native tools and the extension are separate delivery products built from
one implementation. `splitc` and `splitls` must remain first-class native
executables, tested and published separately for command-line, editor-neutral,
and automation use. The VS Code extension must bundle a WebAssembly build of
the same Rust compiler/tooling core: installing one VSIX must provide language
features, release compilation, and debug watch without downloading SplitScript
executables, finding tools on `PATH`, or selecting an OS-specific binary.

This is feasible without pretending that a language server must be an OS
process. A VS Code web extension runs in a browser worker, cannot spawn native
executables, and can run language servers in additional workers. VS Code's
official [`@vscode/wasm-wasi-lsp`
integration](https://code.visualstudio.com/blogs/2024/06/07/wasm-part2)
can run a `wasm32-wasip1` stdio language server and supports custom requests;
its URI bridge and workspace mount also cover virtual and remote workspaces.
The official [WebAssembly component-model
integration](https://code.visualstudio.com/blogs/2024/05/08/wasm) provides a
second path: expose an in-memory Rust API from `wasm32-unknown-unknown`, bind it
to TypeScript through WIT, and run expensive work in a worker. The latter can
use the browser's built-in WebAssembly runtime and avoid the separate [WASI
Core extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode.wasm-wasi-core),
whereas the former preserves the current stdio shell with less adaptation.

Do not equate "one extension" with "one mutable worker". Language queries must
remain responsive during code generation. It is acceptable—and likely
preferable—for the VSIX to instantiate the same bundled compiler module in one
long-lived language-service worker and one build worker. Consolidate source,
semantic state contracts, and packaged artifacts; do not force unrelated work
through one serial event loop merely to reduce the process count.

### Establish the shared product boundary

- [ ] Extract one transport-neutral Rust service boundary around the existing
  compiler and tooling facades. It accepts source text, a source identity and
  revision, compiler options, and cancellation; it returns structured
  diagnostics and generated Wasm bytes. `splitc` remains the native filesystem/
  watch shell and `splitls` remains the native stdio LSP shell. Neither native
  executable may depend on VS Code or the WebAssembly adapter.
  - [x] Land the versioned in-memory request/response boundary with source URI,
    revision, build profile, structured diagnostics/fixes, bounded source input,
    and raw artifact bytes. Keep cancellation pending until it can interrupt
    real compiler stages rather than merely discard an already-finished build.
- [ ] Add a thin extension-only Rust adapter rather than compiling `src/main.rs`
  and its polling watcher into the browser. Decide whether this is a workspace
  crate when the second target is introduced; the compiler/catalog/query
  implementation must still have one owner and one test suite. The same commit
  must keep native `cargo build --bin splitc --bin splitls` working.
- [ ] Define a versioned build protocol such as
  `CompileRequest { uri, revision, source, profile }` and
  `CompileResponse { revision, diagnostics, artifact }`. Debug and release are
  output profiles of that request, not profiles of the embedded compiler
  module itself. Discard stale responses, support cancellation, cap input and
  output sizes, and never scrape human-readable CLI stderr in the extension.

### Prototype and choose the WebAssembly host deliberately

- [ ] Build two small end-to-end prototypes against the real compiler before
  committing the package architecture:
  1. `wasm32-wasip1` plus `@vscode/wasm-wasi-lsp`, preserving stdio LSP and
     adding a custom compile request;
  2. `wasm32-unknown-unknown` plus WIT/component bindings, with the existing
     JSON-RPC `LanguageServer` hosted behind worker messages and compilation
     exposed as an in-memory `list<u8>` operation.
  Compare VSIX size, cold/warm startup, hover latency while compiling, artifact
  transfer/copying, cancellation, memory recovery, virtual-workspace behavior,
  browser support, and maintenance cost. Record the decision in an ADR.
  - [x] Complete the direct core-Wasm protocol slice. A dedicated unpublished
    adapter crate builds for `wasm32-unknown-unknown`; the TypeScript binding
    sends JSON metadata and receives a compact envelope with generated Wasm as
    raw bytes rather than base64. The optimized module is 1,588,797 bytes, runs
    in Node's browser-compatible WebAssembly API with no host imports, compiles
    a real source file, and returns structured diagnostics for an invalid one.
    The native compiler library remains an `rlib`, so ordinary native builds do
    not produce an extension-only DLL.
  - [x] Put the direct adapter behind a dedicated worker and transfer generated
    artifact buffers back without copying them into JSON. A real worker test
    initializes the bundled module, compiles valid and invalid revisions, and
    shuts down deterministically; extension-host compilation never runs on its
    shared event loop.
  - [ ] Measure repeated full-size builds and memory recovery, then exercise the
    worker in a web extension host and virtual workspace. The first host adapter
    uses Node `worker_threads` for today's desktop extension; retain the common
    protocol and add a browser `Worker` adapter rather than leaking Node types
    into the compiler binding.
  - [ ] Complete the matching WASI stdio/custom-request prototype and compare it
    against the direct result. The existing unmodified `splitls` already builds
    successfully for `wasm32-wasip1`, so the remaining experiment is host and
    extension integration rather than compiler portability.
- [ ] Prefer the direct component/worker design if it proves reliable because
  it is strictly self-contained and gives compilation a typed byte-array API.
  Choose WASI when preserving stdio and filesystem behavior materially lowers
  risk; if so, decide explicitly whether an automatically installed Microsoft
  WASI Core extension satisfies the self-sufficiency requirement. A custom LSP
  request is supported, but large Wasm artifacts must not become inefficient
  JSON number arrays or unbounded base64 payloads without measurement.
- [ ] Add `cargo check` and tests for the selected WebAssembly target early.
  Keep filesystem paths, environment lookup, process spawning, and polling in
  native/VS Code shells. The existing compiler library is already principally
  in-memory and `splitls` is only a small stdio wrapper; preserve that useful
  boundary instead of adding conditional platform code throughout the core.

### Replace extension process orchestration

- [ ] Add a `browser` entry and bundle the extension JavaScript, worker code,
  generated bindings, and compiler Wasm into the VSIX. Use
  `ExtensionContext.extensionUri` plus `vscode.workspace.fs`; remove runtime
  dependence on Node `fs`, `path`, `child_process`, `process.platform`, and
  `Uri.fsPath`. The same web-extension implementation should run in desktop VS
  Code, remote extension hosts, Codespaces, `vscode.dev`, and `github.dev`.
  - [x] Add the browser manifest entry, a single-file esbuild bundle, and
    separately bundled compiler and language-server browser workers. Shared
    activation/build orchestration contains only VS Code and web-platform APIs;
    the Node and browser entries inject their own worker clients. A bundle
    harness executes both generated browser workers against the real compiler
    Wasm and rejects external imports other than `vscode`.
  - [x] Exercise the packaged result in an actual VS Code web extension host
    and `@vscode/test-web` virtual workspace. The Chromium acceptance test
    activates and restarts the real LSP client, verifies hover, performs a
    release build, starts debug watch, writes through the virtual filesystem,
    saves a changed document, observes a distinct rebuilt module, and stops the
    watcher. Keep the broader failure/cancellation/platform matrix below open.
- [x] Replace native executable discovery and the public
  `server.path`/`server.arguments`/`compiler.path` settings. The installed
  extension must work offline from its bundled assets and must not ask users to
  install Rust or separate SplitScript releases. A repository-development
  override, if still useful, belongs in developer tooling rather than the
  published product contract.
  - [x] Remove compiler and server path settings and all `splitc`/`splitls`
    discovery and spawning. The desktop extension now starts the shared Rust
    language-server handler in a dedicated worker over direct JSON messages.
- [x] Keep the language client thin and preserve LSP as the editor-neutral
  protocol. A worker/postMessage transport or a WASI stdio transport is an
  implementation detail; hover, completion, semantic tokens, diagnostics,
  definitions, formatting, inlay hints, and custom build requests must all
  consume the same compiler-owned semantic snapshot and symbol identities.
- [ ] Make VS Code own debug-watch orchestration. Listen for saves or relevant
  document changes, debounce and cancel superseded builds, request
  `BuildProfile::Debug`, and write the returned bytes through
  `vscode.workspace.fs`. Explicit Build Release requests
  `BuildProfile::Release`. Preserve exclusive build/watch state, use exact
  document revisions, and define safe output/rename behavior for file and
  virtual workspace providers; do not embed the CLI's filesystem polling loop.
  - [x] Compile immutable, revision-tagged source snapshots on save, coalesce
    queued saves, discard superseded worker responses, and atomically replace
    the neighboring module through `workspace.fs`. Explicit release builds use
    the same path with the release profile and reject a result if the document
    changed while it was being built.
- [ ] Keep interactive language work responsive. Benchmark a single custom-LSP
  compile request against a dedicated build worker/module instance and choose
  the latter if compilation delays requests or retains too much memory. The
  extension should load/compile bundled Wasm bytes once where possible, then
  instantiate isolated services with deterministic shutdown and restart.
  - [x] Isolate compilation in its own long-lived worker immediately. The worker
    initializes the optimized module once, serializes build requests, transfers
    artifact ownership back to the client, rejects all pending requests if it
    exits, and is terminated with the extension controller.
  - [x] Give interactive LSP work a separate long-lived worker and Wasm instance.
    `vscode-languageclient` talks through its standard message transports, so
    the Rust handler still owns diagnostics, hover, completion, semantic tokens,
    navigation, formatting, and inlay hints without parallel VS Code providers.
    Restart and shutdown deterministically replace/terminate the worker.

### Package, publish, and verify both delivery products

- [ ] Extend `cargo xtask check` with the WebAssembly target, generated-binding
  freshness, extension bundling, a VSIX-content audit, and web-extension tests.
  The VSIX must contain no `.exe`, ELF, Mach-O, platform directory matrix, or
  undeclared network bootstrap. Optimize and strip the embedded compiler Wasm
  independently of whether it emits debug or release autosplitter modules.
- [ ] Test the bundled extension through desktop and `@vscode/test-web`, using a
  local file workspace and a virtual workspace. Cover startup, restart, hover,
  a successful and failed release build, repeated debug rebuilds, stale-result
  suppression, cancellation, output writes, and worker failure recovery on
  Windows, Linux, and macOS without rebuilding the VSIX per platform.
- [ ] Keep a separate native release matrix for `splitc` and `splitls`, with
  archives, checksums, versions, and CLI smoke tests. Verify native and embedded
  builds against the same language conformance corpus so the convenient VSIX
  distribution cannot silently implement a different compiler.
- [ ] Document the two installation stories clearly: the VS Code extension is
  batteries-included, while native `splitc`/`splitls` downloads remain available
  for other editors, terminals, CI, and automation. Document supported VS Code
  desktop/web/remote environments, package size and memory expectations, and
  any accepted host-runtime dependency from the architecture decision.

### Offer the extension in a hosted browser IDE

Prefer hosting an open-source Code OSS web workbench with the SplitScript web
extension preinstalled over building a separate Monaco application. That gives
browser-only users the same command palette, settings, keybindings, extension
host, language features, and build workflow without maintaining a second
editor integration. `@vscode/test-web` proves the technical shape, but it is a
development server and downloaded Microsoft VS Code builds are not our
redistributable product. A production deployment must use the MIT-licensed
Code OSS sources (with our own product identity) or another explicitly
redistributable compatible workbench.

- [ ] Build a deployment-sized Code OSS web proof of concept with the packaged
  SplitScript extension installed by default. Record the exact upstream source
  and license obligations, product configuration and branding, extension-
  gallery decision, update strategy, static-hosting/server requirements,
  compressed transfer size, cold/warm startup, memory use, and maintenance
  cost. Do not turn `@vscode/test-web` into production infrastructure.
- [ ] Define browser workspace persistence and artifact delivery. Users must be
  able to create/import a `.split` file, retain it locally, invoke the existing
  **Build Debug** and **Build Release** commands, and download/export the
  resulting `.wasm`. Prefer standard workbench and filesystem-provider APIs so
  compiler and language-server code remains unchanged.
- [ ] Add a focused hosted-product layer for curated examples, new-file
  templates, bounded share links, documentation entry points, and a settings
  preview. Keep process attachment and the actual autosplitter runtime out of
  the ordinary browser sandbox unless a separately designed secure host bridge
  is present.
- [ ] Package the workbench, extension, workers, and Wasm with content hashes
  and a strict CSP; require no runtime network fetch outside the deployment's
  declared static assets and optional extension gallery. Test current Chromium,
  Firefox, and Safari, including worker restart, repeated compilation, memory
  recovery, persistence, and artifact download.
- [ ] Reuse the extension's conformance and `@vscode/test-web` suites against
  the hosted build. The hosted product must not add alternate language-service
  or compiler providers.
- [ ] Keep a custom Monaco shell only as a documented fallback if the Code OSS
  experiment shows unacceptable payload, hosting, licensing, customization,
  or long-term update costs. If needed, first extract a browser-neutral SDK
  around the existing compiler/LSP workers; do not fork language intelligence.

## P2 — Editor and multi-source evolution

- [ ] Introduce file identities only together with a real multi-source feature
  such as modules/imports. Most autosplitters remain one file; avoid forcing
  premature cross-file complexity into every span and query.
- [ ] Enrich diagnostics with focused labels, notes, and machine-applicable
  fixes as real confusing cases are found.
  - [x] Let contextual wrapper inference flow across equality operands before
    diagnosing constructors such as `None`, so `optionalValue == None` learns
    `T?` from the opposite operand just as assignments and returns do.
- [ ] Add snippets for state, settings, lifecycle blocks, match, records, and
  common standard-library patterns. Keep completion candidates compiler-owned
  and the VS Code client thin.

## P2 — Generated documentation

- [ ] Build a rustdoc-like HTML renderer from
  `StandardLibraryDocumentation`, the language catalog, and the complete
  symbol graph. It must not parse Rust authoring source or maintain a second
  API catalog.
- [ ] Generate canonical signatures from semantic schemes; link namespaces,
  types, fields, variants, capabilities, and related symbols by their common
  documentation identity.
- [ ] Publish machine-readable documentation for clients that cannot link the
  compiler library directly.
- [ ] Test that HTML, machine-readable output, hover, signature help, and
  completion resolve the same symbol identity and show the same focused
  example.

## P2 — Migration guidance

- [ ] Publish a compiler-checked “Porting ASL to SplitScript” cookbook driven by
  `luna-porting/manual`. Include complete recipes for module-qualified fields,
  signatures and relative pointers, attach-time discovered addresses, records
  and arrays, nested settings, game-time accumulation, filtered watcher state,
  version probes, and cancellation. Every recipe must compile in `xtask` so a
  porting agent cannot mistake an existing feature for a missing one.
- [ ] Add a generated capability index that says, for each common ASL concept,
  “supported directly”, “use this SplitScript pattern”, “planned”, or
  “intentional sandbox non-goal”. Cover `MemoryWatcher`, `DeepPointer`,
  `SigScanTarget`/`SignatureScanner`, `stringN`, duplicate versioned states,
  `settings.Add` parents and keyed access, `refreshRate`, `exit`, mutable
  `current`, boolean `update`, `TimeSpan`, timer/run metadata, and host UI.
  Generate links to canonical language/standard-library symbol identities
  rather than maintaining an unverified prose-only API list.
- [ ] Make that capability index drive porting-aware compiler diagnostics and
  LSP code actions, not documentation alone. When syntax or resolution strongly
  indicates an ASL concept, emit one focused diagnostic that names the concept,
  its support status, the canonical SplitScript construct, and a documentation
  link; suppress predictable follow-on errors from the same unsupported shape.
- [ ] Add machine-applicable local rewrites for unambiguous mappings such as
  `refreshRate = value` to `setTickRate(value)`, timer phase comparisons to
  `timer.state()`/`TimerState`, simple `TimeSpan.From*` calls to `Duration`, and
  recognized signature/deep-pointer spellings to `sig`, `process.follow`,
  `process.readRelative32`, or expression-backed state reads. Only offer a fix
  after name/type resolution proves that user-defined symbols are not being
  rewritten.
- [ ] Give structural ASL constructs targeted guidance even when one local edit
  is unsafe: duplicate version-labelled `state` blocks, `MemoryWatcher`/
  `StringWatcher`, `settings.Add` trees, boolean-returning `update`, assignments
  to `current`, `exit`, timer/run metadata, and fixed `stringN` fields. Route
  coordinated rewrites through `splitc migrate`; for planned features say so
  explicitly, and for sandbox non-goals explain the safe boundary rather than
  reporting a generic unknown symbol.
- [ ] Test porting diagnostics end to end through batch output, LSP diagnostics,
  hover/documentation links, and code actions. Each rule needs positive,
  shadowing/ambiguity, recovery, cascade-suppression, and fix-round-trip cases;
  applying every machine-applicable edit must yield canonical formatted source.
- [ ] Write guides for authors coming from old ASL/C#, TypeScript/JavaScript,
  and Rust. Cover syntax, lifecycle, numeric/address types, `Duration`, async
  retry/cancellation, `Option`/`Result`, settings, and process reads, including
  semantic differences rather than token substitutions alone.
- [ ] Move foreign-spelling knowledge into a structured migration catalog with
  source-language provenance, replacement kind, explanation, and applicability
  instead of growing ad hoc parser/checker branches.
- [ ] Complete the remaining unambiguous foreign-token fixes after the existing
  `function`/`const`/`var`/`null`/type-name diagnostics: JavaScript `===`/`!==`,
  `${...}` interpolation, TypeScript/CLR boolean and primitive spellings, and
  Rust `let mut`. Keep canonical output unique; do not accept aliases silently.
- [ ] Add type-aware fixes only after resolution so shadowed user symbols do
  not receive incorrect replacements. Cover `.ToString()`/`.Length`,
  `Math.Min`/`Max`/`Clamp`, null-coalescing, and common logging calls only where
  the inferred receiver/result makes the conversion equivalent. Ambiguous
  conversions should show an explanation without an automatic edit.
- [ ] Prefer diagnostics and code actions over accepting permanent aliases for
  foreign syntax. SplitScript has no production compatibility obligation yet.

## Ongoing — Port-driven language development

- [ ] Treat hand-reviewed files under `luna-porting/manual` as the authoritative
  migration evidence. The broad 1,613-file/1,175-candidate generated inventory
  is useful for frequency estimates, but a generated file that compiles is not
  evidence of semantic parity.
- [ ] Keep a structured review record per manual port: original source,
  targeted executable/version, faithful behaviors, deliberate omissions,
  blocking capability IDs, workaround quality, compiler revision, and runtime
  validation status. Distinguish complete, variant-limited, and behavior-limited
  ports rather than reporting all three as merely “compiled”.
- [ ] Re-audit deferred/manual notes against the current compiler catalog at
  milestones. The first review incorrectly treated some existing facilities as
  absent—basic signatures, arrays/records, nested settings, `onDetached`,
  `setTickRate`, and cancellation-aware retry—so stale copied compilers and
  missing documentation must not turn into duplicate feature work.
- [ ] Maintain a representative corpus spanning Unity Mono/IL2CPP, native
  games, emulators, pointer paths, signatures, settings trees, load removal,
  game time, cancellation, and unusual numeric layouts.
- [ ] Port splitters incrementally and record missing features here before
  generalizing them. Promote repeated game-independent patterns into the
  standard library and add compiler plus runtime conformance coverage.
- [ ] Use the corpus as formatter fixtures, LSP integration projects,
  documentation examples, and performance inputs.
- [ ] Do not call the language generally usable based only on Lunistice; require
  several unrelated production-scale ports and stable author feedback.

## Recommended execution order

1. Bootstrap privileged SplitScript standard-library declarations, migrate one
   complete namespace/type/method slice through every consumer, then move the
   remaining bundled library and remove the Rust authoring macro.
2. Implement bounded native strings/fixed arrays and versioned/discovered state
   layouts; these block the largest share of faithful manual ports.
3. Publish the compiler-checked ASL cookbook and capability index alongside the
   first port-driven features so existing scan/array/settings/lifecycle support
   becomes discoverable immediately.
4. Add collections/iteration, String/math/time ergonomics, and data-driven
   settings from concrete manual ports, not as disconnected general-purpose
   library growth.
5. Establish the shared native/WebAssembly service boundary and choose the
   extension host with the measured WASI-versus-component prototype. Package
   language features and compilation into one portable VSIX while retaining
   separately published native `splitc` and `splitls` executables.
6. Establish the Wasmtime/DWARF compatibility fixture, then implement debug
   names and source-line stepping before variable and Wasm GC inspection. Do
   not build VS Code debugger UI until the real-host native workflow works.
7. Add Unity Mono and the next engine/emulator provider, then evaluate timer
   metadata and structured async needs against newly unblocked ports.
8. Keep sandbox-sensitive host capabilities explicitly deferred until their
   security, consent, ABI, and cleanup contracts are designed.
