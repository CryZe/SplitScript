# ARTIFICIAL port

[`examples/artificial.split`](../examples/artificial.split) ports the complete
behavior of the reviewed ARTIFICIAL ASL. The legacy script delegates Unity
metadata discovery to `asl-help`, then watches three static fields on
`AutoSplitterData`: `inGameTime`, `levelID`, and `isRunning`.

The maintained port maps those semantics directly:

| Legacy ASL | SplitScript |
| --- | --- |
| `state("ARTIFICIAL")` | `state "ARTIFICIAL.exe"` because the current Wasm runtime attaches by complete executable name |
| `vars.Helper.TryLoad` Mono callback | ordinary awaited discovery in `onAttach` |
| `mono.Make<T>("AutoSplitterData", field)` | `MonoClass.staticField(field)` followed by typed state reads |
| `TimeSpan.FromSeconds` | `Duration.fromSeconds` |
| `start` / `split` / `reset` / `gameTime` / `isLoading` | the corresponding typed lifecycle blocks |

The provider is source-defined in `stdlib/standard.split`. It resolves
`mono_assembly_foreach` from `mono-2.0-bdwgc.dll`, derives the assembly-list
global from the exported function, finds `Assembly-CSharp`, traverses Mono's
class cache and field metadata, and resolves the static-data table. The port
selects `MonoVersion.V2` explicitly; automatic layout detection would obscure
a target-memory assumption that should remain auditable.

The deterministic fixture constructs a minimal PE32+ export table and a sparse
Mono V2 metadata graph. It proves that the real compiled Wasm discovers all
three static fields, starts when `isRunning` rises, splits when `levelID`
increases during a run, resets when `isRunning` falls, forwards fractional game
time, and pauses game time through `isLoading`. It runs with the full repository
verification matrix:

```console
cargo xtask check
```

The fixture proves the V2 PE64 static-field path used by this port. It does not
claim coverage for V3, older V1 layouts, 32-bit Mono, ELF/Mach-O modules,
instance fields, or managed object/string traversal; those remain explicit
follow-up families driven by maintained ports.
