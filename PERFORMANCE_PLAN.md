# Compiler and generated WebAssembly performance plan

Reviewed: 2026-09-04, against `70998d4` plus the existing uncommitted
standard-library caching/indexing changes. At review time `HEAD` matched the
local `origin/master` reference; the performance work was in the working tree.

Start by eliminating repeated work and unnecessary output. The largest known
compile-time opportunity is reusing standard-library compiler products. The
lowest-risk output improvements are compact local declarations, shared ordinary
function signatures, and operation-level reachability for sets. General
optimization passes should follow those changes, rather than block them.

The original review and measurements below are retained as the starting point.
Implementation progress is tracked separately here; investigation artifacts
are under ignored `target/performance-review`.

## First implementation batch

Implemented on 2026-09-04:

- Cached immutable backend contract validation and indexed function declarations
  and Wasm bodies, including the generic-template fallback (part of order 2).
- Grouped adjacent script/async/start-function locals and interned ordinary
  function signatures across imports and definitions (order 3). Existing
  hand-written runtime local groups are unchanged.
- Added an eight-entry LRU cache of owned completion receiver facts, including
  failed probes, invalidated with the source revision (part of order 5). It
  retains no temporary database. This removes repeated probes at unchanged
  completion sites; the first request after an edit can still require a probe.
- Grouped specialization expressions by owner with stable expression ordering
  (order 6), and borrowed semantic snapshots when building highlights (the small
  first step of order 7).

Release sizes are now 34,526 bytes for Lunistice, 48,773 for Minish Cap, and
3,152 for the set runtime fixture. See [baselines](docs/BASELINES.md) for the
before/after results. These reductions do not include set-operation pruning.

Validation: the full `cargo xtask check` passed for this batch, including the new
cache/type/local regression tests, editor/browser workers, Wasm validation, and
host-runtime fixtures. Repeated Lunistice and Minish Cap builds are byte-identical.

Still open: detailed stage instrumentation, lazy release debug-name construction,
set-operation reachability, direct receiver selection before the first probe,
root-effect probe reuse, shared stage ownership, parsed standard-library reuse,
and the larger backend/optimizer work. The suggested delivery order remains a
guide to those remaining slices, not a list of fully completed milestones.

## Evidence and scope

There are three different performance concerns:

- Time spent executing SplitScript compilation and editor queries.
- Size and runtime cost of the **generated script module**, especially with
  `--profile release`.
- Size/startup of the compiler's own Wasm binary embedded in VS Code. This is a
  separate artifact and a secondary concern for this plan.

`cargo --release` optimizes the Rust compiler executable; SplitScript's
`--profile release` selects generated-code behavior. Measurements must identify
both. Rust Cargo profile settings do not optimize the script module's emitted
instructions.

### Existing latency measurements

The working-tree [baselines](docs/BASELINES.md) record release compilation
medians of 45.68 ms for minimal, 54.44 ms for Lunistice, 46.68 ms for cancellation,
and 49.58 ms for settings. Their earlier stage probe attributes about 24.9 ms to
parsing augmented standard-library source and only 0.067 ms to exact resolution.
These are prior measurements, not newly timed results from this review.

The same document records member-completion medians of 98.2 ms for the small
fixture, 153.0 ms for Lunistice, and 199.2 ms for the generated large fixture.
Even the unchanged-revision multi-query sequence takes 49.1/157.3/234.8 ms.
Those editor measurements are labelled `70998d4`, before the working-tree cache
change. Repeat them before assigning savings to another change. Separate valid,
syntax-invalid, and type-invalid fixtures: fast diagnostics that stop early
are not evidence of a fast successful semantic check.

### Fresh release-output inspection

Rebuilt `splitc` using `cargo build --release --bin splitc`, then compiled the
following with `--profile release`. Sizes below are section **payload** bytes;
the total also includes the module header and section envelopes.

| Fixture | Total bytes | Code | Types | Data |
| --- | ---: | ---: | ---: | ---: |
| `examples/lunistice.split` | 34,830 | 31,165 | 1,848 | 990 |
| `examples/minish_cap.split` | 49,307 | 43,200 | 1,691 | 2,991 |
| `tests/set_runtime.split` | 3,187 | 2,120 | 569 | 58 |

