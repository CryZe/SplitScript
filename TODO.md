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
- Treat every reported workaround, misunderstanding, and omission as product
  evidence, including when the intended facility already exists. Classify it as
  a documentation, compiler-guidance, tooling, language/library, or host-runtime
  issue instead of dismissing it as port-author error. Lead authors to canonical
  typed patterns without hiding genuine ergonomic gaps behind migration advice.
- Record host-runtime gaps found during ports in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md), with evidence and
  semantic requirements before proposing import spellings. Keep compiler-only
  work in this roadmap and implemented contracts in `docs/ABI.md`.
- Remove completed work from this file during the next roadmap update and
  summarize the milestone in the archive.

## Now — make Unity schemas a first-class language facility

Use the Rust Lunistice autosplitter and `examples/lunistice.split` as the first
acceptance pair. The current SplitScript Unity API is not a compatibility
boundary: replace its manual image/class/offset plumbing where the schema model
provides a clearer typed facility. Keep the generated implementation
demand-driven, allocation-conscious, backend-independent, and integrated with
the ordinary compiler symbol graph rather than adding Unity-shaped exceptions
to inference or code generation.

### P0.1 — represent managed metadata in ordinary compiler architecture

- [x] Add first-class `image`, `namespace`, `class`, field, static-field,
  metadata-name (`from`), and conditional-field declarations to syntax,
  AST/HIR, recovery, formatting, and source traversal. Declarations use
  mandatory semicolons and never depend on line breaks. Preserve documentation
  on every declared image, namespace, class, and field.
- [ ] Resolve image schemas through the normal semantic symbol tables. Give
  every generated class, reference, field, and static member a stable
  identity used by type inference, diagnostics, rename, go-to-definition,
  completion, hover, semantic highlighting, selection ranges, documentation,
  and unused analysis. Do not teach those consumers unrelated lists of Unity
  names.
- [ ] Define the backend-independent class model: `T.Ref` is a live remote
  managed reference; `T` is an immutable local snapshot. Class-typed managed
  fields produce references, terminal value fields produce fallible values,
  and `.snapshot()` reads the declared value shape transactionally. Generate
  snapshot readers only when reachable.
  - [x] Derive one backend-independent binding plan from checked schema and
    semantic identities. It preserves image and namespace ownership, class and
    layout alternatives, static versus instance fields, declared versus read
    value types, exact aliases, and conventional C# automatic-property backing
    candidates. Snapshot code generation consumes this projection instead of
    maintaining a parallel Unity class registry.
  - [x] Resolve and lower common static managed roots and fallible
    live-reference field reads through that plan. Every remote hop produces an
    ordinary `T!`, so paths such as `GameManager.instance?.player?.score?`
    expose each possible failure through the language's existing propagation
    model. Cached metadata is shared while object pointers are re-read live.
  - [ ] Generate reachable transactional `.snapshot()` readers through the
    same binding plan.
- [ ] Specify deterministic metadata-name resolution. A missing `from` uses the
  source member name, `from "name"` names one exact metadata member, and
  `from ["first", "second"]` names explicit alternatives. C# automatic-property
  backing fields participate in the same canonical lookup. Ambiguity is an
  error rather than declaration-order selection.
  - [x] Centralize canonical binding-name expansion on managed field
    declarations. Type checking and backend planning now share exact `from`
    alternatives and implicit source-name/backing-field candidates, and reject
    collisions between either form before runtime binding.
  - [x] Feed ordered, namespace-qualified class aliases into both Mono and
    IL2CPP attachment binders. The generated binder performs alias discovery
    once per attachment and retains the selected class metadata for later
    field binding.
  - [ ] Replace first-match alias selection with a complete binding probe and
    report zero or multiple runtime matches against the responsible source
    aliases. A missing candidate must be distinguishable from a transient
    process-memory failure so layout discovery cannot retry forever.
