# A Hat in Time port

[`examples/a_hat_in_time.split`](../examples/a_hat_in_time.split) is the
production-scale port of the original `AHatInTime.asl`. It is also a
conformance example for cooperative process-wide discovery, large hierarchical
settings, persistent memory state, and timing-sensitive split behavior.

## Runtime mapping

The original background scan thread is represented by suspending discovery in
`onAttach`. Process-wide signature scans inspect one bounded window during a
poll and yield before either continuing or delivering their result. State
polling starts only after the timer, save-data, actor, and coordinate roots are
ready.

The port keeps patch-dependent save-data layouts explicit. Structs and static
arrays replace the original dynamic dictionaries for position triggers, pause
triggers, rift identification, and hub-return detection. Its settings tree
registers the same 214 runtime keys as the original script, and helper
functions preserve the recursive parent-setting behavior of legacy ASL.

The split lock observes the optional `u64` returned by
`timer.currentSplitIndex()` from `whileAttached`. The standard library maps the
host ABI's negative sentinel to `None` rather than exposing the signed encoding.
LiveSplit's current Wasm autosplitting contract calls only the `update` export;
it does not provide an `on_split` callback. An index advancement therefore
restarts the lock on the next update. This faithfully handles ordinary and
external splits, including skipped indices, but an undo followed by a split
between the same two polls cannot be observed when the resulting index is
unchanged. Exact event observation remains future host-runtime work rather than
an extra export that LiveSplit never calls.

The runtime fixture validates:

- deterministic alternative-signature layout selection and every discovered
  pointer chain;
- bounded cooperative scan work, including no more than one scan-sized read per
  update;
- exact legacy setting-key parity;
- new-file and any-file starts, resets, simple and detailed time-piece splits;
- external split-index lock restart and delayed lock reopening;
- position and rift-entry splits;
- precise game-time correction and IL-mode game time.

## Deliberate host limitations

One startup-only behavior cannot yet be expressed by the sandboxed host API:

- The original executable patch label can now be reproduced with the bounded
  cooperative [`Module.md5()`](ASL_PORTING.md#attached-process-identity)
  fingerprint. It remains optional because the selected memory layout does not
  depend on that label.
- The original optionally opens a modal prompt that changes LiveSplit's timing
  method. The setting remains registered for migration fidelity, but changing
  timing method needs a typed timer/run API and an explicit host contract.

Neither limitation changes the script's memory discovery, start, split, reset,
loading, or game-time calculations.