All three validate with `wasm-tools validate --features all`. None contains a
name or DWARF section. Each has a 160-byte `splitscript` custom section including
its envelope; removing that identity metadata would have little impact and is
not recommended. Debug companions were used for function-name attribution,
without adding names to release output.

Lunistice's two largest function bodies are the generated provider preparation
poller (11,595 bytes) and `UnityIl2Cpp` poller (6,806 bytes), together about 59%
of its code payload. Their body bytes match the named debug companions exactly.
Minish Cap's largest release body is 27,826 bytes; the debug companion identifies
a GBA discovery poller of the same size and corresponding signature. Its indices
differ between profiles, so this is supporting attribution rather than an exact
body-byte match. Large provider pollers deserve investigation, but their size
alone does not prove that their work is redundant.

The July baseline uses a different compiler and generated-code profile. Do not
treat “under 30 KB” as an established regression boundary or promise that the
small fixes below will restore that size. No history bisect is needed.

## Suggested delivery order

Each row should be a separately reviewable change. Effort estimates are relative
scope, not time commitments. Re-measure after each group before proceeding.

| Order | Work | Benefit | Scope / confidence |
| --- | --- | --- | --- |
| 1 | Add focused timing counters and output attribution | Makes improvements measurable | Small; measurement foundation |
| 2 | Cache fixed validation; index function/body lookup; avoid release debug-name work | Compiler throughput | Small; repeated work confirmed, savings unmeasured |
| 3 | Group local declarations and intern ordinary function signatures | Smaller release modules | Small to medium; duplication measured |
| 4 | Make set operations individually reachable | Smaller modules and less emission | Small to medium; unused bodies demonstrated |
| 5 | Reuse receiver facts and bounded completion probes | Warm editor latency | Medium; repeated semantic work confirmed |
| 6 | Group specialization expressions by owner | Compiler scaling | Small to medium; full-map scan confirmed |
| 7 | Share immutable stage products and failed-check results | Editor latency and allocations | Medium; repeated copying/checking confirmed |
| 8 | Cache parsed standard-library templates, then reusable semantics | Largest fixed-cost opportunity | Medium to large; ID/span design required |
| 9 | Investigate large async/provider code and lazy body lowering | Compile time, module size, runtime | Medium to large; savings require experiments |
| Later | Local release optimization passes, selective inlining, fine-grained incremental checking | Further improvements | Defer until the above is measured |

## 1. Measure the work that will actually change

Extend [compiler_baseline](examples/compiler_baseline.rs) with optional stage
timings or internal counters for user parse, augmentation/render/lex/parse,
resolution, checking, typed HIR/validation, Wasm lowering, specialization,
reachability/planning, and encoding. The public stages alone combine several of
these costs. Count user/library expressions, checked functions, lowered bodies,
concrete function instances, and synthesized completion databases.

Extend [tooling_baseline](examples/tooling_baseline.rs) with isolated repeated
member completion at the same revision/offset, trailing-dot and complete-field
requests, and type-error recovery. Measure actual body edits as well as its
current appended-comment edit. Record native and embedded-worker latency
separately, including process/worker cold initialization. The process-wide
standard-library bootstrap is intentionally excluded by the current warm setup.

Add a reproducible size report using the existing `wasmparser` development
dependency: section bytes, function body sizes with compiler-owned labels,
local declaration groups, exact signature duplicates, emitted helper kinds,
and static data. Keep profiling labels in the report, not in release modules.

Acceptance: sequential before/after runs with identical source, toolchain,
profiles, and warning policy; 200 compiler samples and at least 30 editor
samples after warmup. Do not benchmark while the full test suite is running.
Use counters and semantic assertions in tests, not wall-clock thresholds.

## 2. Remove small, repeated compiler work

- [codegen::compile](src/codegen.rs) calls `validate_intrinsic_effects()` and
  `unity_layout::validate()` for every module. The former walks the complete
  intrinsic/helper dependency graph. Cache these immutable descriptor checks
  once, while preserving validation failures and dedicated validation tests.
  Do not cache source-dependent semantic validation this way.
- [check_function_bodies](src/typeck/body_pass.rs) finds each SCC member by
  scanning `program.functions`. Build an ID-to-position index once. Recovery
  leaves holes in IDs, so an unchecked `functions[id.index()]` is incorrect.
  Reuse the same approach for function lookups in backend planning.
