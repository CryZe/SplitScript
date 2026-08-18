# ASL porting campaign audits

This audit classifies the candidates described by the external
`PORTING_FEEDBACK.md`. They were produced with compiler revision
`69f2bd9d3eb4`. The audit was performed against revision `b82638c`, the source
ASL corpus, the current compiler, and maintained in-tree ports and host
fixtures. The campaign was not rerun: its reports are evidence about the
workflow and diagnostics that the porter actually experienced.

The status terms are deliberately strict:

- **compile-only** means the candidate compiled, but no runtime fixture or live
  game established behavior;
- **behavior-limited** means review found a known omission or semantic change;
- **runtime-verified** means a maintained in-tree port has deterministic host
  coverage for the behavior named here, not that the external candidate or a
  live game was tested;
- **intentionally failing** is reserved for a focused diagnostic probe. None of
  the six campaign candidates is in that category.

## Candidate classifications

| Candidate | Campaign status | Current in-tree evidence | Review result |
| --- | --- | --- | --- |
| Arietta of Spirits | **compile-only** | **runtime-verified** core port | The candidate's extensionless Windows process identity does not satisfy the current exact-name host contract. The ASL version label has no runtime role with one physical layout, and its empty lifecycle blocks are correctly absent. |
| Aquanox | **behavior-limited** | **runtime-verified** core port | Start, split, reset, loading, optional string reads, and detach cleanup are covered. The startup dialog and timing-method read/write remain unavailable host capabilities. |
| Operation Matriarchy | **compile-only** | **runtime-verified** maintained port | The candidate has no runtime evidence and retains mutable values across attachment. The maintained port resets them in `onAttach`, validates the split filename shape, and uses the exact Windows process candidate expected by its fixture. |
| A Plague Tale: Innocence | **behavior-limited** | **runtime-verified** maintained port for Steam, Epic, Xbox, and unsupported layouts | The candidate approximates an exact timer-start event with one game-memory transition and does not make the `APT` parent setting gate its children. The maintained port observes the timer-state transition and explicitly gates every child, but exact host timer events still require runtime support. The startup dialog and timing-method mutation remain omitted. |
| Axiom Verge | **behavior-limited** | **runtime-verified** maintained Steam/vanilla scenario; other build branches are compile-covered | The candidate is a reduced demonstration, not a faithful port. It omits `RandomAV`, three of four distribution/process offsets, most settings, and exact timer events. It scans only the main module, omits the scan result's four-byte adjustment, passes a byte count where `readUtf16Le` expects code units, truncates fractional game ticks, and fails to advance checkpoint/item counters past known disabled settings. The maintained port corrects the memory, settings, and build-selection issues, approximates start resets through a timer-state transition, and registers all 119 boolean settings. Dialog/timing-method behavior and exact host timer events remain unavailable. |
| Neon White | **compile-only** | **runtime-verified** maintained port | The candidate's shadow-global transcription remains compile-only. The independently reconstructed maintained port uses direct current-snapshot replacement and covers filtering, timing, lifecycle decisions, failed reads, and reattachment with a deterministic host fixture. Its reviewed offsets still represent only one build and have not been live-game validated. |

All six candidates retain the ASL spelling of their native process name. The
current Windows host reports executable filenames including `.exe`, so those
extensionless candidates do not attach there. Portable cross-platform process
identity remains R3 host work, but documentation and audits must describe the
exact-name contract that exists today.

## Docs-only campaign at revision `87d3650`

The later docs-only campaign produced 75 compiling files but no live-runtime
evidence. Its report often classified a clean compile as `PORTED`, and several
entries declared existing facilities unavailable. This first tranche checks
five high-risk candidates against their ASL sources and the current compiler.

The findings below use these classifications:

- **undiscovered** means the language already supports the source behavior;
- **port bug** means the translation changed behavior despite sufficient
  existing facilities;
- **host gap** means preserving the behavior needs a runtime contract;
- **language or library question** means a public design decision remains and
  this audit does not choose it;
