# Himno port

[`examples/himno.split`](../examples/himno.split) ports the complete behavior
of the reviewed Himno ASL. The legacy helper starts from the static
`PlayerStats.script` singleton and follows it to the instance fields
`currDistrict` and `inRun`.

The maintained port declares that indirection as managed metadata:

```splitscript
image "Assembly-CSharp" {
    class PlayerStats {
        static PlayerStats script;
        i32 currDistrict;
        bool inRun;
    }
}

state Unity.mono(MonoVersion.V2) ["Himno.exe"] {
    district: i32 = PlayerStats.script?.currDistrict?;
    inRun: bool = PlayerStats.script?.inRun?;
}
```

The generated live reference rereads the static `PlayerStats.script` field on
every poll. State polling therefore observes a replacement managed object
without rerunning `onAttach` or caching a stale target address.

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