- [x] Replace independent state and managed-class layouts with one
  attachment-wide `layout: Layout` record composed from explicitly declared,
  enum-valued dimensions such as edition, storefront, renderer, or build.
  Matching one dimension refines every state field, managed field, snapshot,
  and attachment-scoped global conditioned on that value without requiring a
  cartesian product of public layout variants. Keep ordinary metadata aliases
  and equivalent class-binding alternatives private: a class must never expose
  a second public `.layout` selector merely because the same logical schema has
  multiple metadata spellings.
  - [x] Add dimension declarations to the state DSL and represent the generated
    `Layout` record and read-only `layout` value through ordinary record/value
    identities. Require finite enum-valued dimensions initially, preserve their
    documentation, and integrate formatting, recovery, navigation, completion,
    hover, semantic highlighting, reference docs, and unused analysis.
  - [x] Allow state and managed-class fields to be conditioned by ordinary,
    statically decidable predicates over global layout dimensions. Type-check
    those predicates once, reject runtime-dependent conditions, and use the
    same predicate representation for member availability, control-flow
    refinement, binding, diagnostics, and code generation.
  - [x] Make managed schema probes contribute constraints to the global layout
    dimensions. Probe results preserve both offsets and presence. When the
    complete set of conditional fields gives every bounded layout combination
    a distinct exact presence pattern, attachment selects the generated
    `Layout` automatically before user `onAttach` code runs. Otherwise require
    an explicit `onAttach` return and explain why automatic selection is not
    decisive. The compiler bounds the product rather than eagerly enumerating
    an unbounded cartesian product, and a failed automatic match rejects that
    process for the remainder of its lifetime.
  - [x] Remove generated `<Class>.Layout` enums, `<Class>.layout` values, and
    per-class public refinement. If a genuinely observable class-only schema
    distinction is needed, declare it as another global dimension; if the
    distinction preserves the public API, keep it as a private binding
    alternative.
  - [x] Give both backends one internal three-state field probe that separates
    transient read failure, completed absence, and a found offset. Generated
    attachment binding probes complete alternatives without adding a public
    lookup API or duplicating the metadata scanner. Reuse this mechanism as
    the low-level evidence source for global layout constraints.
  - [ ] Replace the temporary inert zero/multiple-match behavior with a focused
    attachment diagnostic carrying labels for the responsible layouts.

### P0.2 — bind schemas once per attachment

- [ ] Introduce a single internal Unity metadata adapter shared by Mono and
  IL2CPP. Keep native metadata traversal and process scanning intrinsic, but
  generate high-level bindings and reads from the source schema. Replace the
  public split between `UnityModule`/`UnityClass` and
  `MonoModule`/`MonoClass` where it no longer expresses a real semantic
  difference; retain a deliberately low-level dynamic escape hatch.
- [x] Make Unity an attachment/state provider with an automatic form and
  explicit backend/version forms, including `state Unity [...]`,
  `state Unity.il2cpp(2020) [...]`, and `state Unity.mono(...) [...]`.
  Provider setup is cooperative and cancelled with the process. Ordinary state
  fields can read generated managed members without manually retaining classes,
  static tables, instance addresses, or offsets.
  - [x] Generalize state syntax so a named provider can own source-declared
    process candidates (`state Provider ["game.exe", ...]`), with catalog-mode
    validation for providers whose process list is instead fixed. Keep provider
    names in their contextual namespace so the future `state Unity` provider
    can coexist with the existing `Unity.*` API namespace.
  - [x] Model qualified provider configurations in the privileged catalog and
    ordinary syntax. `state Provider.selector(...) [...]` uses normal typed,
    compile-time-constant expressions; completion, hover, semantic
    highlighting, formatting, selection ranges, and provider documentation are
    derived from the same selector declarations. Distinguish source-process
    providers from the single catalog-declared default so Native remains the
    meaning of bare `state "..."` while Unity can also accept source candidates.
  - [x] Publish the Unity provider with a compiler-owned preparation phase that
    completes before user `onAttach`. Automatic mode detects Mono versus IL2CPP
    from loaded runtime modules and Unity metadata; explicit selectors retain
    typed version arguments.
