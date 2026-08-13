# Himno port

[`examples/himno.split`](../examples/himno.split) ports the complete behavior
of the reviewed Himno ASL. The legacy helper starts from the static
`PlayerStats.script` singleton and follows it to the instance fields
`currDistrict` and `inRun`.

The maintained port keeps that indirection explicit and reusable:

```splitscript
let playerStats = await image.class("PlayerStats")
let script = await playerStats.staticFieldPath("script")
let districtOffset = await playerStats.field("currDistrict")
districtPath = script.dereference(districtOffset as i64)
```

`MonoClass.staticFieldPath` describes the static field without resolving it
once during attachment. `MemoryPath.dereference` moves the old final offset
into the path's pointer-read sequence and applies a new final offset. State
polling therefore observes a replacement `PlayerStats.script` object without
rerunning `onAttach` or caching a stale target address.

The deterministic fixture constructs the same PE64 Mono V2 metadata graph used
by the ARTIFICIAL conformance test, then replaces the static singleton between
polls. It proves start on entering district 1, split on district 11 to 12, and
reset on returning to a non-running district 1 through three distinct managed
objects. It runs with the repository verification matrix:

```console
cargo xtask check
```

The reviewed legacy helper auto-detects its Mono layout. This first maintained
translation selects `MonoVersion.V2` as the explicit target contract exercised
by the fixture. The fixture proves the PE64 V2 memory contract and lifecycle
behavior; the selected layout still needs validation against a real installed
game build and does not claim binary coverage for other builds or platforms.