- [wasm_ir::Program::body](src/wasm_ir.rs) scans bodies and can scan again for a
  generic template fallback. Index exact owners and template declarations,
  preserving that fallback. Keep deterministic iteration independent of maps.
- [function_plan::declare](src/codegen/function_plan.rs) formats and stores
  debug names even for release, although [module assembly](src/codegen/module_assembly.rs)
  only emits them when debug artifacts exist. Make name construction lazy and
  debug-only. This saves compiler work, not release file bytes.

Acceptance: equivalent diagnostics, recovery behavior, and emitted bytes for
lookup/validation changes, accounting for intentional compiler identity metadata
differences between builds. Record stage savings before pursuing smaller scans.

## 3. Compact the Wasm encoding

### Adjacent local declarations

[script_functions](src/codegen/script_functions.rs) and
[async_state](src/codegen/async_state.rs) repeatedly use
`Function::new(local_types.into_iter().map(|ty| (1, ty)))`. This encodes each
local as a separate declaration group. Combine adjacent locals with exactly
equal Wasm value types; preserve local ordering and indices.

Inspection of the fresh binaries found **213 bytes** of local-declaration
payload savings in Lunistice and **369 bytes** in Minish Cap from this change
alone. These calculations regroup existing declarations, including reference
types; they exclude possible extra savings in enclosing length prefixes. They
are not measured optimized output sizes. A shared helper should also cover
hand-written runtime functions where adjacent groups can be combined.

### Ordinary function signatures

[imports::encode](src/codegen/imports.rs) and
[function_plan::declare](src/codegen/function_plan.rs) append a new type for
each function. Outside the GC recursive group, Lunistice has 78 function-type
entries but only 62 distinct exact signatures; Minish Cap has 86 versus 54.

Share an interner between imports and defined functions, keyed by exact Wasm
parameter/result types, including reference nullability and concrete heap-type
indices. Allocate in deterministic first-use order. Start with ordinary function
types after GC layout assignment; leave recursive groups, subtype contracts,
and nominal GC layout identity untouched. Do not merge two source layouts just
because their printed fields look alike.

Acceptance: validate both profiles; execute closure/function-reference, generic,
async, and GC runtime tests. Local grouping must preserve instructions and
indices; signature sharing may change type indices and must update every
consumer. Report actual whole-module savings without adding overlapping size
estimates together.

## 4. Emit only demanded set operations

This is a confirmed **Set-specific gap**, not evidence that every runtime helper
is emitted unconditionally. [function_plan](src/codegen/function_plan.rs) declares
all six operations for each reachable set type, and
[set_functions::compile](src/codegen/set_functions.rs) emits all six. Arrays
already distinguish demand for `push`, `removeAt`, and `clear` in
[reachability](src/codegen/reachability.rs); most runtime helpers also follow
explicit dependency roots.

A fresh probe using only `Set.new<u32>()` and `visited.length()` produces a
1,960-byte release module. Its unused `contains`, `insert`, `remove`, and `clear`
bodies occupy 66, 137, 144, and 40 bytes respectively: **387 body bytes** before
function/type entries and length prefixes. Body hashes match the named debug
companion. These are removable candidates, not an implemented saving.

Reproduction source:

```splitscript
state "game.exe" {}
let visited = Set.new<u32>()
whileAttached {
    setVariable("count", visited.length())
}
```

Track demand by concrete set type **and operation**. Expand internal edges such
as `insert -> contains`, plus needed equality/storage helpers, to a fixed point.
Use that plan consistently for declarations and emission. Avoid removing
equality helpers still required by a demanded operation.

Acceptance: new/length-only code contains no mutation/search bodies; insert-only
code retains its contains dependency. Run string, structural-equality, iterator
mutation/version, and existing set runtime coverage. Audit other helper families
with the same report before claiming this problem is widespread.

## 5. Stop rechecking a whole file for warm member completion

In [completion::analyze_receiver_database](src/completion.rs), path receivers
prefer a resolved method-call receiver. With a field such as `point.x`, there
may be a perfectly good expression type but no resolved call receiver; `?`
then discards the direct result. `infer_receiver` removes the member suffix and
constructs a fresh `CompilerDatabase`. That probe is not retained across
requests. Recovered receiver facts also force a probe. Root effect completion
has another fresh-database fallback in `completion_operation_analysis`.

