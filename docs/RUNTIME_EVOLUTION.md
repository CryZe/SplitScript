# LiveSplit host-runtime evolution

This document records semantics that SplitScript needs from the LiveSplit
autosplitting runtime but that the current WebAssembly host contract cannot
express cleanly. It is the runtime-facing requirement ledger: language and
compiler work stays in [`TODO.md`](../TODO.md), while the implemented import
contract stays in [`ABI.md`](ABI.md).

Requirements discovered while porting ASL scripts belong here when satisfying
them requires new or changed host behavior. A compiler workaround should not
silently become the permanent design when the missing fact or lifetime is owned
by LiveSplit.

## Design rules

- Preserve the existing `env` ABI for existing Wasm autosplitters. Add new
  facilities through a versioned, feature-detectable contract rather than
  changing an existing signature in place.
- Describe semantic guarantees before choosing import names or raw encodings.
- Prefer opaque, unforgeable host references over guest-visible integer handles.
- Do not expose manual allocation, copying, or `free` operations in the
  SplitScript language when the runtime can own that lifetime safely.
- Keep deterministic cleanup for resources whose prompt release is observable.
  Garbage collection is a safety and ownership mechanism, not a scheduling
  guarantee.
- Preserve the runtime's responsiveness contract. APIs that enumerate
  processes, modules, mappings, or scan memory need bounded or suspending forms
  so one `update` cannot monopolize the autosplitting thread.
- Every promoted facility needs a runtime conformance fixture covering success,
  absence/failure, ownership, ordering, and cancellation.

## R1: GC-managed host resources

**Priority:** P0

**Status:** Proposed runtime direction; import namespace and exact signatures
are intentionally undecided.

### Problem

The current ABI represents `SettingsMap`, `SettingsList`, `SettingValue`, and
attached processes as nonzero integer handles. Most settings operations return
owned handles that the guest must copy and free explicitly. That is appropriate
for a C-shaped ABI, but it is a poor source-language contract: WebAssembly GC
structs do not provide user-defined finalizers, so the compiler cannot attach a
reliable `settings_map_free` call to an arbitrary GC object's lifetime.

Missed frees would leak host resources. Eager frees would be unsafe when values
are aliased, stored in globals, captured by an async continuation, or returned
from functions. Requiring script authors to manage these lifetimes would also
undermine the scripting-language ergonomics.

### Direction

