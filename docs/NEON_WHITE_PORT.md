# Neon White port

[`examples/neon_white.split`](../examples/neon_white.split) is a maintained
port reconstructed from the original `neonwhite.asl`. It does not promote the
external campaign candidate by inspection. The source uses direct
`current.field` replacement to express the legacy `update` block's filtered
snapshot, while `old` remains the preceding accepted snapshot.

The preserved behavior is:

| Legacy behavior | Maintained SplitScript behavior |
| --- | --- |
| A one-frame empty `levelId` must not split | `whileAttached` replaces it with `old.levelId` before timer decisions |
| A loading-frame zero `levelRushMicroseconds` must not move game time backwards | The zero is replaced with the preceding rush time except on a first Level Rush scene |
| Rush total advances before the current playthrough resets | The current playthrough is temporarily replaced with zero so the same level is not counted twice |
| A new playthrough is represented by `-1` microsecond | Current-level timing is included again on that marker |
| A changed non-first level ID splits | The `split` block compares the filtered current and preceding IDs |
| Returning to `nu.unity` splits | The scene transition is an independent split condition |
| Entering a first Level Rush scene starts or resets | The same scene predicate is used by `start` and `reset`; timer state determines which action is eligible |
| Real time is discarded | `isLoading` remains unconditionally true, and `gameTime` supplies rush plus accepted playthrough time |

The deterministic host fixture verifies initial equal-snapshot seeding,
transient empty IDs, transient zero rush time, both sides of the
include-current-level transition, both split conditions, start and reset,
millisecond game-time conversion, independent field advancement after one
required read fails, process closure, and clean snapshot seeding after
reattachment. It runs through `cargo xtask check`.

This is runtime-verified against the simulated host observations above, not a
claim of live-game validation. The four pointer paths and `UnityPlayer.dll`
offsets cover only the build represented by the reviewed ASL. The current Wasm
runtime also matches exact process candidates, so the maintained Windows port
uses `Neon White.exe` rather than legacy ASL's extensionless `Neon White`.
Portable process identity remains runtime-design work. There are no settings
or alternate layouts in the source ASL.