First select the receiver expression explicitly and use its known type when
valid, preserving the distinction between `receiver.method` and
`receiver.method().`. Do not simply substitute an enclosing call's result type.
Keep recovery when facts are missing or unreliable. Next memoize a bounded
number of probes/results per source revision, keyed by completion kind, receiver
position, replacement range, and repair. Include failures. Cache owned semantic
facts or retain their owner; never reuse a probe's `TypeId` against another
database's type store.

Acceptance: repeated same-revision field completion performs no new semantic
check when direct facts exist; a necessary probe runs at most once per key.
Test partial calls, chained/optional/result receivers, generics, user methods,
syntax/type errors, edits, and multiple offsets. Preserve results and recovery
quality while reducing the warm-sequence latency in the existing benchmark.

## 6. Make specialization proportional to the functions it visits

[specialization::materialize](src/codegen/specialization.rs) collects an
`ExprId -> owner` map, then scans **the entire map for each reachable function
instance**, rejecting expressions from other owners. The scan component is
approximately `instances * all expressions`, including library expressions.

Build `owner -> ordered expression IDs` and a separate root-expression list in
one traversal. Each specialization then visits only its template's expressions.
Use stable ordering: the current hash-map traversal is not a suitable foundation
for deterministic constructed-type allocation. Preserve nested closure
ownership and capability/overload resolution.

Acceptance: counters show visits proportional to roots plus expressions in
visited instances. Test 50/500/2,000 unrelated helpers and multiple generic
instances, recursive functions, closures, and source-defined providers. Compare
behavior and deterministic output rather than assuming every changed type index
is a semantic regression.

## 7. Share immutable compiler products before adding incremental inference

[database/queries](src/database/queries.rs) clones recovered syntax into strict
parse, clones parsed/lowered programs to consume stage APIs, and clones the
semantic model when building highlights. `semantic_snapshot()` can run strict
checking, lose the semantic output on error, and perform recovering lowering
and checking again. A repaired-source path can add another database.

Start with the cheap borrow: hold the snapshot `Arc` while building highlights
instead of cloning its semantics. Then introduce shared immutable document,
syntax, and lowered products behind the stage APIs. Share ordinary strict and
recovering lowering where their inputs and invariants actually agree.

Unify the checking implementation around an internal result containing partial
semantics plus diagnostics; strict compilation accepts it only when all required
invariants pass. Reuse partial facts for tooling instead of doing the same
inference twice. Keep recovery placeholders out of typed code generation and
keep warning policy separate from semantic-cache invalidation.

Acceptance: unchanged editor results on valid and invalid source; failed strict
checks do not automatically cause duplicate inference; retained-cache and peak
allocation measurements show the ownership change is worthwhile. Use the
existing query/recovery tests and explicitly cover warning-policy changes.

## 8. Reuse standard-library templates across compilations

The current working-tree change caches rendered source and name indexes. It
does **not** cache parsing or checking. In
[augment_program_with_library_bodies](src/stdlib/library_bodies.rs), each source
is concatenated with every library body, lexed and parsed again, including the
already-parsed user prefix. Checking then processes all injected functions;
[validation](src/validation.rs) verifies their signatures/effects against the
bootstrapped metadata again. [Wasm lowering](src/wasm_ir.rs) also lowers all
non-erased function templates before backend reachability chooses emitted ones.

Stage this larger change:

1. Cache a parsed, immutable library template per validated `StandardLibrary`
   graph. Keep user-dependent provider preparation separate. Prototype assembly
   without reparsing the user prefix or lexing static library text.
2. Establish an explicit remapping boundary for every syntax identity and span:
   functions, expressions, bindings, constructed types, nominal declarations,
   and generated provider declarations. Existing visible-count assumptions and
   user-source diagnostics must remain correct. An internal library source/ID
   domain is a possible design, but does not require public modules.
3. Reuse validated generic signatures, typed templates, and effect facts only
   after separating library-owned identities from each compilation's mutable
   type/inference stores. Materialize user-specific instantiations on demand.

Do not reuse a checked synthetic program's raw IDs in another compilation. Do
not replace repeated lexing with repeated deep copies of owned token text and
assume it is faster; the existing baseline notes explicitly warn about that.
Measure clone/remapping cost before choosing the template representation.