- [x] Cache common image, class, static-table, and field-offset metadata for one
  attachment. Re-follow dynamic object pointers on each read so replaceable
  singletons remain correct. Scalar live paths allocate no fresh GC object per
  tick.
- [x] Extend the attachment cache with managed field offsets and presence
  evidence used to validate the attachment-wide layout. Strings, arrays, and
  explicit snapshots may still allocate their returned values.
- [x] Preserve the existing transactional state-field failure boundary for
  generated member reads. A failed pointer hop or memory read retains the last
  accepted field value; Unity declarations must not introduce a second failure
  model.

### P0.3 — prove the design by simplifying Lunistice

- [ ] Declare the Lunistice `GameManager` and `Timer` schemas, including shared
  fields, base-game and DLC-demo layouts, alternative singleton names,
  `LevelTimeParts`, and the bounded DLC scene string.
- [ ] Port `examples/lunistice.split` from manual class/instance/offset globals
  and raw `process.read` calls to generated references or snapshots. Preserve
  all existing autosplitter behavior and keep the user's current local example
  edits out of intermediate mechanical rewrites.
- [ ] Add synthetic runtime coverage for both base and DLC metadata layouts,
  singleton replacement, backing-field lookup, inherited fields, failed reads,
  and ambiguous layouts. Compare the resulting script structure and behavior
  with `C:\Projekte\lunistice-auto-splitter`.
- [ ] Treat the milestone as complete only when all generated Unity symbols are
  navigable and documented in the editor and reference viewer, and the port no
  longer contains manual Unity metadata offsets or attachment bookkeeping.

### P1 — complete the Unity object model on the same foundation

- [ ] Add cooperative `T.instances()` scanning by managed class/vtable. Bound
  work per poll and preserve process cancellation; never hide an unbounded
  synchronous process scan behind an ordinary-looking method.
- [ ] Unify the existing scene snapshots under `unity.scenes`, including active,
  loaded, and don't-destroy-on-load scenes. Add typed hierarchy lookup and
  `.component<T>()`, with an explicitly dynamic escape hatch when no schema is
  declared. Cache or cooperatively traverse hierarchy data so ticks remain
  responsive.
- [ ] Expose Unity global managers such as `unity.time.frameCount` and
  `unity.time.timeScale` through reachable source-defined declarations rather
  than bespoke compiler names.
- [ ] Design bounded managed-string declarations and managed storage syntax
  before implementing collections. Managed arrays and managed lists both
  materialize as `[T]`; do not reintroduce a public `List<T>` value type.
  Dictionaries and dynamic typed values need a separately approved language
  design based on representative ports.
- [ ] Generate complete Unity reference documentation and migration guidance,
  including schema declarations, layout refinement, live references versus
  snapshots, failure behavior, allocation behavior, Mono/IL2CPP selection, and
  the low-level escape hatch.

## P0 — make docs-first ASL porting semantically reliable

The fresh docs-only campaign at compiler revision `87d3650` produced 75
compiling `.split` files across 39 entries marked ported and 34 marked
ported-limited, plus 20 blocked and 3 source-missing entries. A clean compile
did not establish a faithful port: every native output used an extensionless
Windows process name, none used named state layouts, and several reports
declared existing APIs missing. Treat this campaign as discoverability and
semantic-review evidence, not as a conformance corpus.

