# Operation Matriarchy port

[`examples/operation_matriarchy.split`](../examples/operation_matriarchy.split)
ports the complete behavior of the reviewed legacy ASL. The original watches a
12-byte `Game.dll` resource name such as `01_02_1.dds`, combines the first two
underscore-separated components into a level ID, and interprets the third as
the active/loading marker.

The maintained port maps those semantics directly:

| Legacy ASL | SplitScript |
| --- | --- |
| `state("GAME")` | `state "GAME.exe"` because the current Wasm runtime attaches by the complete executable name |
| `string12` | `String at ... as utf8(12)` |
| `init` variables | inferred globals reset in `onAttach` |
| `String.Split` | fallible exact `String.split`, with ordinary array indexing |
| `update` bookkeeping | `whileAttached` updates the persistent parsed level and active marker |
| `start` / `reset` / `split` / `isLoading` | the corresponding typed lifecycle blocks |

`String.split` preserves empty segments, matching the relevant .NET behavior,
and rejects only an empty delimiter or an unrepresentable result. The port also
guards malformed names with fewer than three underscore components; it retains
the last accepted parsed state instead of allowing an index exception to stop
the autosplitter.

The host fixture models the exact executable and module names, the direct
12-byte read, the initial inactive snapshot, activation of `01_01`, transition
to `01_02`, reset on inactive `01_01`, load removal, and process detachment. It
runs as part of the repository verification matrix:

```console
cargo xtask check
```