Maintain an exhaustive library-validation path in build/tests and validation
for each injected graph. Source-specific constraints, user capability
implementations, overload selection, and provider/schema preparation must still
be checked. Merely omitting unused source bodies before checking can change
diagnostics and is not an acceptable shortcut.

Acceptance: materially reduce the measured augmentation/checking floor, with
matching diagnostics, editor identities, deterministic output, and runtime
behavior. Exercise multiple graphs/contexts, source lengths, Unicode, malformed
source, generic calls, custom capability methods, and managed providers. Land
parsed-template reuse independently if semantic reuse needs a larger redesign.

## 9. Investigate the large bodies and remaining backend work

The size evidence directs attention to generated provider preparation and
asynchronous discovery. [compile_async_body](src/codegen/async_state.rs) emits a
linear sequence of program-counter comparisons, loading the frame PC for each
state. Measure state count, dispatch bytes, repeated generated binding logic,
and polling runtime separately from the useful provider work.

Try a structured `br_table` dispatcher for sufficiently large state machines,
with small machines retaining the simpler form when smaller. Factor repeated
binding/discovery sequences into shared ordinary helpers where the encoded
call/signature/frame cost is lower. These are experiments: neither dispatcher
changes nor helper extraction is yet proven to shrink the observed modules.
Preserve retry ordering, suspension, break/continue, cancellation, attachment
lifetime, and debug source locations. Use Lunistice, Minish Cap, and the async
runtime fixtures as acceptance workloads.

Separately, consider lazy Wasm lowering of templates once the complete checked
program exists. It can avoid building IR for unused library bodies without
skipping source validation. Derive demand from the existing semantic/backend
contracts, including provider attachment/preparation, named function values,
closures, custom Display/Debug, constants, and capability/overload calls. Avoid
creating a second subtly different reachability definition merely to save a
pass. This primarily reduces compiler work; emitted functions already follow
reachability today.

## Deferred work and constraints

- **Release optimization passes:** begin with typed constant folding and dead
  branch cleanup, then tiny wrapper inlining under an explicit size budget.
  Run them after profile erasure and before final dependency/data/type planning,
  or recompute those products. [constant.rs](src/constant.rs) classifies some
  syntax-level constants; it is not a general evaluator. Preserve integer width,
  overflow/traps, floating-point signed zero/NaNs, evaluation order, mutation,
  fallible operations, and async behavior. Effect metadata alone does not prove
  an expression cannot trap. General inlining can enlarge the very pollers that
  dominate current output.
- **Fine-grained incremental checking:** first remove duplicate checks and the
  static library cost. Then measure remaining edit latency before introducing
  function/SCC invalidation. Full-sync text transport alone is not the principal
  measured bottleneck. If queued edits remain a problem, design version-aware
  cancellation/coalescing around the synchronous LSP/worker request path while
  preserving ordering and diagnostics for the newest version.
- **Compiler-in-Wasm packaging:** benchmark the embedded module separately.
  Explore a dedicated Cargo profile, LTO/size settings, and stripping for that
  artifact, measuring startup and compilation latency as well as download size.
  Do not add a script optimizer dependency to solve the compiler binary's size,
  or assume a smaller embedded compiler executes faster.
- **Avoid semantic shortcuts:** settings descriptions, state polling, failure
  values, and cancellation can remain observable even if a user expression does
  not read their result. Existing demand-driven helper, float-table, string, GC,
  and debug-section handling should be preserved. Remove additional work only
  with an explicit dependency/observability argument.

## Completion criteria

For each implemented slice, record before/after stage or query timings and
release size attribution in [docs/BASELINES.md](docs/BASELINES.md). Require
semantic/runtime equivalence, stable editor recovery, bounded caches, and
deterministic output. Run focused tests during development and `cargo xtask check`
before committing compiler/tooling changes. Debug metadata quality remains a
requirement even though only release module size is a primary optimization goal.

The first milestone is orders 1–6: measurable reductions in repeated work and
unneeded encoding, with no general optimizer or incremental inference framework.
Use those results to scope immutable-product sharing and standard-library reuse.

Initial review validation: `cargo xtask check` passed on the accompanying working-tree
caching/indexing changes, including Rust, editor/browser-worker, Wasm validation,
and runtime checks. The separate fresh release inspection above also validated
its three fixture modules. Implementation began after that review; see the
progress section above.