### Turn every reported blocker into an actionable product outcome

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
- [x] Turn campaign feedback into targeted, minimized reproducers rather than
  exhaustively auditing every generated port. Classify each reported friction
  point as a discoverability or guidance failure around an existing facility, a
  compiler or diagnostic bug, a genuine language/library gap, or a host-runtime
  gap. An existing implementation does not invalidate the report: record why
  the intended route was missed and improve documentation, search, completion,
  diagnostics, or API ergonomics as appropriate. Inspect the corresponding ASL
  and `.split` files only when the report does not establish the intended
  behavior or when a clean compile may hide a semantic mismatch. The known
  high-risk examples include omitted
  `.exe` identities and named layouts, dropped `refreshRate`, incorrect
  `ulong` narrowing, missed fixed/growable array operations, and manual managed
  string decoding. The focused audit now records exact-name attachment and
  named layouts as discoverability failures, maps `refreshRate` to `tickRate`,
  preserves `ulong` state reads as `u64` while isolating the exact Duration
  conversion question, validates fixed and growable array patterns, and
  separates managed-string decoding from the genuine managed-collection gap.
- [ ] Follow through on entries reported as blocked before designing adjacent
  host work, using a focused source comparison and compiler probe rather than a
  full-file audit. Every report must produce a concrete product outcome even
  when the required feature already exists: strengthen its canonical docs and
  search path, add contextual editor/compiler guidance, improve the API, or
  record a genuine language/runtime boundary. Do not close an item merely by
  relabeling the porter's conclusion as incorrect.
  `timer.state()`, `timer.currentSplitIndex()`, game-time pause/resume,
  `Module.fileVersion()` / `productVersion()`, process-name arrays, named
  layouts, settings families, growable `[T]`, and Mono static-singleton paths
  already cover parts of AoE2DE, Borderlands, TUNIC, and other reported
  blockers. Their omission is evidence that those facilities were not led to
  strongly enough. AER's report demonstrates that raw `MemoryPath` polling,
  explicit `SetZeroOrNull` fallback, one `tickRate` declaration, and automatic
  attachment cancellation cover its timer-critical loading behavior without a
  managed-object bridge or dynamic watcher registration; its auxiliary sound
  and modal UI remain fidelity-ledger omissions. Bzzzt's report likewise shows
  that partitioned compile-time settings families preserve its 51
  bounded keys and exceptional defaults, while `staticFieldPath` plus
  `dereference` follows its replaceable `Main.instance` fields. Its family is
  verbose under the current uniform-default rule, but it does not require
  runtime settings registration. Assemble with Care's base/derived metadata is
  also composable through `staticTable`, `field`, and `MemoryPath`; only its
  loading-scene snapshot remains a timer-critical provider gap.
  Crazy Machines exposed weak discovery of process-name arrays, named layouts,
  `process.name()`, and `tickRate`; the generated state and layout documentation
  now presents that complete composition, while live validation of the legacy
  identities and offsets remains. Hades exposed both a guidance gap around an
  address cursor plus `while` and a genuine ergonomic gap in retrying a group of
  known module alternatives. Its two module names now compose through
  `loadedModule` and the implemented whole-expression retry boundary, while
  arbitrary future prefix matches remain a narrower host enumeration question.
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
- [x] Turn the first repeated legacy spellings from the docs-only campaign into
  exact documentation journeys and contextual source diagnostics.
  `MemoryWatcherList` and `Task.Run` now lead to focused compiler-owned
  migration pages instead of a broad guide or generic unknown-name error;
  arbitrary ASL `stringN` widths normalize to the bounded-native-string topic.
  `settings[key]` and `settings.ContainsKey(key)` offer machine-applicable
  rewrites to `settings.enabled(key)` and `settings.contains(key)`. Keep the
  semantic distinction visible: settings lookup is a direct migration,
  `Task.Run` requires intent-specific cooperative control flow, and
  `MemoryWatcherList` depends on how it is populated. Do not publish
  placeholder migration pages whose only useful answer is that ergonomic Mono
  value/string paths are unavailable; prioritize implementing those provider
  features instead, then document their real API. Unity scene migration now
  points to the implemented typed snapshot API.

