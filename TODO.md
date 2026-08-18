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

## Now — make docs-first ASL porting semantically reliable

The fresh docs-only campaign at compiler revision `87d3650` produced 75
compiling `.split` files across 39 entries marked ported and 34 marked
ported-limited, plus 20 blocked and 3 source-missing entries. A clean compile
did not establish a faithful port: every native output used an extensionless
Windows process name, none used named state layouts, and several reports
declared existing APIs missing. Treat this campaign as discoverability and
semantic-review evidence, not as a conformance corpus.

### Correct false gaps before adding adjacent features

- [x] Extend the campaign behavior ledger with the first high-risk tranche
  against the ASL sources.
  [`docs/PORTING_CAMPAIGN_AUDIT.md`](docs/PORTING_CAMPAIGN_AUDIT.md)
  records exact-name attachment, alternate process names, named layouts,
  polling rate, timer state, managed strings, changed timer accumulation, and
  the remaining Unity scene and lifecycle questions for Arietta of Spirits,
  TUNIC, A Proof of Concept, Aim Climb, and 25 To Life. The audit confirmed the
  existing facilities together with a compiler probe; it did not treat clean
  compilation as fidelity or choose APIs for unresolved gaps.
- [x] Reclassify AoE2DE's aggregate blocker using its source. PE file versions,
  named layouts, timer state, and optional split index already cover most of
  the script. The residual host requirements are configured segment count and
  exact ordered reset notification; unknown-version fallback remains an
  explicit port policy choice rather than a compiler limitation.
- [ ] Audit every campaign output against its ASL source and classify each
  difference as preserved behavior, an intentional policy choice, an existing
  but undiscovered SplitScript facility, a genuine language/library gap, or a
  host-runtime gap. Do not accept `PORTED` from compiler success alone. Start
  with the high-risk silent mismatches already found: all native state names
  omit the `.exe` required by the current Windows host; no output uses a named
  `layout`; no output calls the existing `timer` namespace; `refreshRate` was
  dropped instead of using `tickRate`; `ulong` was incorrectly narrowed to
  `i64`; fixed/growable array operations were reported missing; and TUNIC
  manually decodes a managed string instead of using
  `process.readManagedString`.
- [ ] Re-review entries currently marked blocked before designing host work.
  `timer.state()`, `timer.currentSplitIndex()`, game-time pause/resume,
  `Module.fileVersion()` / `productVersion()`, process-name arrays, named
  layouts, settings families, growable `[T]`, and Mono static-singleton paths
  already cover parts of AoE2DE, COTM, Castle of Illusion, Borderlands, TUNIC,
  Circuit Superstars, and other reported blockers. Preserve the residual host
  gap only after removing these false premises.
- [ ] Promote a small corrected subset to reviewed fixtures: one exact-name
  native process, one process-name array, one multi-version named layout, one
  timer-state/split-index script, one `tickRate` script, one managed-string or
  Mono singleton path, and one genuinely unsupported host case. The first audit
  tranche and AoE2DE review identify candidates across all of these categories;
  correct and runtime-test them before promotion. Require an explicit behavior
  ledger and live runtime test where the game is available.

### Make the existing model discoverable from `splitc docs`

- [x] Put the current attachment-name contract in the generated `state`,
  Native-provider, and ASL-porting pages: state strings are exact host process
  identities, and a Windows executable candidate currently includes `.exe`.
  Show a process-name array next to the single-name form and explain that it
  handles alternate executable names in one autosplitter. Keep the separate
  cross-platform identity design deferred; documentation must describe the
  runtime that exists today without implying extension inference.
- [x] Front-load a complete multi-version example in the generated `state` and
  `layout` documentation and the ASL guide. It must show two named layouts,
  attach-time evidence, returning `StateLayout.*`, the common field interface,
  layout-specific refinement, and the unsupported-build
  `await process.closed()` path. The fact that none of the campaign outputs used
  layouts despite many multi-state sources proves that the current isolated
  `layout Name { ... }` example is insufficient.
- [ ] Add a concise ASL-concept index/checklist near the beginning of the
  porting guide. Link exact legacy concepts to canonical pages for versioned
  states, multiple process names, timer phase and split index, exit-time game
  time cleanup, `refreshRate`, numeric type spellings, module/file versions,
  settings families versus dynamic lookup, array length/growth, managed
  strings, and Unity static-singleton/object paths. Keep the detailed recipes,
  but make them reachable without reading the guide linearly.
