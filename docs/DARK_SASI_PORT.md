# Dark SASI autosplitter port

This maintained port is based on `DarkSASI.asl` from the reviewed ASL corpus.
The campaign's compile-only candidate reduced every level-8/12 transition to a
split and omitted the route-index dispatch and delayed final split. The
repository fixture executes the restored behavior in WebAssembly for both a
normal route and a route whose first segment was skipped.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Start on the level value transition `0 -> 15` | Original `start` predicate |
| Select the expected level by LiveSplit route position | `timer.currentSplitIndex()` returning `u64?` |
| Handle a skipped earlier segment | Host-owned split index remains authoritative |
| Restart timing on every index-2 poll | Reassign `Instant.now()` until level 8 advances the timer |
| Split 52 seconds after the third route split | Monotonic `Instant.hasElapsed(Duration.fromMilliseconds(52_000))` |
| Reset pending delayed state on a new start | Assign `None` at the original start edge |
| Remove loads while the level value is zero | Original `isLoading` predicate |

`timer.currentSplitIndex()` deliberately maps every negative signed ABI value
to `None`; the script falls through without splitting in that state. It does
not mirror LiveSplit's route position in a global integer, so manual skips stay
observable without a callback or polling guess.

The monotonic clock is independent from LiveSplit's real time and game time.
That makes it the direct replacement for the source's `Stopwatch`: load removal
does not stop the 52-second delay, and wall-clock adjustments cannot move it
backwards.

## Runtime status

The deterministic host fixtures cover normal and skipped segment histories,
the absent split-index state, the first three route decisions, exact threshold
behavior one nanosecond before and at 52 seconds, repeated index-2 timestamp
restart, manual timer restart, loading pause/resume, delay cleanup, and process
detach. This source has no settings, pointer traversal, version layouts, failed
memory reads, or tick-rate changes to validate.