### Close feedback loops without papering over language design

- [x] Design and implement whole-expression retry before prescribing
  the Hades module-discovery recipe. Generalize the existing `retry expression`
  so its operand establishes a local failure boundary; a value block is then
  the ergonomic `retry { ... }` form rather than a separate hard-coded grammar.
  Any postfix `?` reaching that boundary ends the current attempt, suspends, and
  starts the complete operand again on the next attached tick. Ordinary success
  yields `T`, so the retry expression has type `T` and makes its containing
  function `async T`; it does not create a storable `async T` value. Give
  propagation targets a boundary identity instead of only a result type so
  code generation can distinguish function return, state-field rejection, and
  retry restart without guessing from syntax. Preserve process-lifetime
  cancellation. The operand is one synchronous attempt: reject an [`await`] or
  another [`retry`] evaluated anywhere inside it, whether it is a parenthesized
  expression, conditional, match, or value block. Calling an async function is
  still synchronous future construction and remains valid; only polling that
  value with [`await`] violates the boundary. This is the dual of [`await`]:
  `await` consumes an already-asynchronous value, while `retry` repeatedly
  evaluates synchronous fallible work and is itself the suspension point.
  Explicit `Err` and `throw` reaching the boundary both retry;
  `return`/`break`/`continue` keep their ordinary explicit lexical transfers
  out of the attempt. The accepted model is documented in the language reference
  and keyword hover, with examples for a direct fallible expression, a
  multi-step block, alternative fallible operations, and an explicit
  retry-triggering error. Add focused contrasts to the ASL, C#, JavaScript, and
  Rust guides, especially that Rust-style `?` normally targets a function while
  this construct deliberately creates a local retry boundary. Explain that
  every attempt runs within one tick and must remain bounded; intrinsically
  asynchronous discovery belongs outside the attempt behind [`await`]. A
  general value-producing [`loop`] remains the lower-level escape hatch, not
  the canonical retry transaction. Keep [`retry`] at the same prefix
  precedence as [`await`] and fallback [`else`] at its existing low
  precedence. Because adjacent `retry value else fallback` admits two
  materially different retry boundaries, emit a warning whenever neither
  interpretation is parenthesized and offer fixes for both `(retry value) else
  fallback` and `retry (value else fallback)`.
- [x] Make fallback [`else`] take one ordinary expression instead of encoding
  a private list of value/return/break/continue branches. Represent [`return`],
  [`break`], [`continue`], and [`throw`] as first-class [`Never`]-typed
  expressions throughout syntax, inference, typed HIR, async lowering,
  codegen, formatting, refactoring, and editor traversal. This lets chained
  fallible operations end in `else throw ...` and makes the same control-flow
  forms work consistently in every expression position.
- [x] Close the reported state-field propagation journey after general value
  blocks made the intended form directly expressible. A state expression can
  discover an address in local steps, use postfix [`?`] on any intermediate
  [`T!`], and finish with another fallible read; all failures target the one
  transactional field boundary, so a helper function is unnecessary. The
  generated [`state`] and [`?`] reference pages now show this exact composition,
  and their compiler-checked example prevents the older rejection from
  returning unnoticed.
- [ ] Design semantic lints from failures that compiled cleanly.
  - [x] Warn when a statically named value setting is never read by reachable
    behavior (the campaign declared `allSkullsMode` but read an unrelated
    always-false global). Direct current/old access and exact literal runtime
    keys count as reads; computed keys conservatively suppress the warning;
    headings and generated family members are excluded. The machine-applicable
    `_` suppression fix preserves the setting's host-visible key.
  - [x] Guide literal dynamic setting lookups toward their static meaning.
    `settings.enabled("key")` and `oldSettings.enabled("key")` receive a
    machine-applicable typed-member rewrite when the declared boolean has a
    source-visible name. A known `contains("key")` is diagnosed as always true
    with a semantics-preserving `true` rewrite; boolean declarations also offer
    a maybe-incorrect typed-value rewrite, while choice and file declarations
    identify their typed member without changing the surrounding boolean
    expression. Computed keys and generated family entries remain dynamic.
  - [ ] Decide whether and how to diagnose state fields that never influence
    reachable behavior. Intentional display-only state and host observation
    need explicit semantics before enabling that warning by default.