- **policy** means the difference is intentional and documented.

The existing facilities were compiled together in one probe: alternate exact
process names, named layouts selected through `process.name()`, `tickRate`,
`timer.state()`, and `process.readManagedString()` all compose in one source
file.

### First-tranche classifications

| Candidate | Campaign result | Audit result |
| --- | --- | --- |
| Arietta of Spirits | `PORTED` | The extensionless Windows provider compiles but does not attach under the current host contract. This is an **undiscovered** exact-name requirement. Omitting the single ASL version label remains a reasonable **policy** until another physical layout is supported. |
| TUNIC | `PORTED-LIMITED` | The report incorrectly says one file cannot accept both `TUNIC` and `Secret Legend`; one state candidate array supports both exact executable names. The port also manually decodes a Mono string despite `process.readManagedString()`. Both are **undiscovered**. Missing Unity scene behavior is a **language or library question**, and exact `onStart` cleanup remains a **host gap**. |
| A Proof of Concept | `PORTED-LIMITED` as two files | One state candidate array, two named layouts, `process.name()`, and a returned `StateLayout` can represent both executables in one file. The split is **undiscovered**, not a single-provider limitation. Timer run-offset mutation and exact `onStart` restoration remain **host gaps**. |
| Aim Climb | `PORTED-LIMITED` | Dropping ASL's 60 Hz `refreshRate` is **undiscovered** because `tickRate { attached: 60 }` owns this lifecycle behavior. The module-qualified `.exe` reads do not repair the extensionless provider. Dynamic lookup of a statically declared setting is valid but misses its typed member. |
| 25 To Life | `PORTED-LIMITED` | `timer.state() == TimerState.NotRunning` can preserve the source's accumulator reset, so the reported timer-state gap is **undiscovered**. Accumulating on every positive IGT rollback instead of only a positive-to-zero boundary is a **port bug**. The provider is also extensionless. |

### Behavior ledger

#### Arietta of Spirits

- Source: one process identity and one labelled physical layout.
- Candidate: the same extensionless process identity, with the label omitted.
- Correction: use and live-verify the exact Windows executable filename,
  including `.exe`. No layout selection is needed until another build exists.

This is a silent failure category: type checking says nothing about whether the
host will ever discover the declared process.

#### TUNIC

- Source: `TUNIC` and `Secret Legend` are alternate attachment identities.
- Candidate: only extensionless `TUNIC`, with a report claiming the second name
  requires another generated file.
- Correction: declare both exact executable names in one state candidate array.
- Source: UnityASL reads `LastEvent` as a managed string.
- Candidate: hard-coded Mono object offsets followed by `readUtf16Le`.
- Correction: call `process.readManagedString(address, maxUtf16Units)`.
- Remaining question: scene transitions arm starting and drive area splits, but
  the candidate removes scene state and starts on any timer-running edge. A
  Unity scene facility needs a separate design discussion.
- Remaining host gap: source `onStart` clears run-scoped event history even for
  starts outside the script's `start` block. The candidate clears it only on a
  game-state reset.

The current candidate must not be described as preserving start or area-split
behavior.

#### A Proof of Concept

- Source: one script supports two executable identities, two `LevelID`
  addresses, two route tables, and identity-specific start/reset rules.
- Candidate: two independent SplitScript files because the porter believed one
  provider could not represent both.
- Correction: use an alternate-name provider, one layout per executable,
  `process.name()` in `onAttach`, and return the matching generated
  `StateLayout` variant. Keep the distinct route mapping behind that selection.
- Remaining host gap: saving, replacing, and restoring the timer run offset
  requires timer configuration and exact `onStart` support. It must not be
  approximated through game time.

#### Aim Climb

- Source: `refreshRate = 60` and one statically declared `Split` setting.
- Candidate: polling is omitted as allegedly host-controlled, and the setting
  is queried dynamically by its host key.
- Correction: use `tickRate { attached: 60 }` and prefer the typed setting
  member. Use the exact `.exe` filename in the provider itself; having `.exe` in
  module-qualified field paths does not affect attachment discovery.