- [x] Make `splitc docs QUERY` resolve exact canonical names and unambiguous
  foreign spellings directly, then render ranked results for broader queries
  instead of silently choosing one. Multiword queries do not need quoting. The
  compiler-owned ranking is also used by the editor and covers symbol names,
  summaries, details, migration diagnostics, and foreign spellings. Queries
  such as `timer.CurrentPhase`, `TimeSpan.FromMilliseconds`, `modules.First`,
  `refreshRate`, `multiple processes`, and `.exe` lead to the relevant
  canonical topics without requiring their SplitScript names first.
- [x] Give `splitc docs` a real terminal renderer instead of printing the
  Markdown/HTML representation used by the editor preview. Render headings,
  paragraphs, lists, borderless aligned tables, signatures, and examples as
  readable terminal text; collapse intra-document links to their visible labels
  because virtual reference paths are not useful in a terminal. Apply ANSI
  styling and SplitScript code highlighting only when stdout is a TTY, using
  the CLI's shared automatic color policy; redirected output must be stable
  plain text with no escape sequences or HTML tags. Keep this as another
  renderer over the compiler-owned documentation graph rather than parsing or
  maintaining a second documentation source.

### Close feedback loops without papering over language design

- [ ] Decide how a fallible state expression composes internal postfix `?` with
  a fallible final call. The natural expression
  `process.read<T>(process.follow(base, path)?)` currently reports that a state
  expression using `?` must not produce another result, forcing a helper
  function. Evaluate flattening the final `T!` into the state-field failure
  boundary, explicit propagation on both operations, and the effect on nested
  calls before changing the checker. Add parser, inference, diagnostic, and
  runtime-retention fixtures for the chosen rule.
- [ ] Review the `Duration` constructor surface with integer and floating-point
  inputs before adding migration-only guidance. Decide whether
  `fromMilliseconds` and the other unit constructors should accept a numeric
  capability while preserving exact integer conversion, whether overload-like
  source declarations are needed, or whether distinct whole/fractional names
  remain clearer. Include large signed values, inference, and precision in the
  decision; do not merely redirect integral arguments after type checking.
- [ ] Design semantic lints from failures that compiled cleanly. Evaluate an
  unused-setting warning (the campaign declared `allSkullsMode` but read an
  unrelated always-false global), a suggestion from literal
  `settings.enabled("key")` to the typed `settings.name` member, and checks for
  declared settings or state fields that never influence reachable behavior.
  Account for settings families, host-visible declarations, dynamic keys, and
  intentional display-only state before enabling warnings by default.
- [ ] Evaluate ordinary runtime ranges from the repeated bounded-index ports.
  Compare `for index in 0..count` with a source-defined range value and the
  existing `while` loop; keep settings-family ranges a compile-time DSL. Do not
  force index arrays merely to express iteration, but do not add a range concept
  until its inference, endpoint types, overflow, and inclusive/exclusive
  semantics are settled.

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
- [ ] Distinguish compile-time settings generation from genuinely runtime-named
  settings in port reviews. Bounded level and boss tables should use settings
  families, and statically known entries should use typed `settings.name`
  access. Consider runtime registration only for data discovered from the game
  itself, such as A Short Hike's dynamic tag dictionary, and specify
  persistence, ordering, duplicate keys, live UI updates, and typed value access
  before proposing a host API.
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

- [ ] Treat Unity scene snapshots as a now-proven provider gap. TUNIC,
  Anemoiapolis, Building 71, Cannibal Abduction, Chop Goblins, Assemble with
  Care, and Beeny need active/loaded scene names or indices and well-defined
  previous/current/loading semantics. First document and exercise the existing
  ARTIFICIAL static-field, Himno static-singleton/`MemoryPath`, and
  `readManagedString` surface so those are not mistaken for missing instance or
  string support; then design one typed scene API rather than reproducing
  `asl-help` callbacks.
- [ ] Extend Unity Mono managed-object support from corpus-proven residual
  needs: typed cross-class object paths, managed list/dictionary traversal, and
  dynamic typed tag values for Alba, A Short Hike, AER, Bzzzt, Circuit
  Superstars, and Assemble with Care. Separate stable singleton/field chains
  already expressible through `staticFieldPath` and `field` from collection
  enumeration that genuinely needs new library/runtime support. Keep target
  families explicit (V1, PE32, ELF, Mach-O) and source-defined; do not add
  reflection-shaped compiler exceptions or silently guess offsets.
- [ ] Assess an Unreal provider only after representative `GWorld`, object, and
  name traversal ports establish the required surface.
- [ ] Add the next emulator provider from a real port—such as Dolphin, PCSX2,
  RetroArch, or DOSBox—without introducing parser or type-checker conditionals
  for emulator names.