- [x] Add first-class integer ranges with explicit `start..<end` and
  `start..=end` operators and matching `T..<T` / `T..=T` type syntax. Bare
  `..` is a focused diagnostic with fixes for both endpoint policies. Runtime
  `for` evaluates stored ranges once, preserves them across suspension, treats
  reversed ascending ranges as empty, and handles an inclusive maximum without
  overflowing. Direct range loops keep scalar bounds in compiler-owned locals
  and allocate no GC range object; settings-family ranges remain a compile-time
  DSL.
  - [x] Expose immutable `.start` and `.end` fields plus source-defined
    `.contains(value)` and `.isEmpty()` methods. Generic type constructors can
    now declare fields in standard-library source, and the shared catalog
    drives inference, completion, hover, documentation, and member identity;
    only physical Wasm GC layout remains a backend concern.

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
- [ ] Design typed byte-order reads for every [`MemoryReadable`] value once a
  representative port needs more than scalar conversion. The design must
  recursively decode records and fixed arrays, compose with ordinary reads and
  state-field `at` declarations, and make mixed-endian fields auditable without
  per-tick temporary allocations. `Numeric.swapBytes()` now covers the Sonic 3
  A.I.R. scalar case; do not choose the aggregate API before a concrete target
  establishes its shape.
- [ ] Add exact record layout controls only when a target requires them:
  offsets, padding/alignment, packing, and per-field byte order. Keep
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

- [x] Separate user-facing `Display` from structural `Debug` while retaining a
  boilerplate-free default. An exact `fn Type.toString() -> String` controls
  direct display; otherwise `Display` falls back to `Debug`. Records, enums,
  arrays, fixed arrays, sets, options, results, ranges, and iterator steps
  derive a stable multiline `Debug` representation conditionally on their
  contained values. Nested strings and characters are quoted and escaped, and
  `fn Type.debugString() -> String` overrides nested formatting without `impl`
  ceremony. Capability checking and hover insight describe custom versus
  derived implementations eagerly, while code generation materializes only
  reachable concrete formatters and recursively reachable custom methods.
  Generated traversal is depth-bounded so mutable recursive container graphs
  render `<cycle>` instead of hanging an autosplitter. Equality, Display, and
  Debug share canonical aggregate metadata and deterministic backend planning.
- [x] Let catalog-defined structural method contracts be satisfied by ordinary
  user methods without `impl` ceremony. `fn Type.toString() -> String` and
  `fn Type.debugString() -> String` satisfy `Display` and `Debug`
  respectively, and implicit conversion calls retain the source method's
  reachability and effects. Standard-library implementations remain explicit
  and privileged.
- [ ] Continue designing the user-facing trait/type-class model around the
  existing source-defined capability graph. Evaluate memory reading, equality,
  numeric operations, and hashing individually; representation-sensitive
  capabilities must remain sealed unless their contracts can be implemented
  safely in ordinary source. Decide separately whether user programs ever need
  to declare their own capabilities.
- [ ] Keep trait declarations, implementations, documentation, method lookup,
  and capability inheritance in the source-defined standard-library model,
  never in a parallel checker table.
- [ ] Add a custom capability handler registry only when the first capability
  cannot be expressed by declared membership, structural equality, structural
  memory layout, or a source-defined implementation.