No new host facility is needed for this source behavior.

#### 25 To Life

- Source: clear cumulative game time whenever the timer is not running, and add
  old IGT only when the new IGT is exactly zero.
- Candidate: clear only on the game-level reset predicate and accumulate on any
  decrease from a positive value.
- Correction: use `timer.state()` in `whileAttached` for the timer reset, and
  preserve the narrower positive-to-zero accumulation condition unless live
  testing establishes that the ASL source itself is wrong. Verify and use the
  exact `.exe` provider identity.

This case demonstrates why adding an API based only on campaign feedback would
be harmful: the required timer-phase API already exists.

### Next audit tranche

The next tranche should cover a timer split-index source, module/file-version
selection, fixed and growable array behavior, and one genuinely unsupported
host case. It should continue to separate static compilation, deterministic
host-fixture coverage, and live-game validation.

## Cross-cutting findings

### Compiler defects resolved during the audit

The formatter could merge a generic close with a following assignment or
comparison. Revision `b82638c` fixed the parser/formatter boundary without
making whitespace semantic and added nested generic, cast, comparison, shift,
result/option postfix, and strict-equality recovery coverage.

String `+` and `+=` now produce focused guidance for template interpolation
and `String.concat` instead of ending at a numeric-capability error. No rewrite
is offered because text, separators, and evaluation grouping require author
intent.

Fallible-value diagnostics now name value fallback and exhaustive `match`
handling, and mention postfix `?` only inside an actual failure boundary. A
direct optional return using `else None` gets a narrowly-scoped
machine-applicable `else return None` rewrite. Generic failures retain their
capability requirements through inference, name the missing capability, and
list accepted concrete types when the catalog proves that set is finite.

### Supported facilities that were missed

These reports indicate discoverability or diagnostic failures, not missing
language features:

- growable `[T]` is the ordered `List<T>` replacement and supports indexing,
  iteration, search, insertion at the end, removal, and clearing;
- finite ASL dictionaries used only to declare settings should become the
  settings DSL, while `Set<T>` covers unordered run-scoped membership;
- captured helpers that do not need first-class identity become named `fn`
  declarations with explicit parameters or shared globals;
- state candidate rejection retains a previous field value without making
  `current` mutable; derived run-owned values belong in globals;
- `process.loadedModule(name)`, `process.mainModule()`, executable versions,
  and typed layout selection cover the reviewed edition probes;
- `scan`, `readRelative32`, `MemoryPath`, `onAttach`, and `retry` cover the
  reviewed scanner callback and background retry shapes without exposing
  threads.

The roadmap therefore asks the compiler and editor to lead authors to these
facilities rather than adding compatibility aliases or duplicate abstractions.

### Real compiler and documentation work

The campaign provides remaining concrete cases for:

- literal settings-key checking, completion, and nearest-key suggestions;
- lifecycle, state-mutation, module-discovery, and missing-state diagnostics
  that link directly to the canonical recipe;
- packaging and link checks so compiler-owned migration material is reachable
  in native, bundled, and editor workflows.

### Actual host or language boundaries

Exact `shutdown`, `onStart`, `onSplit`, and `onReset` behavior needs ordered,
lossless host events. Dialogs, timing-method mutation, arbitrary filesystem or
process control, and similar UI capabilities need a sandbox policy. Full
unknown-module enumeration and portable process identity remain runtime design
work. Dynamic bags, closures, and user-created background threads are not
justified by these candidates because their reviewed uses already have typed
translations; a future maintained port must prove a remaining semantic need.

## Evidence rule

The maintained host fixtures are the authoritative automated evidence for the
six in-tree ports. Candidate compilation is never sufficient for behavioral
parity, and the campaign's Neon White candidate remains distinct from the
independently reconstructed maintained port. The exact evidence and remaining
build limitations are recorded in
[`NEON_WHITE_PORT.md`](NEON_WHITE_PORT.md).
