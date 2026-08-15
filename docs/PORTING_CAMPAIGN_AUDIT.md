# Six-script porting campaign audit

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
| Arietta of Spirits | **compile-only** | **runtime-verified** core port | No known behavioral omission. The ASL version label has no runtime role with one physical layout, and its empty lifecycle blocks are correctly absent. |
| Aquanox | **behavior-limited** | **runtime-verified** core port | Start, split, reset, loading, optional string reads, and detach cleanup are covered. The startup dialog and timing-method read/write remain unavailable host capabilities. |
| Operation Matriarchy | **compile-only** | **runtime-verified** maintained port | The candidate has no runtime evidence and retains mutable values across attachment. The maintained port resets them in `onAttach`, validates the split filename shape, and uses the exact Windows process candidate expected by its fixture. |
| A Plague Tale: Innocence | **behavior-limited** | **runtime-verified** maintained port for Steam, Epic, Xbox, and unsupported layouts | The candidate approximates an exact timer-start event with one game-memory transition and does not make the `APT` parent setting gate its children. The maintained port observes the timer-state transition and explicitly gates every child, but exact host timer events still require runtime support. The startup dialog and timing-method mutation remain omitted. |
| Axiom Verge | **behavior-limited** | **runtime-verified** maintained Steam/vanilla scenario; other build branches are compile-covered | The candidate is a reduced demonstration, not a faithful port. It omits `RandomAV`, three of four distribution/process offsets, most settings, and exact timer events. It scans only the main module, omits the scan result's four-byte adjustment, passes a byte count where `readUtf16Le` expects code units, truncates fractional game ticks, and fails to advance checkpoint/item counters past known disabled settings. The maintained port corrects the memory, settings, and build-selection issues, approximates start resets through a timer-state transition, and registers all 119 boolean settings. Dialog/timing-method behavior and exact host timer events remain unavailable. |
| Neon White | **compile-only** | No maintained in-tree runtime fixture | The mutable effective-state transcription is plausible and preserves the ASL update order, but it has not been executed against a deterministic snapshot sequence or a live game. It must not be cited as proof that the snapshot rewrite is faithful. |

All six candidates retain the ASL spelling of their native process name. That
is not currently diagnosed because the portable attachment contract is
unsettled: Windows commonly needs a `.exe` filename while extensionless names
are normal on Linux and macOS. This remains R3 host work, not evidence that the
candidate process identities are correct.

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
five in-tree ports. The Neon White candidate should be promoted only with a
fixture covering first-snapshot seeding, transient empty level IDs, suppressed
zero rush time, include/exclude transitions, scene-based start/reset, splits,
game time, failed reads, detach, and reattach. Compilation remains necessary
but is never sufficient for behavioral parity.
