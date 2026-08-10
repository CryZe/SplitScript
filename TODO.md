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
- Record host-runtime gaps found during ports in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md), with evidence and
  semantic requirements before proposing import spellings. Keep compiler-only
  work in this roadmap and implemented contracts in `docs/ABI.md`.
- Remove completed work from this file during the next roadmap update and
  summarize the milestone in the archive.

## Now — turn the bulk ASL ports into faithful migration evidence

The external porting campaign is valuable pressure-test input, not yet a
maintained corpus. Its 392 sources used the current compiler but were not run
against games or the repository's deterministic host fixtures. Several reports
miss facilities that were already available, which is evidence that our
documentation, examples, completion, or diagnostics did not make the intended
solution discoverable. Compilation success must not be mistaken for attachment
or behavioral parity.

- [ ] Triage every reported limitation against the current language and the
  source ASL before planning a replacement feature. For facilities that already
  exist, identify why the author missed them and improve the relevant search
  path: cookbook, standard-library docs, completion, hover, or a contextual
  diagnostic. Recompile only focused probes needed to reproduce a concrete
  report; do not rerun the campaign as though it used a stale compiler.
- [ ] Classify each external port as compile-only, runtime-verified,
  behavior-limited, or intentionally failing; compare the source ASL for
  semantic omissions instead of trusting generated port notes. Record the
  compiler revision for future campaigns as provenance, not because this
  campaign used a different compiler.
- [ ] Continue refreshing the ASL migration catalog and cookbook for
  non-lifecycle misunderstandings found by the campaign. The next concrete
  audit queue is host-backed timer/run metadata paths and collection mutations
  whose ordering or duplicate semantics are not already covered by arrays and
  sets. String/array length, collection count, `Convert.To*`, and the existing
  timer-phase API now have canonical guidance; consult the roadmap archive
  before re-planning any completed string, numeric, timer-state, or
  finite-settings slice. Do not add compatibility aliases.
- [ ] Use those maintained ports to decide each next implementation slice.
  Prefer recurring static settings, signed offsets, or focused string gaps over
  callbacks, reflection, UI prompts, or unrestricted filesystem access. Move a
  gap back down the roadmap when the current language can already express it
  clearly and only documentation or diagnostics were missing.

## P0 — unblock the next representative native ports

### Lifecycle semantics exposed by legacy ASL

- [ ] Determine whether a post-initial-snapshot attachment hook is needed for
  ASL `init` bodies that genuinely consume `current`/`old`. `onAttach` remains
  the suspending pre-snapshot discovery and layout-selection phase. Add a new
  block only if passing explicit reads or a guarded first `whileAttached` tick
  makes a maintained port materially less clear.
- [ ] Design a clear way for `whileAttached` to suppress the remaining timer
  actions for the current update when a maintained port requires ASL
  `update { return false; }` exactly. State-field rejection is not equivalent:
  it retains one candidate, whereas ASL refreshes the whole snapshot and then
  skips `isLoading`, `gameTime`, `reset`, `split`, and `start`. Avoid a
  `shouldEvaluate`-style concept or unexplained boolean return.
- [ ] Consider an exact process-exit-only action implemented by the generated
  update lifecycle. It would differ from `onDetached`, which intentionally also
  runs on initial entry into the detached state, and would remove the common
  `attachedOnce` guard. Settle its name and ordering before adding it.
- [ ] Keep ASL `shutdown` and exact `onStart`/`onSplit`/`onReset` events as host
  requirements rather than approximating them. Shutdown requires the host to
  invoke a teardown export before disabling, reloading, or dropping a module;
  timer events require the ordered lossless contract in R2. Track teardown in
  R6 of [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).

### State layouts, discovery, and process identity

- [ ] Add layout sharing or overrides only if a maintained port proves that
  repeated pointer paths across many versions are materially unmaintainable.
  Keep the selected physical layout auditable.
- [ ] Complete safe process/module version probes as ports require them:
  module enumeration/search, product-version identity, and a deterministic
  executable fingerprint. Prefer host metadata over unrestricted filesystem
  access or hashing an entire module inside Wasm.
- [ ] Finish the remaining official host ABI as typed language facilities,
  preserving semantics without exposing owned numeric handles or manual
  `free` calls. Timer segment history, skip/undo, executable path, host OS, and
  host architecture are available now. Design PID discovery/attachment around
  the language's single process-lifetime boundary; expose mapped ranges through
  bounded cooperative iteration and a `MemoryRange` record with typed flags;
  and represent recursive settings maps/lists/values as GC-owned collections
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
  storage, logical length, and amortized `push` now preserve aliases across
  growth; `[T; N]` remains fixed-length and does not advertise or accept
  size-changing methods. Next design bulk extension, indexed insertion/removal,
  value removal, `pop`, and `clear` from corpus evidence, with explicit
  bounds/failure and must-use behavior. Both array forms retain indexing,
  iteration, search, length, and `values[index] = value` replacement. Plain
  indexed assignment is non-structural and preserves aliases; add compound
  indexed assignment only with temporaries that evaluate the collection and
  index exactly once. Structural mutation invalidates active iteration without
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

