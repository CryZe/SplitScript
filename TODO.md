# SplitScript active roadmap

This file contains only work that is still active, deliberately deferred, or
ongoing. Completed milestones belong in
[`docs/ROADMAP_ARCHIVE.md`](docs/ROADMAP_ARCHIVE.md), not as checked boxes here.

The roadmap is ordered by user and porting impact:

- **Now** — the next concrete goal;
- **P0** — work that blocks faithful autosplitters or protects a foundational
  compiler boundary;
- **P1** — important product, tooling, and language expansion after the current
  porting blockers;
- **P2 / deferred** — useful work that should wait for evidence or a cheaper
  dependency;
- **Ongoing** — evidence and maintenance expected alongside every priority.

General rules:

- Drive language and standard-library growth from reviewed, faithful ports.
- Prefer one reusable typed abstraction over compatibility aliases or
  game-specific compiler branches.
- Keep ordinary library behavior in `stdlib/standard.split`; reserve Rust for
  representations, validated intrinsics, runtime helpers, and ABI boundaries.
- Add compiler, runtime, formatter, and editor coverage in the same change when
  a feature crosses those surfaces.
- Treat reports about an already-supported facility as discoverability and
  diagnostic evidence. Lead authors to the canonical typed pattern instead of
  adding compatibility aliases or duplicate abstractions.
- Record host-runtime gaps found during ports in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md), with evidence and
  semantic requirements before proposing import spellings. Keep compiler-only
  work in this roadmap and implemented contracts in `docs/ABI.md`.
- Remove completed work from this file during the next roadmap update and
  summarize the milestone in the archive.

## Now — finish compiler-owned reference workflows

### Make compiler-owned documentation browsable inside the editor

Treat the in-editor reference as the primary documentation product. Use VS
Code's built-in Markdown preview rather than building a custom HTML webview.
Standalone HTML is a later renderer of the same model, not the next milestone.
The compiler and both native and bundled servers already expose hierarchical
standard-library pages through read-only `splitscript-docs:` Markdown documents;
VS Code opens the index beside a script, supports search, breadcrumbs, links,
and compiler-derived semantic code. Keep that working base and finish the
missing graph rather than rebuilding the viewer.

- [ ] Finish semantic code in generated reference pages. Signatures now use
  catalog identities for syntax coloring and navigation, and examples can use
  rustdoc-style hidden `# ` fixture lines so the compiler can map exact
  semantic tokens and standard-library definitions back onto visible code.
  Convert every remaining shared fixture-backed callable example to an exact
  hidden-context fixture, then require that the visible snippet occurs in and
  is type-checked as part of its validation program. Keep syntax highlighting
  as the recovery presentation for incomplete examples, never as a substitute
  for semantic identity when an example is valid.

## P0 — unblock the next representative native ports

### Lifecycle semantics exposed by legacy ASL

- [ ] Keep ASL `shutdown` and exact `onStart`/`onSplit`/`onReset` events as host
  requirements rather than approximating them. Shutdown requires the host to
  invoke a teardown export before disabling, reloading, or dropping a module;
  timer events require the ordered lossless contract in R2. Track teardown in
  R6 of [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).

### State layouts, discovery, and process identity

- [ ] Add layout sharing or overrides only if a maintained port proves that
  repeated pointer paths across many versions are materially unmaintainable.
  Keep the selected physical layout auditable.
- [ ] Complete the remaining safe process/module identity probes as ports
  require them: full module enumeration and a deterministic executable
  fingerprint. Waiting `process.module(name)` and synchronous optional
  `process.loadedModule(name)` now cover known-name discovery. Numeric PE file
  and product versions are available through one shared source-defined
  `VS_FIXEDFILEINFO` traversal. Prefer host metadata over unrestricted
  filesystem access or hashing an entire module inside Wasm.
