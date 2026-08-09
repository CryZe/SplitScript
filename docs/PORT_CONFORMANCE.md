# Port conformance fixtures

Compiling an autosplitter proves its syntax, types, effects, and generated
WebAssembly, but not its behavior against a game. A maintained port needs a
deterministic host fixture that drives the generated `update` export through
the important process, settings, memory, timer, and lifecycle transitions.

The shared Node harness lives in
[`tests/support/splitscript_host.mjs`](../tests/support/splitscript_host.mjs).
It implements the current SplitScript host imports and records every observable
timer/runtime operation. Fixtures stay focused on game behavior instead of
copying dozens of empty ABI functions.

## Minimal fixture

```js
import { SplitScriptHost } from "./support/splitscript_host.mjs";

const host = await SplitScriptHost.instantiate(process.argv[2], {
    settings: { splitBoss: true },
});
host.addProcess("game.exe", {
    path: "/games/game.exe",
    modules: {
        "game.dll": { address: 0x10000000n, size: 0x200000n },
    },
    ranges: [{
        address: 0x10000000n,
        bytes: new Uint8Array(0x200000),
        flags: 5,
    }],
});

host.start();
host.updateUntil(
    () => host.timerCalls.splits === 1,
    "the expected split never occurred",
);
```

Add the `.split` source and harness to `RUNTIME_FIXTURES` in
[`src/bin/xtask.rs`](../src/bin/xtask.rs). `cargo xtask check` then compiles the
source in the selected profile, validates the Wasm GC module, and executes the
fixture in the repository and CI verification matrix.

## Processes and memory

`addProcess(name, options)` declares one exact host process name. Its options
model:

- `open`, which can later be changed with `setProcessOpen` to drive detach and
  restart behavior;
- `path`, returned by `process.path()`;
- `modules`, an object whose values contain `address`, `size`, and optionally
  `path`;
- `ranges`, bounded byte arrays with 64-bit base addresses and access flags;
  ordinary process reads must fit wholly inside one range; and
- `read`, an optional focused callback for sparse layouts, intentional failed
  reads, changing values, or assertions about read width and address.

The harness records exact attachment candidates in `attachAttempts` and
detached handles in `detaches`. Addresses are represented as JavaScript
`bigint`, so both 32-bit target values and the complete unsigned 64-bit address
space remain exact. A custom read callback receives the guest destination,
length, address, and host, whose `view()` and `bytes()` helpers write the Wasm
memory.

## Settings and timer state

Initial settings are passed to the constructor or `instantiate`; `setSetting`
changes the live host map before a later update. Every load receives an actual
snapshot, allowing `settings` and `oldSettings` behavior to be tested. The
harness also records setting widgets, choice options, file filters, tooltips,
and owned map/value handles so fixtures can assert that handles are released.

Set `timerState`, `currentSplitIndex`, or `segmentHistory` before an update to
model the LiveSplit timer. Calls are recorded under `timerCalls`, while custom
variables, debug messages, and requested frequencies are available through
`variables`, `messages`, and `tickRates`. Timer start and reset update the
fixture's basic timer state; ports needing a more elaborate run model should
drive the public fields explicitly.

`update(count)` runs an exact number of guest updates. `updateUntil` is a
bounded polling helper for attachment and cooperative async discovery; its
default limit is 32 and failures include a JSON-safe host summary. Never use an
unbounded loop in a conformance fixture.

## Scope and evidence

The harness simulates the ABI, not the LiveSplit scheduler or an operating
system. It proves generated-module behavior under declared host observations;
host scheduling, interruption, and Wasmtime-specific behavior still require
tests in `livesplit-core`. Use real process names, addresses, settings keys,
timer transitions, and failure paths from the source autosplitter. Classify a
port as runtime-verified only for behavior its fixture actually exercises.

The action-default, tick-rate lifecycle, host-metadata, settings, and signed
pointer fixtures are the small reference cases for one-time setup, initial and
subsequent detachment, live settings snapshots, process metadata, module
lookups, full-width addresses, sparse reads, and pointer traversal.