- [x] Introduce source-defined `Iterable` and `Iterator` capabilities and make
  `for` project its item type through those contracts. Built-in arrays, sets,
  and integer ranges now create first-class cursor values. `next` returns
  `IteratorStep<T>`, whose `Item(T)` and `End` cases keep an iterator over `T?`
  from confusing `Item(None)` with exhaustion. Array and set cursors detect
  structural mutation; direct `for` loops retain their allocation-free
  specialized lowering. Untyped helper parameters infer the minimal
  `T: Iterable` constraint, and the projected `T.Item` participates in ordinary
  bidirectional parameter, result, and capability inference.
- [x] Lower `for` over an existing `Iterator` through its ordinary `next`
  protocol, consuming that cursor, while `for` over `Iterable` retains an
  independent traversal. Lazy source-defined `map` and `filter` adapters store
  ordinary closures, compose without intermediate collections, participate in
  generic capability dispatch, and remain live across suspending loop bodies.
  Iterator cursors themselves implement `Iterable`; calling `iterator()` is an
  identity operation that preserves the current cursor position. Generic
  `T: Iterable` loops lower through `T.iterator()` and `Iterator.next`, while
  monomorphic collection and range loops retain their direct fast paths.
  Constructed callable layouts, generic standard-library record fields,
  reachability, scratch planning, and intrinsic dependencies all use the same
  demand-driven specialization path rather than adapter-specific intrinsics.
- [ ] Design user-defined mutable iterator state once user-authored associated
  types are available. Specify fallible and asynchronous iteration separately;
  do not conflate either one with the completed synchronous `IteratorStep`
  protocol.

### Engine and emulator providers

- [ ] Decide the source-defined provider refresh lifecycle before claiming
  parity for emulator cores that unload without their host process exiting.
  `state PS2` validates RetroArch's core mapping on every read and fails safely
  after unload, but attachment discovery currently reruns only after process
  detach. Compare a general provider refresh hook with lifecycle-level
  reattachment; discuss the public model before adding either one.

- [ ] Design an ergonomic, source-defined Unity Mono value-path surface for
  the repeated `mono.Make<T>` and `mono.MakeString` porting pattern before
  prescribing lower-level pointer work as the canonical migration. The public
  operation should retain the helper's useful shape: select a target-family
  layout explicitly, name static or singleton roots and managed fields, infer
  or state the final read type, compose with `state` polling, and decode managed
  strings without exposing object-header arithmetic at each call site. Compare
  methods on `MonoClass` / `MemoryPath`, a typed watcher/path value, and narrow
  state-field sugar with representative Beeny-style scalar and string reads;
  discuss the source syntax before choosing one. Intrinsics may supply metadata
  discovery, but traversal and high-level composition belong in the
  source-defined standard library. Until that design lands, diagnostics and
  migration pages must identify the ergonomic gap honestly: existing
  `staticFieldPath`, `field`, and `readManagedString` primitives can explain
  what is possible, but a porter should not be told that manually rebuilding
  every helper chain is the finished API.
- [x] Treat Unity scene snapshots as a now-proven provider gap. TUNIC,
  Anemoiapolis, Building 71, Cannibal Abduction, Chop Goblins, Assemble with
  Care, and Beeny need active/loaded scene names or indices and well-defined
  previous/current/loading semantics. `Unity.sceneManager()` now discovers the
  ASR-supported UnityPlayer layouts cooperatively in source-defined standard
  library code. `activeScene()` and `loadedScenes()` return immutable
  `UnityScene` snapshots with address, signed index, path, and name; failed
  reads retain accepted state values instead of exposing live handles or a
  partial collection. The ASL porting guide documents the direct helper
  translation rather than reproducing `asl-help` callbacks.
- [ ] Extend Unity Mono managed collections after the typed value-path design,
  using corpus-proven residual needs: add managed list/dictionary traversal for
  Alba and A Short Hike, and
  represent A Short Hike's dynamic typed tag values. Separate stable
  singleton/field chains
  already expressible through `staticFieldPath` and `field` from collection
  enumeration that genuinely needs new library/runtime support. Alba does not
  require runtime-created state fields: growable arrays can retain discovered
  task addresses, names, required values, and previous readings once typed
  managed-list traversal exists. Keep target families explicit (V1, PE32, ELF,
  Mach-O) and source-defined; do not add reflection-shaped compiler exceptions
  or silently guess offsets.
