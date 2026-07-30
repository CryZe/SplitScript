# SplitScript roadmap

This roadmap is ordered by dependency and impact, not merely implementation
size. Language semantics should settle before editor tooling treats them as a
stable public contract.

Priority meanings:

- **P0** — correctness or foundational design needed by real autosplitters.
- **P1** — important language and API ergonomics once the foundations exist.
- **P2** — tooling, migration, build profiles, and ecosystem scaling.
- **Ongoing** — work that should continuously validate every other priority.

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
- [x] Generate standard type, field, and variant IDs together with their
  declaration rows, so adding an ordinary symbol cannot leave a parallel ID
  enum or inverse owner list out of sync.
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
- [ ] Simplify inference to known semantic `TypeId` values plus inference
  variables. Remove the parallel nominal variants and conversion tables in
  `ast::TypeRef`, `inference::Type`, and `BuiltinType` as their migrations
  complete.
  - [x] Collapse every standard-library nominal inference case into
    `Type::Standard(StdlibTypeId)` and every checked result into
    `TypeKind::Standard(StdlibTypeId)`; no library type has a dedicated
    inference or semantic variant.
  - [ ] Replace the remaining parallel core/source/constructed inference term
    variants with known semantic `TypeId` values plus inference variables.
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
- [ ] After modules and generic library functions exist, allow ordinary
  standard-library declarations and bodies to be authored in SplitScript and
  compiled into the same validated symbol graph. Keep host boundaries,
  representation primitives, and suspension/control flow as scoped
  intrinsics.
- [ ] Finish with full compiler, formatter, generated-documentation, LSP,
  extension, example-autosplitter, Wasm validation, and runtime regression
  coverage. Add a test proving that a new ordinary catalog record with fields
  becomes resolvable, documentable, completable, and code-generatable without
  adding a concrete-type match elsewhere.

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
      LevelOrScene.Scene(process.read.managedString(
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
- [ ] Keep most future library functionality in SplitScript source once modules
  and generic functions exist. Reserve compiler intrinsics for representation
  primitives, host boundaries, and suspension/control-flow operations that
  cannot be ordinary source code.

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
    managed-memory, and Unity/IL2CPP helpers into
    `codegen/runtime_helpers.rs`. Generate ordered core/equality body plans
    through one interface from resolved indices and discovered array support.
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
  through an enclosing type, such as `Array<T?>!`, without giving adjacent
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
  `process.read.managedString(address, maxUtf16Units)`, because a managed string
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