- [ ] Finish the remaining official host ABI as typed language facilities,
  preserving semantics without exposing owned numeric handles or manual
  `free` calls. Timer segment history, skip/undo, executable path, host OS, and
  host architecture are available now. Design PID discovery/attachment around
  the language's single process-lifetime boundary. Mapped ranges are now
  exposed as a synchronous GC-owned `[MemoryRange]` snapshot with typed
  readable, writable, and executable flags. Dynamic declaration membership is
  available through `settings.contains(key)` without exposing host values;
  represent recursive settings maps/lists/values as GC-owned collections
  and a typed value enum. Preserve atomic `storeIfUnchanged` behavior when
  mutable settings data is eventually exposed. The settings declaration DSL
  remains the normal registration API, and `start`/`split`/`reset` blocks remain
  preferable to duplicate direct timer commands. Coordinate the host-owned
  portions through R1 and R3 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).
- [ ] Extend signatures only for corpus-proven gaps: reusable scan targets,
  fallback signatures, range/page selection, capture transforms, relative
  address decoding, and concise pointer-follow composition. Existing `sig`,
  scan, follow, and `readRelative32` APIs should be documented before new APIs
  are introduced.
- [ ] Add exact record layout controls only when a target requires them:
  offsets, padding/alignment, packing, and per-field endianness. Keep
  field-order native-endian layout as the default and diagnose overlaps and
  unsupported combinations.

### Polling, mutable watcher patterns, and settings

- [ ] Add conditional settings visibility or enablement only with explicit host
  semantics for persisted hidden values and parent changes. Until then,
  document that headings are visual and parent boolean settings must gate child
  behavior in source; do not let a compiling hierarchy imply behavior the host
  does not provide.
- [ ] Continue `[T]` as the growable ordered sequence instead of adding a
  separate `List<T>` type. Stable wrapper identity, replaceable capacity-backed
  storage, logical length, amortized `push`, and capacity-preserving `clear`
  now preserve aliases across growth and reset; clearing releases live GC
  references without reallocating. `[T; N]` remains fixed-length and does not
  advertise or accept size-changing methods. Source-defined `extend` appends a
  typed array and safely handles self-extension. Source-defined optional
  `pop` now composes indexed access with `removeAt`, returns `None` without a
  structural mutation when empty, and retains capacity. Equality-constrained
  `remove(value)` now removes the first match and reports absence without a
  structural mutation. Indexed `removeAt` shifts in place while preserving
  aliases and capacity, with explicit bounds behavior. Corpus review found no
  current indexed-insertion use, so defer that API until maintained-port
  evidence establishes its semantics. Both array forms retain indexing,
  iteration, search, length, and `values[index] = value` replacement. Plain
  indexed assignment, including compound operators, is non-structural and
  preserves aliases while evaluating the collection, index, and right operand
  exactly once. Structural mutation invalidates active iteration without
  allocating snapshots; preserve that rule for every future collection
  mutator.

### Standard-library and type-system boundaries

- [ ] Design the user-facing trait/type-class model around the existing
  source-defined capability graph. Begin with memory reading, `Display`,
  equality, numeric operations, and hashing; decide separately whether user
  programs can declare traits and implement them for their own types.
- [ ] Keep trait declarations, implementations, documentation, method lookup,
  and capability inheritance in the source-defined standard-library model,
  never in a parallel checker table.
- [ ] Let privileged standard-library declarations state suspension, retry,
  cancellation, and attachment requirements through metadata validated against
  the trusted intrinsic contracts. High-level async composition should remain
  source-defined.
- [ ] Add a custom capability handler registry only when the first capability
  cannot be expressed by declared membership, structural equality, structural
  memory layout, or a source-defined implementation.

### Engine and emulator providers

- [ ] Extend Unity Mono beyond the maintained ARTIFICIAL static-field and Himno
  static-singleton/object-path cases only when another representative port
  proves the next surface: managed strings, typed cross-class object chains,
  scene-manager access, V1 layouts, 32-bit PE, or ELF/Mach-O discovery. Keep
  each target family explicit and source-defined; do not add reflection-shaped
  compiler exceptions or silently guess offsets.
- [ ] Assess an Unreal provider only after representative `GWorld`, object, and
  name traversal ports establish the required surface.
- [ ] Add the next emulator provider from a real port—such as Dolphin, PCSX2,
  RetroArch, or DOSBox—without introducing parser or type-checker conditionals
  for emulator names.

## P1 — expand migration guidance and automated fixes