- [ ] Add Unity Mono discovery from representative ports, using the same typed
  provider/catalog model as native processes, GBA, and Unity IL2CPP. Include
  class/field/object discovery and managed strings without reflection-shaped
  compiler exceptions.
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
  casts, monotonic delays, exact process-name matching, and helpers that take
  `old`/`current` snapshots explicitly. Add a concise source-to-target
  capability table that links to complete standard-library symbols instead of
  making authors search the language reference. Compile the owning maintained
  examples in `cargo xtask check`.
- [ ] Expand capability-driven diagnostics and code actions beyond the initial
  structured entries. Emit one focused explanation and suppress predictable
  cascades for recognizable ASL constructs, including legacy lifecycle blocks,
  timer member chains and member casing. Distinguish “supported under a
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
  fixed-width numbers, `Duration`, async cancellation, `Option`/`Result`,
  settings, and process reads—not only token substitutions.
- [ ] Include the canonical compiler identity already exposed by the compiler
  service and generated-module metadata in machine-readable port reports so
  future evidence remains reproducible.

## P1 — source-level debugging in debug builds

Debug builds should support breakpoints and source stepping in `.split` files;
release builds must remain stripped. Embedded DWARF is the source-level format.
Do not add JavaScript source maps.

### Prove the Wasmtime/debugger boundary

- [ ] Build a minimal fixture against the exact Wasmtime configuration used by
  LiveSplit. With `Config::debug_info(true)`, verify source breakpoints,
  stepping, stacks, scalar locals/globals, and GC references across supported
  debugger/platform combinations.
- [ ] Record Wasm GC inspection as an experimental result. The DWARF-for-Wasm
  convention locates Wasm values but does not by itself define traversal of
  `structref`/`arrayref`. If standard consumers cannot inspect aggregates,
  expose honest opaque references first; do not describe Wasmtime's moving GC
  heap as C-style memory.

### Preserve provenance and emit metadata

- [ ] Add one single-file source identity to compilation input, with a
  deterministic synthetic filename for path-less callers and an explicit path
  privacy policy. Do not introduce general `FileId` infrastructure before a
  real multi-source feature.
- [ ] Retain source origins for typed-HIR statements, expressions, control
  flow, lexical scopes, and async suspend/resume boundaries. Generated runtime
  scaffolding must have no source location.
- [ ] Build one profile-aware `DebugArtifactPlan` after function, global,
  local, GC-layout, and body indices are final. Record exact instruction
  boundaries during encoding and verify them with `wasmparser`.
- [ ] Emit the WebAssembly `name` section in debug builds from existing plans:
  functions, lifecycle/state readers, parameters, locals, globals, GC types,
  and fields. Assert that release modules contain no name or `.debug_*`
  sections and leak no source paths.
- [ ] Emit DWARF incrementally with `gimli::write`: compilation unit,
  subprograms and line table first; scalar types and variable locations next;
  GC aggregates only to the level proven usable by the compatibility fixture.
  Represent async variable locations honestly across suspension.

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
  `isLoading`, `onDetached`, `timer.state()`, and `setTickRate` already replace
  common ASL patterns.
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
- [ ] Add general floating-point power only when a maintained port needs a
  negative or non-integral exponent. Port and attribute a vetted implementation
  such as Rust compiler-builtins' MIT-licensed libm `pow`/`powf`, including its
  scaling helpers, rather than introducing an ad-hoc approximation or a host
  import. Keep `squared()` as the simple exact-intent API for exponent two.
- [ ] Add deterministic integer radix formatting for the Fallout 3/New Vegas
  `Convert.ToString(cellId, 16)` pattern. Keep ordinary `as String` tied to
  decimal Display; design the radix API around an explicit supported base,
  signed-value semantics, invalid-base failure, and case selection (or compose
  lowercase hexadecimal with `toAsciiUpperCase`) before implementing it in the
  source-defined standard-library model.
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

- [ ] Generate rustdoc-like HTML and machine-readable standard-library
  documentation from the canonical catalog and symbol identities. Signatures,
  links, hover, completion, and generated pages must agree and use the same
  focused compiler-checked examples.
- [ ] Add completion snippets for state, settings, lifecycle blocks, match,
  records, and common standard-library patterns. Keep candidates compiler-owned
  and the VS Code client thin.
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
  fields; `Option<None>` still needs distinct empty/present states;
  `Result<None>` retains tag/error; `[None]` retains its logical length. Keep
  all specialization behind one physical-layout abstraction so construction,
  matching, equality, field indices, DWARF, and codegen cannot disagree.
- [ ] Design explicit `throw`/`catch` boundaries later. Postfix `?` remains the
  ergonomic propagation operation and uncaught errors return through `Result`.
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

1. Audit the campaign's misunderstandings against the current compiler and
   improve the documentation, completion, and diagnostics that failed to reveal
   existing features. Defer process-name warnings until the host's
   cross-platform matching contract is settled.
2. Add the remaining smallest recurring game-independent pieces—repeated
   static settings and focused string operations—with runtime and editor
   coverage.
3. In parallel with stable language semantics, establish the Wasmtime/DWARF
   compatibility fixture and land debug names plus source-line stepping.
4. Harden and publish the bundled VSIX and native releases, then evaluate the
   hosted Code OSS workbench.
5. Add Unity Mono and the next emulator/engine provider from representative
   ports.
6. Keep physical `None` aggregate specialization and sandbox-sensitive host
   capabilities deferred until measurements or explicit product requirements
   justify them.