Add a parallel, versioned host ABI whose resource-bearing values use nullable
`externref` rather than guest-forgeable integers. Wasmtime describes `externref`
as an [opaque, GC-managed reference to host data](https://docs.rs/wasmtime/latest/wasmtime/struct.ExternRef.html).
The guest can retain and pass the value but cannot inspect or forge its host
representation.

The first candidates are:

- settings maps, lists, and setting values;
- immutable snapshots returned by loading the global settings map;
- copies or persistent updates derived from those snapshots;
- optional manually attached process resources, if SplitScript later permits
  attachment outside its compiler-managed `state` lifetime.

The high-level SplitScript types remain nominal and typed even though all host
objects share the Wasm `externref` representation. Each import must validate the
expected host kind. A null reference represents absence or failure only where
the declared SplitScript type is optional or fallible.

The GC-native settings surface should preserve the full semantics of the
existing ABI:

- recursive maps, lists, booleans, `i64`, `f64`, and strings;
- value and collection copies that do not alias mutable state unexpectedly;
- insertion and indexed access without guest-visible ownership transfer;
- atomic `storeIfUnchanged(old, new)` behavior so UI changes are not lost;
- no source-visible `copy` merely for lifetime management and no `free` at all.

Prefer immutable snapshots and persistent-style update operations unless a
mutable design can specify aliasing and concurrent frontend changes just as
clearly. UTF-8 strings may continue to cross linear memory initially; replacing
every byte-buffer API is not required to gain safe resource ownership.

### Collection and deterministic cleanup

An `externref` allows Wasmtime to reclaim its host data after neither Wasm nor
the embedder roots it. This still requires the LiveSplit runtime to scope its
own roots correctly and to allow or trigger collection. Conformance tests must
prove that repeatedly loading and discarding settings does not retain every
historical object.

Collection timing is nondeterministic. Settings values can normally tolerate
that. An attached process owns operating-system resources and should still be
released deterministically by SplitScript's existing process-lifetime region.
If arbitrary attachment is added later, its design needs a scoped lifetime or
an explicit idempotent `close`; an unreachable `externref` may provide fallback
cleanup but must not be the only way to detach promptly.

Host objects should not form ownership cycles unless the runtime also provides
a way to collect those cycles. The design must document behavior at store
shutdown, hot reload, trapped updates, and cancelled async work.

### Acceptance criteria

- A script can retain and compose settings maps, lists, and values without any
  explicit lifetime operation.
- Dropping all guest and embedder references eventually releases the host data.
- Wrong-kind references produce a controlled host error rather than a trap or
  unsafe downcast.
- Legacy `env` modules continue to run unchanged.
- The compiler can select the GC-native ABI by an explicit supported-version or
  feature contract, not by guessing from a failed instantiation.

## R2: exact timer event delivery

**Priority:** P1

**Status:** Required semantics known; transport undecided.

The current runtime calls only the Wasm `update` export. Polling
`timer_current_split_index` observes ordinary advancement, but it cannot
distinguish an undo followed by another split between two updates. It also
cannot provide exact `onStart`, `onSplit`, and `onReset` events while detached.
The maintained Axiom Verge port can clear its event cursors after an attached
`NotRunning` transition, but cannot reproduce the legacy `OnStart` callback
when the timer starts before process attachment. Its diagnostic-only `OnSplit`
callback is intentionally omitted rather than approximated. Circuit Superstars
likewise rearms accumulated game time in `onStart`; moving that reset into its
game-driven `start` predicate would miss externally initiated starts.

The runtime should eventually expose an ordered, lossless timer-event contract.
This could be callback exports or a sequenced event queue consumed during
`update`; the semantic requirements matter more than that choice:

- starts, splits, skips, undos, resets, and externally initiated actions are
  observable exactly once and in order;
- events are delivered even when no process or emulator is attached;
- the split event specifies its segment identity and whether it is observed
  before or after advancement;
- ordering relative to process attachment, detachment, snapshot refresh, and
  game-time updates is explicit;
- reentrancy and suspension behavior are defined;
- a sequence number makes missed or duplicated delivery testable.

SplitScript can build optional `onStart`, `onSplit`, and `onReset` blocks only
after this host contract exists. It must not invent exports that LiveSplit does
not call.

## R3: typed process and module discovery

**Priority:** P1, promoted by port evidence

**Status:** Individual requirements are known; no combined host proposal yet.

Ports need more stable process identity than an executable name in several
cases. Candidate host-owned facilities are:

- bounded process enumeration with stable process IDs and start ordering;
- module enumeration for scripts that genuinely inspect unknown names. Known
  required names use the waiting `process.module(name)` API, while known
  optional names now use synchronous `process.loadedModule(name)` over the
  existing address/size imports;
- product/file version metadata;
- a deterministic executable or module fingerprint that does not require
  hashing an entire image inside one guest update. COTM 1.1.0/1.1.2 and COTM2
  1.2.2/1.3.1 are concrete evidence: their ASL sources select same-name layouts
  by known whole-file MD5 values, and COTM documents identical module sizes;
- path/name metadata for mapped memory ranges. SplitScript now snapshots the
  existing count/index ABI synchronously into GC-owned `MemoryRange` values
  with readable, writable, and executable flags; enumerating this cheap host
  metadata does not need the cooperative scheduling required by memory scans.

Before expanding discovery, settle and document the attachment-name contract:
whether matching uses the full executable filename, how case is handled, and
what name is reported after a match on each host OS. Legacy ASL commonly omits
Windows' `.exe`, while extensionless names are normal on Linux and macOS. Do
not add a compiler warning until deciding whether the host should normalize
executable identity portably, accept target-specific aliases, or retain exact
literal names. Whichever policy is chosen needs stable attachment fixtures on
all three hosts rather than accidental platform behavior.

Any collection-shaped result should use the GC-managed resource direction from
R1 or, when the operation itself is expensive, a bounded visitor/polling
contract. It should not introduce another
integer handle plus manual `free` family. Long-running discovery must cooperate
with the runtime's hanging-autosplitter threshold and process cancellation.

## R4: settings presentation and transactional mutation

**Priority:** P1, promoted by port evidence

**Status:** Basic declarations are supported; richer frontend semantics remain
to be proven by ports.

The existing host supports booleans, headings, choices, file selection,
filters, tooltips, and a global settings map. Future ports may require
conditional visibility or enablement and repeated/table-shaped settings. Those
features need a frontend/runtime contract, not merely new SplitScript syntax.
The maintained Axiom Verge port provides concrete semantics: its category
checkboxes recursively gate child values in legacy ASL. SplitScript preserves
that behavior explicitly in source, but the current host cannot present those
checkboxes as the visual and interactive parents of nested controls.

The bulk ASL campaign found finite loops that register 35, 50, or 110 boolean
settings and then look them up by string key. A compile-time repeated/table
declaration can solve those finite cases without runtime mutation and belongs
in `TODO.md`. Runtime-created settings are justified only when keys or labels
are genuinely discovered from game data or otherwise unbounded. For that case,
the host must define when additions become visible, how ordering and hierarchy
are preserved, what happens to persisted values for temporarily absent keys,
and whether registration is permitted after startup.

Dynamic settings mutation must compose with concurrent changes from the
LiveSplit UI. R1's GC-native map API therefore needs to retain the current
compare-and-store behavior rather than replacing it with an unconditional
overwrite.

## R5: typed timer/run observation and controlled mutation

**Priority:** P1, promoted by repeated port evidence

**Status:** Existing narrow operations are supported; broader semantics and
transport are undecided.

The current contract exposes timer state, current split index, whether a
segment was split, timer actions, game-time publication/pause/resume, and
variables. This already covers ordinary `start`/`split`/`reset`, load removal,
and many one-shot guards. It does not expose the nested LiveSplit objects that
legacy scripts use through `timer.CurrentTime`, `timer.CurrentSplit.Name`,
`timer.Run.Offset`, timing-method selection, or attempt/category metadata.

The porting campaign found three different needs that must not be collapsed
into one untyped `timer` escape hatch:

- process-independent debounce and delay logic, which should use SplitScript's
  existing monotonic `Instant.now()` and requires no new timer ABI;
- read-only snapshots of timer real time, game time, current segment identity,
  run metadata, and timing method;
- controlled mutations such as changing run offset or timing method, whose
  ordering and user-visible effects need an explicit host contract.

A future versioned ABI should expose typed optional values when LiveSplit has
no current attempt, split, or game time. Reads taken during one update must form
a coherent snapshot. Mutations must specify whether they apply before or after
timer-decision exports, how they interact with reset/start/split events, and
whether user UI changes win. Run-offset units, sign, precision, persistence,
and undo/reset behavior must be explicit. Source APIs should be
least-privilege: reading metadata must not grant arbitrary run mutation.

Timing-method prompts and message boxes are frontend interactions, not timer
state. They require a separate consent-aware UI design and may remain outside
the sandbox; a port must not silently claim parity merely because it omitted
the prompt.

Acceptance tests should cover absence before a run, active and paused timers,
segment changes, reset, positive and negative offsets, timing-method changes,
and ordering relative to `whileAttached`, timer-decision actions, game-time
publication, and the eventual exact-event contract in R2.

## R6: script shutdown notification

**Priority:** P1, promoted by lifecycle-port evidence

**Status:** Required semantic boundary known; no host export exists.

ASL `shutdown` runs when the autosplitter is disabled, LiveSplit exits, the
script path changes, or the script is reloaded. It is script-instance teardown,
not process detachment. SplitScript can detect an attached process closing
inside `update`, but generated Wasm cannot run code after the host simply drops
the store or module instance.

If maintained ports prove teardown side effects necessary, the host should
invoke a dedicated export before discarding a SplitScript instance. The
contract must define ordering relative to process-exit handling, settings and
timer events, debug-watch replacement, traps, cancellation of suspended work,
and host shutdown. It must also state whether teardown has a bounded execution
budget and whether it may suspend; prompt resource cleanup should normally be
compiler/runtime-owned rather than left to user code.

Do not map ASL `shutdown` to `onDetach`: the latter runs after each process
closes, while shutdown may run once with no current process
and may observe only partially available historical process state.

## R7: affine resource values and transitive deterministic drop

**Priority:** P2 / deliberately deferred

**Status:** Design direction worth preserving; no implementation commitment
until a maintained port or public host API requires user-owned resources.

R1 records a host-GC direction for settings values whose cleanup need not be
prompt. A complementary solution may be needed for files, manually attached
processes, or other resources whose release is observable and must be
deterministic. Core Wasm GC objects have no user-defined finalizer, so merely
placing an owned host handle inside a GC record cannot arrange a reliable close
when that record becomes unreachable.

SplitScript could instead add an affine `resource` category. A resource leaf
would be an opaque host handle with trusted destruction behavior. Any type that
transitively contains a resource would derive an internal `NeedsDrop` property
and would not be `Copy`, much like a Rust aggregate containing a non-`Copy`
field:

```splitscript
resource File

record Request {
    file: File,
    path: String,
}
```

`Request`, `Request?`, `Request!`, and `[Request]` would all require
deterministic drop. Assignment would move ownership rather than create an
untracked GC alias, while ordinary method calls and non-consuming parameters
could borrow implicitly. The compiler would generate structural drop glue for
records, active enum variants, options, results, and initialized collection
elements. It would invoke that glue on normal scope exit, overwrite, `return`,
`throw`, `break`, `continue`, and async completion or cancellation. Returning
or otherwise transferring a value would suppress the source owner's drop.

The Wasm GC allocation could remain alive until a later collection; only the
external resource is released by deterministic drop. This does not require
solving general GC reachability because the type system maintains one tracked
owner. It does require resource-bearing aggregates to lose ordinary freely
aliased GC-reference semantics. A resource-bearing value must not be hidden
inside another freely copied GC object: the non-`Copy`/`NeedsDrop` property has
to propagate through every owning edge.

An initial design should avoid partial field moves, arbitrary resource-bearing
GC graphs, and implicit shared ownership. Generated async frames may own moved
resources only when every completion and cancellation path runs their drop
glue. The host should additionally retain an instance- or attachment-scoped
resource table that forcibly closes remaining entries after a trap, cancelled
instance, or failed teardown; compiler-generated normal-flow cleanup cannot be
the sole leak barrier.

Not every nominal host value should be owned. The process supplied by a normal
`state` declaration is naturally borrowed from the attachment lifetime, and
the ordinary settings view can remain host-owned. Explicit file opening,
manual attachment, or a settings builder consumed by registration are better
candidates for owned resource values.

Before implementing this direction, settle:

- how moves and implicit borrows appear in diagnostics, function signatures,
  returns, globals, and editor type information;
- whether `T?`/`T!` receive specialized resource drop glue before user
  records and variable-length collections;
- how current aliasable arrays, sets, records, and future closures are
  restricted when they transitively contain a resource;
- the exact non-suspending, non-failing destructor ABI and its behavior during
  traps, hot reload, and store destruction;
- whether a later explicit shared-resource wrapper is justified, rather than
  making reference counting and its cycle rules part of every resource; and
- how closely the ABI should mirror Component Model `own`/`borrow` resources,
  even if SplitScript continues to emit a Core Wasm GC module.

This remains lower priority than APIs needed by maintained autosplitters. Its
purpose is to preserve a plausible deterministic ownership model so the lack
of Wasm GC finalizers does not force source-visible `free` calls or premature
commitment to nondeterministic cleanup.

## R8: total and bounded tick-rate scheduling

**Priority:** P1 / runtime correctness

**Status:** The frequency unit and transition timing are proven; invalid-value
and scheduler bounds need a host fix.

`runtime_set_tick_rate` receives updates per second and stores its reciprocal
as the wait duration used after the current update. The selected interval is
persistent, including across process detachments. SplitScript therefore owns
its policy explicitly through `setup`, `onDetach`, and `onAttach`; the host must
not invent a process-lifecycle reset.

The current host rejects non-positive finite inputs but does not reject every
non-finite value. `NaN` can reach the stored interval and fail later when it is
converted to a duration; positive infinity produces a zero interval and can
turn the runner into a busy loop. Extremely small or large positive values
also need an explicit scheduling policy rather than relying on floating-point
reciprocal edge cases.

The existing import signature can retain its `f64` parameter, but the host
implementation should:

- reject `NaN`, both infinities, zero, and negative values before publishing a
  new interval;
- define finite minimum and maximum frequencies that keep the resulting wait
  representable and prevent an accidental busy loop or impractically long
  sleep;
- leave the previous valid interval unchanged when validation fails;
- guarantee that reading the current interval is total and cannot panic; and
- test the initial 120 Hz interval, 60/100/120 Hz updates, invalid inputs, and
  persistence across ordinary process detachments.

Until that host validation is tightened, SplitScript documents a positive,
finite source contract and emits ordinary calls without duplicating scheduler
policy in generated Wasm.

## R9: safe observation of instrumented loading events

**Priority:** P2 / blocked-port evidence

**Status:** Semantic requirement recorded; no raw process-mutation API is
proposed.

The Amnesia: A Machine for Pigs and Amnesia: The Dark Descent ASL scripts
derive loading state by modifying the game process. They scan executable code,
allocate an executable buffer, copy displaced instructions, install jumps that
write a synthetic flag, suspend and resume the process around modification,
and restore all bytes and allocations during shutdown. Ordinary signatures,
reads, layouts, and lifecycle state cover the rest of these scripts but cannot
observe the transient events captured by that instrumentation.

A future host solution must provide the loading information without allowing an
autosplitter instance to leave the game patched. Its semantic requirements are:

- event observation must preserve the load-begin and load-end transitions that
  the injected flag currently records, rather than sampling an unrelated
  approximation;
- ownership must remain attachment-scoped, with deterministic cancellation and
  cleanup on normal detach, rejected attachment, trap, hot reload, and host
  shutdown;
- setup and teardown must not block ordinary autosplitter ticks for an
  unbounded duration;
- partial setup must roll back safely, and cleanup must be idempotent even when
  the game closes during instrumentation;
- the runtime must define which executable versions and architectures are
  supported instead of accepting unchecked script-provided machine code; and
- conformance coverage must prove transition ordering, failed setup, process
  closure at every stage, trap cleanup, reattachment, and byte-for-byte process
  restoration if code patching remains part of the host implementation.

No temporary SplitScript workaround preserves the synthetic loading signal.
The scripts can port their readable fields and other lifecycle decisions, but
load removal remains blocked. Exposing arbitrary allocation, memory writes,
process suspension, or jump installation directly to untrusted scripts would
expand the security and lifecycle surface substantially; a native game-specific
loading signal or tightly constrained host-owned instrumentation contract must
be evaluated before any ABI spelling is chosen.

## Recording requirements from ports

When an ASL port exposes a possible runtime gap, add it here before designing a
compiler-specific escape hatch. Record:

1. the game and exact source behavior that provides the evidence;
2. why the current host ABI cannot represent it faithfully;
3. the required semantic guarantee, independent of proposed function names;
4. ownership, lifetime, cancellation, ordering, and responsiveness constraints;
5. the temporary SplitScript workaround, if one exists;
6. whether the issue blocks the port, reduces fidelity, or only affects
   ergonomics;
7. the conformance test that would prove a future runtime implementation.

If the gap can be solved entirely by ordinary SplitScript source or its
standard library, it belongs in `TODO.md` instead. If it changes the generated
module's implemented contract, update `ABI.md` only when that contract actually
exists.