- [ ] Expand the compiler-checked **Porting ASL to SplitScript** cookbook from
  maintained ports. The first bounded-string, versioned-layout, one-shot, and
  detach-cleanup recipes are complete; add module fields, signatures and
  relative pointers, discovered addresses, records/fixed arrays, settings,
  game time, state filtering, cancellation, mixed-width pointers, numeric/index
  casts, monotonic delays, exact process-name matching, and reusable helpers
  that accept arbitrary snapshots explicitly. Snapshot-dependent helpers that
  use the contextual `old`/`current` values directly are now documented and
  compiler-checked. Add a concise source-to-target
  capability table that links to complete standard-library symbols instead of
  making authors search the language reference. Compile the owning maintained
  examples in `cargo xtask check`.
- [ ] Expand capability-driven diagnostics and code actions beyond the initial
  structured entries. Emit one focused explanation and suppress predictable
  cascades for recognizable ASL constructs, including legacy lifecycle blocks,
  timer member chains, member casing, result recovery, and string
  concatenation. Distinguish “supported under a
  different spelling,” “requires a semantic rewrite,” “requires host support,”
  and “intentionally sandboxed out.” Use machine-applicable rewrites only after
  resolution proves equivalence and that user-defined names are not being
  rewritten.
- [ ] Expand the structured foreign-spelling entries beyond the existing
  declarations, option value, strings, durations, and numeric types. Add new
  entries only for corpus-proven, unambiguous spellings that are not already
  handled by the type-aware callable suggestion machinery. Keep canonical
  syntax unique; do not add compatibility aliases. Do not diagnose
  JavaScript-style `${...}` because it validly means a literal dollar sign
  followed by interpolation in SplitScript.
- [ ] Write short compiler-checked guides for authors coming from ASL/C#,
  TypeScript/JavaScript, and Rust. Explain semantic differences—lifecycle,
  fixed-width numbers, `Duration`, async cancellation, `T?`/`T!`,
  settings, and process reads—not only token substitutions.
- [ ] Include the canonical compiler identity already exposed by the compiler
  service and generated-module metadata in machine-readable port reports so
  future evidence remains reproducible.

## P1 — source-level debugging after the debugger boundary is chosen

Debug builds should support breakpoints and source stepping in `.split` files;
release builds must remain stripped. Embedded DWARF is the source-level format.
Do not add JavaScript source maps. Further implementation is deliberately
paused until the JavaScript-debugger experiment is compared with the current
native Wasmtime path and a typed-IR interpreter; do not let partially working
DWARF displace current porting and language-correctness work.

### Prove the Wasmtime/debugger boundary

- [ ] Before extending native DWARF further, evaluate the JavaScript debugger's
  WebAssembly support against the real Wasmtime host, especially GC objects.
  If it still cannot provide a coherent SplitScript-level experience, compare
  a language-native debugger that interprets typed IR with continuing to adapt
  native debuggers. Do not commit to a custom DAP until this experiment shows
  which boundary actually preserves source values and control flow.
- [ ] Build a minimal fixture against the exact Wasmtime configuration used by
  LiveSplit. With `Config::debug_info(true)`, verify source breakpoints,
  stepping, stacks, scalar locals/globals, and GC references across supported
  debugger/platform combinations. Source stepping works in the Windows host.
  Wasmtime's native-DWARF transform now preserves source subprogram names and
  direct scalar local/parameter location lists: direct values use its required
  trailing `DW_OP_stack_value`. A Windows CodeLLDB run against the real host now
  resolves `setup`, `add`, and `whileAttached`, binds source breakpoints, and
  displays a live scalar local. Emit the supported `DW_LANG_C11` compatibility
  language until SplitScript has an LLDB plugin: `DW_LANG_lo_user` leaves names
  and locals hidden. Deliberately omit compilation-unit PC ranges so LLDB derives
  ownership from complete child subprogram ranges; Wasmtime 45's generic unit
  range transform drops native regions for non-monotonic control flow. Continue
  with parameter-liveness cases, globals, GC-backed values, and other debugger /
  platform combinations. Wasmtime 45 explicitly discards
  `DW_OP_WASM_location` for globals and operand-stack values, so compiler tests
  that merely decode those expressions are insufficient.