## P1 — expand migration guidance and automated fixes

- [ ] Expand the structured foreign-spelling entries beyond the existing
  declarations, option value, strings, durations, and numeric types. Add new
  entries only for corpus-proven, unambiguous spellings that are not already
  handled by the type-aware callable suggestion machinery. Keep canonical
  syntax unique; do not add compatibility aliases. Do not diagnose
  JavaScript-style `${...}` because it validly means a literal dollar sign
  followed by interpolation in SplitScript.
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

- [x] Add cooperative cancellation points to expensive compiler stages so a
  superseded editor build can stop work rather than merely have its completed
  response discarded. The shared compiler and service distinguish typed
  cancellation from diagnostics; the embedded worker retains opaque analysis
  and Wasm-IR stages, yields between them, and discards superseded debug-watch
  revisions before publication. Add finer-grained checks inside a pass only if
  measurement shows one stage still blocks for too long.
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
- [ ] Design the remaining typed least-privilege timer/run API without
  redeclaring facilities that already exist. `timer.state()`, optional
  `currentSplitIndex()`, segment history, skip/undo, and explicit game-time
  pause/resume are available now and first need better porting discovery. The
  residual host surface is timing method, category/game/attempt metadata,
  current segment name and run count, timer real/game-time snapshots, and run
  offset. Separate read-only snapshots from mutations, distinguish the
  monotonic `Instant` clock from timer real time, and add ABI support only where
  the host can expose stable semantics. Use the repeated `timer.CurrentTime`,
  `timer.CurrentSplit.Name`, `timer.Run.Offset`, category, and timing-method
  ports as the evidence ledger; coordinate the host side through R5 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).
- [ ] Add structured async discovery combinators only as ports require them:
  timeout, race/select, bounded concurrent scans, and cancellation scopes.
  Hades provides an immediate small case: wait for the first of several known
  module names without an infinite hand-written polling loop. Decide whether a
  source-defined `process.moduleAny(names)` is sufficient before introducing
  general future selection. Do not expose threads or unconstrained background
  tasks.
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
- [ ] Explore Unity managed strings as a readable wrapper/derived layout only
  beyond the existing `process.readManagedString(address, maxUtf16Units)`
  helper. Preserve pointer chasing, length validation, and UTF-16 conversion,
  and make the wrapper worthwhile through typed object-path composition rather
  than adding another spelling for the same read.
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
- [ ] After the immediate exact-name documentation work, settle cross-platform
  process identity with the host runtime before warning about extensionless
  native `state` names. ASL commonly omits Windows' `.exe`, but extensionless
  names are valid on Linux and macOS, so a compiler warning would create false
  positives without target knowledge. Decide whether the language should state
  a target/platform policy, the runtime should normalize executable identity,
  or declarations should provide target-specific candidates. Only then add a
  warning or migration rewrite, with attachment fixtures on all three hosts.
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
- [ ] Treat compiler-clean generated ports as hypotheses rather than successful
  ports. Audit attachment identity, selected builds, lifecycle/timer behavior,
  settings reachability, integer signedness/width, failure behavior, and omitted
  source branches even when no diagnostic fired. Record both false blockers
  (existing features reported missing) and false successes (compiling scripts
  that cannot attach or silently disable behavior).
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

1. Correct the current attachment-name, process-array, named-layout, timer,
   tick-rate, numeric, array, managed-string, and Unity object-path
   discoverability failures in compiler-owned documentation.
2. Choose the CLI documentation-search interaction and terminal renderer, then
   make foreign ASL/C# spellings and conceptual queries reach the same canonical
   graph as VS Code without printing Markdown, HTML, or virtual intra-doc links.
3. Semantically audit the fresh campaign and promote a small corrected,
   runtime-tested subset instead of treating all compiler-clean outputs as a
   corpus.
4. Review the state-field `?` boundary and numeric `Duration` constructor
   ergonomics with the user before implementing either design; then add the
   chosen diagnostics, documentation, and tests together.
5. Reclassify blocked ports after subtracting existing features, then design
   the proven Unity scene/managed-collection and remaining timer host surfaces.
6. Harden and publish the bundled VSIX and native releases, then evaluate the
   hosted Code OSS workbench.
7. Resume source-debugging work only after the JavaScript debugger, native
   Wasmtime/DWARF path, and typed-IR interpreter have been compared against the
   same GC and async fixtures.
8. Keep physical `None` aggregate specialization and sandbox-sensitive host
   capabilities deferred until measurements or explicit product requirements
   justify them.
