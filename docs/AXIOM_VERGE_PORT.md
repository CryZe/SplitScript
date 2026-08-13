# Axiom Verge port

[`examples/axiom_verge.split`](../examples/axiom_verge.split) is a maintained
port of the legacy `AxiomVerge.asl`. It pressure-tests process-wide discovery,
known optional modules, dynamic UTF-16 identifiers, reusable 32-bit pointer
paths, dynamic setting lookup, and legacy parent-setting semantics.

## Conformance record

| Field | Status |
| --- | --- |
| Source | Legacy `AxiomVerge.asl` from the reviewed ASL corpus |
| Declared targets | Vanilla `AxiomVerge.exe` and randomizer `RandomAV.exe`, with Steam and Epic offsets |
| Runtime fixture | Vanilla Steam layout in `tests/axiom_verge_runtime.mjs` |
| Compiler provenance | The port and fixture are compiled from the same repository revision by `cargo xtask check` |
| Classification | Runtime-verified for the fixture; variant-limited and behavior-limited as described below |

The port preserves all 119 legacy boolean setting keys, labels, defaults, and
tooltips. This includes the eight category-parent settings. Legacy ASL reads a
child recursively through every parent, so `eventEnabled` applies the same
gating instead of treating category controls as decorative headings.
`settings.contains` remains separate: a disabled but declared event advances
its checkpoint cursor exactly as in the original script.

The three C# delegates are ordinary typed functions. They construct the same
32-bit deep-pointer paths, validate key-point lengths, and decode the game's
native UTF-16LE identifiers through bounded process reads. The platform matrix
selects the original vanilla/randomizer and Steam/Epic root displacement after
one cooperative process-wide signature scan. A synchronous
`process.loadedModule("steam_api.dll")` probe distinguishes the known optional
Steam module without delaying attachment.

The deterministic host fixture verifies:

- exact process attachment, Steam detection, signature discovery, and the
  vanilla Steam root displacement;
- all 119 registered setting keys, including generated note settings and
  legacy parent keys;
- one bounded process-wide scan read and every 32-bit pointer traversal used
  by the state and dynamic identifier readers;
- disabled child settings and disabled parent settings advancing the cursor
  without splitting, followed by an enabled checkpoint split;
- an enabled item split, the `FirstDeath` reset option, and sixty-hertz game
  time conversion.

## Deliberate limitations

The current settings host can display headings and independent controls, but
cannot make a boolean control visually own nested children. The port therefore
shows an explicit category checkbox inside each category while preserving the
legacy recursive behavior in source. Conditional settings hierarchy remains a
host-design item rather than compiler-only syntax.

The original `OnStart` callback clears event cursors even when no process is
attached. SplitScript currently observes the equivalent
`TimerState.NotRunning` transition only while attached. Exact lossless timer
events require the host lifecycle contract tracked in
[`RUNTIME_EVOLUTION.md`](RUNTIME_EVOLUTION.md). Its `OnSplit` callback only
printed a diagnostic and is omitted rather than approximated with polling.

The legacy scanner tried ten times with half-second sleeps and then left an
unsupported build running with invalid watcher roots. The port instead awaits
cooperative discovery until the process closes. This avoids blocking an update
and never begins state polling with an invalid layout, but deliberately has no
five-second unsupported-build message. A general discovery deadline should be
designed once a maintained port needs an actionable timeout result.

Only the vanilla Steam memory matrix is host-executed. The other three original
offset choices are present and type checked, but remain variant-limited until
fixtures or real-game validation cover them.