- [ ] Design a debugger-visible representation for source globals. Prefer a
  debug-only, runtime-independent shadow location in linear memory over
  hard-coding Wasmtime's private VMContext layout; update the shadow on every
  source-global write and prove scalar inspection before attempting GC values.
- [ ] Record Wasm GC inspection as an experimental result. The DWARF-for-Wasm
  convention locates Wasm values but does not by itself define traversal of
  `structref`/`arrayref`. If standard consumers cannot inspect aggregates,
  expose honest opaque references first; do not describe Wasmtime's moving GC
  heap as C-style memory.

### Preserve provenance and emit metadata

- [x] Carry one single-file source identity through parsing, lowering,
  checking, and backend planning. CLI builds use the absolute input path,
  extension builds use VS Code's native file path (or the URI for non-file
  documents), and intentionally path-less APIs use deterministic `input.split`.
  Do not introduce general `FileId` infrastructure before a real multi-source
  feature.
- [x] Retain source origins for all typed-HIR constructs. Expression and
  statement/control-flow origins now survive Wasm IR lowering and movement into
  async poll bodies, and explicit suspend/resume boundaries retain the original
  `await`/`retry` span while generated runtime scaffolding has no source
  location. Executable enum and aggregate global initializers map `_start` rows
  to their declarations; primitive constant expressions correctly have no
  executable breakpoint address.
- [ ] Extend the profile-aware `DebugArtifactPlan` beyond its completed final
  function-index and function-body maps. Expression instruction boundaries are
  recorded during encoding, rebased to Code-section-relative DWARF addresses,
  and verified against `wasmparser`; async suspend/resume boundaries use
  distinct line discriminators. Add GC-layout and async-frame-location plans.
- [x] Emit a deterministic WebAssembly `name` section for every imported and
  defined function in debug builds, including runtime helpers, generic source
  specializations, async init/poll functions, lifecycle/state readers, and the
  exported entry points. Release modules contain no `name` or `.debug_*`
  sections.
- [ ] Extend the same `name` section beyond the completed source-owned globals,
  parameters, and direct-function locals. Add GC types, fields, settings
  storage, and honest names for values moved into async frames after their final
  index plans expose stable source identities.
- [x] Emit deterministic DWARF v5 compilation-unit, source-backed subprogram,
  and expression line-table sections with `gimli::write`. Tests decode the
  result and require every row to land on a real Wasm instruction boundary;
  release output remains stripped.
- [ ] Extend the completed primitive scalar types, source-global/direct-function
  parameter locations, declaration-to-scope local ranges, and statement/control
  flow rows with enums and honest async-frame variable locations. Suspension
  and resumption already have distinct rows at the source `await`/`retry` span.
  Add GC aggregates only to the level proven usable by the compatibility fixture.

### Integrate the host and editor only after metadata works

- [ ] Add a debug-profile marker to the SplitScript ABI metadata and enable
  Wasmtime debug transforms only for debug sessions. Measure startup, tick, and
  memory overhead.
- [ ] Prove a manual native-debugger workflow in the real host before adding VS
  Code UI: attach, state polling, lifecycle actions, retry/await resumption, and
  module reload.
- [ ] Prefer a thin VS Code launch/attach integration with a supported native
  adapter. Build a SplitScript DAP only if the compatibility experiment proves
  a concrete source-language or GC inspection gap.
- [ ] Add deterministic artifact tests, backtrace/source-line coverage, and a
  documented host/Wasmtime/debugger/platform capability matrix.

References:

