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
- module enumeration rather than repeated guessed-name probes;
- product/file version metadata;
- a deterministic executable or module fingerprint that does not require
  hashing an entire image inside one guest update;
- typed, bounded iteration over mapped memory ranges, including readable,
  writable, executable, and path-backed flags.

Any collection-shaped result should use the GC-managed resource direction from
R1 or a bounded visitor/polling contract. It should not introduce another
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

Dynamic settings mutation must compose with concurrent changes from the
LiveSplit UI. R1's GC-native map API therefore needs to retain the current
compare-and-store behavior rather than replacing it with an unconditional
overwrite.

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