- [ ] Assess an Unreal provider only after representative `GWorld`, object, and
  name traversal ports establish the required surface.
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
- [ ] Finish first-class function values and lexical closures for iterator
  adapters, separately from legacy delegate migration. The maintained Axiom
  Verge port already established that its three callback-shaped C# delegates
  are clearer as ordinary typed functions, so callbacks alone are not evidence
  for a delegate/event model. Lazy `map` and `filter` do provide a distinct
  reason to store callable behavior. Callable type syntax, arrow closures,
  bidirectional parameter/result inference, invocation, independent closure
  bodies, typed Wasm GC function references, immutable environments, and
  shared GC cells for mutable captures are implemented. Shared cells work for
  returned and nested closures and survive async continuation frames. Untyped
  higher-order helpers infer callable parameters from invocation and specialize
  those signatures independently at each concrete call site. Async closure
  bodies now use the same typed continuation-frame and runtime-tag dispatch as
  source async functions, including captured values and mutable parameters
  that survive suspension. A closure declared in a generic helper is emitted
  independently for every concrete helper instance, including its callable
  signature, immutable environment, mutable capture cells, and async frame.
  Explicit `(parameters) -> Result => body` signatures
  constrain closure bodies, and omitted parameter and result types are shown as
  one complete virtual signature through inlay hints. Latent effects now stay
  symbolic through callable parameters, returned closures, captures, and lazy
  `map` / `filter` adapter fields; invoking or iterating a concrete value
  instantiates those effects per call site, so an unrelated effectful callback
  cannot poison a pure specialization. Function hover exposes which parameters
  are invoked or iterated. Ordinary lexical control flow is enforced: `return`
  belongs to the closure, while `break` and `continue` cannot target loops
  outside it. Named functions and closures now share the same callable type,
  typed GC representation, invocation path, and latent-effect analysis. A
  captureless adapter bridges a named function's ordinary ABI to the callable
  environment ABI without synthesizing source closures.
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
  using the capability graph's associated-type projection machinery, then make
  its declarations, documentation, completion, and lowering catalog driven
  rather than disguising the operation as a callable method.
- [ ] Add structural anonymous records only after named records prove materially
  noisy. Decide explicitly whether anonymous records are memory-readable.

## P2 — documentation and editor evolution

- [ ] After the in-editor documentation reference has proven the documentation
  graph and navigation model, add machine-readable export and rustdoc-like
  standalone HTML as additional renderers. Publishing HTML must not introduce
  a second hierarchy, link scheme, example store, or documentation source.
- [x] Add document highlights for every source-owned occurrence of the symbol
  under the cursor, with read/write classification, and type-definition
  navigation for inferred source and catalog types.
- [ ] Extend document highlights to compiler-owned standard-library and
  language symbols without resolving every token independently. Add folding
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
  source branches even when no diagnostic fired. Record both discoverability
  failures (existing facilities that porters could not find) and silent false
  successes (compiling scripts that cannot attach or disable behavior).
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
4. Reclassify blocked ports after subtracting existing features, then design
   the proven Unity managed-collection and remaining timer host surfaces; use
   the implemented scene snapshots when reevaluating Unity ports.
5. Harden and publish the bundled VSIX and native releases, then evaluate the
   hosted Code OSS workbench.
6. Resume source-debugging work only after the JavaScript debugger, native
   Wasmtime/DWARF path, and typed-IR interpreter have been compared against the
   same GC and async fixtures.
7. Keep physical `None` aggregate specialization and sandbox-sensitive host
   capabilities deferred until measurements or explicit product requirements
   justify them.