- [WebAssembly name section](https://webassembly.github.io/spec/core/appendix/custom.html#name-section)
- [DWARF for WebAssembly](https://yurydelendik.github.io/webassembly-dwarf/)
- [Wasmtime native debugging](https://docs.wasmtime.dev/examples-debugging-native-debugger.html)

## P1 — harden and publish the portable toolchain

The extension already bundles the same Rust compiler and language service as
optimized core Wasm in separate browser-compatible workers. Desktop and web
extension hosts support language features, release builds, and revision-safe
debug watch without external executables. Native `splitc` and `splitls` remain
separate first-class products. The architecture experiment is complete; the
remaining work is product hardening and distribution.

- [ ] Record the direct core-Wasm worker architecture and native/embedded
  conformance boundary in an ADR. Remove obsolete WASI-versus-component
  alternatives from product documentation unless a measured limitation forces
  reconsideration.
- [ ] Add cooperative cancellation points to expensive compiler stages so a
  superseded editor build can stop work rather than merely have its completed
  response discarded.
- [ ] Measure repeated full-size builds, warm language queries, memory recovery,
  and worker restarts in desktop, web, and virtual workspaces. Keep the language
  worker responsive while the separate compiler worker builds.
- [ ] Complete packaging audits in `cargo xtask check`: generated Wasm/binding
  freshness, optimized artifact size, VSIX contents, no native binaries or
  network bootstrap, worker failure recovery, stale-result suppression, and
  local/virtual output writes.
- [ ] Test one platform-neutral VSIX on Windows, Linux, macOS, desktop, remote,
  and web hosts. Publish native `splitc`/`splitls` archives separately with
  checksums, versions, smoke tests, and the same conformance corpus.
- [ ] Document batteries-included extension installation separately from native
  CLI/LSP installation, including supported hosts, package size, memory use,
  debug-watch output, and failure recovery.

### Hosted browser IDE

- [ ] Build a deployment-sized Code OSS web proof of concept with the packaged
  SplitScript extension preinstalled. Record upstream/license obligations,
  branding, extension-gallery policy, static hosting requirements, payload,
  startup, memory, and maintenance cost. `@vscode/test-web` remains test
  infrastructure, not a redistributable product.
- [ ] Define persistent browser workspaces and Wasm artifact download/export,
  then add curated examples, templates, documentation links, and a settings
  preview without forking the compiler or language service.
- [ ] Package the workbench and extension with content hashes and a strict CSP;
  test Chromium, Firefox, and Safari. Keep a custom Monaco shell only as a
  fallback if Code OSS proves unsuitable.

## P1 — remaining language and runtime breadth

- [ ] Design host-driven `onStart`, `onReset`, and `onSplit` timer-event
  actions. They must fire even when no game process or emulator is attached;
  they are timer lifecycle events, not aliases for process lifecycle blocks.
  Inside these actions, expose attachment-dependent roots (`process` or `gba`)
  and the whole `old`/`current` snapshots as typed options, so code must handle
  absence before reading members. Specify the snapshot captured for each event,
  whether `onSplit` observes the segment before or after advancement, ordering
  relative to polling and detach, reentrancy, and whether suspension is safe.
  Add host ABI, runtime, type-checking, hover/completion, and detached-event
  fixtures together; do not weaken availability in ordinary attached blocks.
  The current LiveSplit Wasm runtime only calls `update`; this requires a real
  upstream host contract before any event export may be implemented. R2 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md) is the canonical
  runtime-side requirement.
- [ ] Design a typed least-privilege timer/run API for timing method, category,
  attempt metadata, current segment/history, real time, game time, and run
  offset. Separate read-only snapshots from mutations, distinguish the
  monotonic `Instant` clock from timer real time, and add ABI support only where
  LiveSplit can expose stable semantics. Use the repeated `timer.CurrentTime`,
  `timer.CurrentSplit.Name`, `timer.Run.Offset`, and timing-method ports as the
  evidence ledger; coordinate the host side through R5 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).
- [ ] Document and test existing lifecycle mappings before adding host APIs:
  `isLoading`, `onDetach`, `timer.state()`, declarative `tickRate`, and dynamic
  `setTickRate` already replace common ASL patterns.
- [ ] Add structured async discovery combinators only as ports require them:
  timeout, race/select, bounded concurrent scans, and cancellation scopes. Do
  not expose threads or unconstrained background tasks.
- [ ] Broaden suspending control flow incrementally from real ports and add a
  host-executed conformance fixture for each new shape.
- [ ] Port one callback-heavy splitter such as Axiom Verge before designing
  first-class function values or closures. Determine whether a named-function
  table plus `match` is clearer, or whether stored callbacks materially reduce
  complexity; do not import C# delegates, reflection, or event subscription as
  compatibility concepts by default.
- [ ] Complete remaining ordinary library gaps when a port needs them:
  immutable String operations beyond the corpus-proven P0 slice, additional
  numeric operations, and typed time operations proven useful by maintained
  ports.
- [ ] Add an associative map only after a maintained port demonstrates a
  runtime key-to-value lookup that cannot be folded into `settings`, a record,
  a finite `match`, or parallel typed arrays. If that evidence arrives, design
  one typed `Map<K, V>` around the source-defined equality/hash capability
  hierarchy, stable GC identity, mutation-during-iteration rules, indexing
  absence semantics, documentation, and inference. Do not add C#
  `Dictionary<K, V>` as a compatibility alias; the A Plague Tale chapter table
  is compile-time settings data and is not evidence for a runtime map.
- [ ] Add general floating-point power only when a maintained port needs a
  negative or non-integral exponent. Port and attribute a vetted implementation
  such as Rust compiler-builtins' MIT-licensed libm `pow`/`powf`, including its
  scaling helpers, rather than introducing an ad-hoc approximation or a host
  import. Keep `squared()` as the simple exact-intent API for exponent two.
- [ ] Generalize first-class indexing beyond arrays only when another real type
  needs it. Design an operator protocol with inferred index and output types
  (the current single-receiver capability graph has no associated types), then
  make its declarations, documentation, completion, and lowering catalog
  driven rather than disguising the operation as a callable method.
- [ ] Explore Unity managed strings as a readable wrapper/derived layout while
  preserving their pointer chasing, length validation, and UTF-16 conversion.
- [ ] Add structural anonymous records only after named records prove materially
  noisy. Decide explicitly whether anonymous records are memory-readable.

## P2 — documentation and editor evolution

- [ ] After the in-editor documentation reference has proven the documentation
  graph and navigation model, add machine-readable export and rustdoc-like
  standalone HTML as additional renderers. Publishing HTML must not introduce
  a second hierarchy, link scheme, example store, or documentation source.
- [ ] Add document highlights for all occurrences of the symbol under the
  cursor and type-definition navigation for inferred expressions. Add folding
  ranges for declarations, blocks, multiline expressions, comments, and
  settings trees once their recovered-syntax boundaries are stable.
- [ ] Add call hierarchy after the compiler exposes one reusable call graph,
  multi-range formatting when it materially improves editor workflows, and
  debugger inline values together with the eventual debugging strategy. Do
  not prioritize implementation hierarchy, linked editing, document colors,
  or inline completion without a concrete SplitScript use case.
- [ ] Add completion snippets for lifecycle blocks, match, records, and common
  standard-library patterns. Module scope plus state, settings, and tick-rate
  declarations are grammar-aware already. Keep candidates compiler-owned and
  the VS Code client thin.
- [ ] Continue adding focused labels, notes, and machine-applicable fixes for
  real confusing cases rather than growing a speculative diagnostic catalog.
- [ ] Introduce file identities, modules, and imports only together with a real
  multi-source use case. Most autosplitters should remain pleasant as one file.

## P2 / deliberately deferred

- [ ] Design a contextual `default` literal backed by a source-defined
  `Default` capability. Like `None`, it may be assigned directly where the
  expected type or later constraints determine a unique target, but it must not
  silently become the fallback for failed inference. Define capability
  membership for primitives and standard-library types; make records defaultable
  only when every field is defaultable; and require an explicit decision for
  enums and collections rather than assuming a first variant or allocating
  implicitly. Keep `default` distinct from `None`: `None` is the unit value and
  absent option case, while `default` constructs the target type's declared
  default value. Add focused ambiguity/unsupported-type diagnostics, hover,
  completion, formatting, and source-defined capability documentation when the
  feature is implemented.
- [ ] Settle cross-platform process-name semantics with the host runtime before
  warning about extensionless native `state` names. ASL commonly omits
  Windows' `.exe`, but extensionless names are valid on Linux and macOS, so a
  compiler warning would create false positives without a target-aware
  contract. Decide whether the runtime should match executable identity
  portably, expose target-specific aliases, or retain exact names; only then
  add documentation, diagnostics, and attachment fixtures for all three hosts.
- [ ] Specialize physical aggregate layouts around zero-sized `None` only when
  measured size or allocation pressure justifies it. Records may omit unit
  fields; `None?` still needs distinct empty/present states; `None!` retains
  tag/error; `[None]` retains its logical length. Keep
  all specialization behind one physical-layout abstraction so construction,
  matching, equality, field indices, DWARF, and codegen cannot disagree.
- [ ] Design explicit `throw`/`catch` boundaries later. Postfix `?` remains the
  ergonomic propagation operation and uncaught errors return through `T!`.
- [ ] Coalesce non-overlapping async-frame slots only if real autosplitters make
  frame size material and cleanup remains cancellation-safe.
- [ ] Extend `debug` to more declaration kinds only when a concrete use case
  defines checking, reachability, and release erasure.
- [ ] Write an explicit sandbox policy before adding process writes/injection,
  arbitrary file access, network/process launching, modal UI, audio, or broad
  host control. Use stats-file game time, install-file discovery, injected load
  removers, and timing-method prompts from the ASL corpus as concrete policy
  cases. Prefer file settings and typed host metadata where they suffice.
  Dangerous capabilities require visible consent and cleanup semantics; some
  may remain intentional non-goals.

## Ongoing — evidence, correctness, and maintainability

- [ ] Treat hand-reviewed ports as authoritative migration evidence. Generated
  candidates estimate frequency but do not prove semantic parity.
- [ ] Keep a structured record per port: source, target build, preserved
  behavior, omissions, blockers, workaround quality, compiler revision, and
  runtime status. Distinguish complete, variant-limited, and behavior-limited
  ports.
- [ ] Re-audit old notes after major compiler milestones. Missing or
  hard-to-find documentation, weak diagnostics, and unrecorded compiler
  provenance must not become duplicate feature work.
- [ ] Preserve diagnostics that porters explicitly found useful. Keep focused
  regression tests for optional/value mismatches, unhandled `T!` values, and
  machine-applicable unused-variable fixes while improving adjacent guidance;
  a new help path must not replace a clearer primary error or reintroduce
  cascades.
- [ ] Maintain a representative warning-free corpus spanning native games,
  Unity Mono/IL2CPP, emulators, pointer paths, signatures, settings trees,
  loading, game time, cancellation, and unusual numeric layouts.
- [ ] Use maintained ports as formatter fixtures, LSP integration projects,
  documentation examples, runtime tests, and performance inputs.
- [ ] Keep `cargo xtask check` as the single local and CI verification matrix.
  Extend it whenever a product surface is added; generated Wasm/Wat belongs
  under ignored `target` directories.
- [ ] Review modules above roughly 1,000 lines when related work changes them.
  Split only at a named product, context, or visitor boundary; line count alone
  is not a reason to scatter shared mutable state.
- [ ] Add a generated large-catalog performance dimension when alternate
  catalog construction exists, covering validation, indexing, completion,
  hover, and documentation queries.

## Recommended execution order

1. Fix compound-token fission and formatter lexical boundaries, then lock in
   parse/format/parse equivalence across nested generics and the maintained
   corpus. No formatter may remain capable of corrupting compiling source.
2. Close the latest campaign's self-guidance gaps: contextual result/string/
   constraint diagnostics, compile-time checking and completion for literal
   setting keys, canonical missing-state guidance, and direct links from
   recognizable ASL constructs to their existing recipes.
3. Extend the working in-editor standard-library reference with language and
   migration topics, finish exact hidden-context examples, and link hover,
   completion, diagnostics, and native CLI topics into the same graph. Treat
   standalone HTML as a later renderer.
4. Keep irregular nested static settings explicit until another maintained
   port demonstrates a small reusable table abstraction; select the next
   concrete provider or host-contract fixture instead of inventing a settings
   metaprogramming language.
5. Harden and publish the bundled VSIX and native releases, then evaluate the
   hosted Code OSS workbench.
6. Add Unity Mono and the next emulator/engine provider from representative
   ports.
7. Resume source-debugging work only after the JavaScript debugger, native
   Wasmtime/DWARF path, and typed-IR interpreter have been compared against the
   same GC and async fixtures.
8. Keep physical `None` aggregate specialization and sandbox-sensitive host
   capabilities deferred until measurements or explicit product requirements
   justify them.
