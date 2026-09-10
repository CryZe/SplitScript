# VS Code Auto Splitter debugger adapter plan

## Goal

Add a desktop-only debugger to the SplitScript VS Code extension that runs Auto
Splitting Runtime (ASR) WebAssembly modules in VS Code's Node/V8 WebAssembly
engine, exposes the ASR host ABI, and presents the useful state from
`asr-debugger` as native VS Code views.

The existing language server and compiler remain browser-compatible. The
debugger is an additional desktop capability and does not make the browser
extension depend on native code.

## Findings

- `asr-debugger` has eight tabs: Main, Statistics, Logs, Variables, Settings
  GUI, Settings Map, Processes, and Performance.
- `livesplit-auto-splitting` currently combines three concerns:
  - a Wasmtime execution engine and scheduler;
  - the ASR host ABI (timer, runtime, settings, process, and WASI imports);
  - native process discovery and memory access.
- The ASR imports are synchronous WebAssembly imports. Process operations
  therefore need a synchronous native boundary; an asynchronous child-process
  RPC protocol is not a good fit.
- The checked-in SplitScript example modules compile successfully with the
  installed Node 24 `WebAssembly.Module`, including the Wasm GC modules.
- SplitScript debug builds already contain a name section and DWARF 5 data for
  source lines, functions, globals, locals, lexical scopes, and variable types.
  This is useful metadata, but the JavaScript WebAssembly API does not itself
  provide pause, step, or local-variable inspection.
- The extension already has separate Node and browser entry points and runs its
  compiler and language server in workers. The runtime should follow the same
  isolation pattern.

## Proposed architecture

```text
VS Code debug session / views
            |
            v
  TypeScript session controller + inline DAP adapter
            |
            v
  Node worker thread (one per launched ASR module)
    - WebAssembly.Module / Instance
    - tick scheduler
    - timer model, logs, settings and handle tables
    - WASI preview 1 adapter
            |
            v
  Rust N-API addon
    - process discovery / attach / liveness
    - process memory reads
    - modules, paths and memory ranges
```

### Why this split

- V8 owns the WebAssembly instance, which keeps runtime state easy to expose to
  TypeScript and makes later source-level instrumentation practical.
- A worker keeps 120 Hz updates and slow or broken modules away from the
  extension host. Terminating the worker is the reliable first implementation
  of `Kill` for an infinite loop; V8's public WebAssembly API has no Wasmtime
  epoch-interruption equivalent.
- Timer, settings, logging, and handle semantics are small deterministic data
  structures and should be implemented in TypeScript.
- Only operating-system process access stays in Rust. Copy the minimal relevant
  implementation from `livesplit-auto-splitting` into a workspace crate with
  source/license attribution, rather than copying the Wasmtime runtime.
- The adapter can use `DebugAdapterInlineImplementation`, while all WebAssembly
  execution remains in the worker. This avoids another protocol process without
  risking extension-host responsiveness.

## Suggested project layout

```text
crates/splitscript-process-native/
  Cargo.toml
  src/lib.rs                 # N-API surface and validation
  src/process.rs             # vendored/adapted native process implementation
  src/process_list.rs
  src/wasi_path.rs

editors/vscode/src/debugger/
  debugAdapter.ts            # DAP request/event implementation
  debugConfiguration.ts      # launch configuration provider
  sessionController.ts       # session ownership and state fan-out
  runtimeProtocol.ts         # typed worker messages
  runtimeWorker.ts           # V8 WebAssembly host and scheduler
  asr/
    imports.ts               # complete import table
    memory.ts                # checked pointer/string/buffer access
    handles.ts               # nonzero i64/BigInt handle tables
    timer.ts
    settings.ts
    userSettings.ts
    wasi.ts
  views/
    runtimeView.ts
    statisticsView.ts
    variablesView.ts
    settingsView.ts
    settingsMapView.ts
    processesView.ts
    performanceView.ts       # later milestone
```

The native add-on should expose domain operations, not raw OS handles or
pointers. Webviews must never call it directly.

## VS Code surface

Contribute a `splitscript` debugger type with `launch` support for active
`.split` and `.wasm` files. Put the debugger views in a dedicated Auto Splitter
Debugger Activity Bar container so the standard Run and
Debug views remain uncluttered; users can still rearrange individual views.

| `asr-debugger` tab | VS Code equivalent |
| --- | --- |
| Main | Runtime view plus debug toolbar commands: launch, restart, stop/kill, reload script; timer state and controls remain in the view |
| Statistics | Statistics tree: tick rate, average/slowest tick, handles, Wasm memory size, reset action, and inline memory action |
| Logs | `Auto Splitting Runtime` Output channel and Debug Console events, with clear/save supplied by VS Code |
| Variables | Live Variables tree while running; later also standard DAP Scopes/Variables while paused |
| Settings GUI | Webview view for bool, title, choice, text, and file-select widgets with tooltips and nesting |
| Settings Map | Read-only hierarchical tree plus Clear action |
| Processes | Tree/table-like view with PID and executable path plus lazy mapped-range memory inspection |
| Performance | Deferred webview histogram; collect bounded samples from the beginning so adding the graph does not change the runtime protocol |

Views should consume immutable session snapshots at a throttled UI rate (for
example 5-10 Hz), not one message per 120 Hz tick.

## Launch model

Support two equally accessible inputs:

1. A saved `.split` file. Compile a debug artifact in memory and launch it
   directly. Save-triggered hot reload reuses the existing compiler worker.
2. An arbitrary `.wasm` file, with an optional script path. Modules with an
   `update` export use the recurring auto-splitting loop; other modules invoke
   `_initialize` / `_start` once when present and otherwise finish running
   their WebAssembly start section during instantiation.

Suggested initial `launch.json` shape:

```json
{
  "type": "splitscript",
  "request": "launch",
  "name": "Debug Active Auto Splitter",
  "program": "${file}",
  "stopOnEntry": false,
  "hotReload": true
}
```

For a hot reload, create a fresh worker and WebAssembly instance. Preserve the
user settings map by default, but reset process handles, Wasm state, timing
statistics, and breakpoint state. Timer preservation should be an explicit
later option, not an accidental behavior.

## ASR host implementation

Implement and test every import currently exposed by
`livesplit-auto-splitting`, grouped as follows:

1. Runtime: tick rate, logging, OS, and architecture.
2. Timer: state reads and all timer actions; use the simulated debugger timer
   from `asr-debugger`, not a real LiveSplit connection.
3. Settings values, lists, maps, compare-and-swap store, and user-setting
   widgets.
4. Process attachment, PID listing, liveness, reads, module queries, executable
   paths, and memory ranges through the Rust add-on.
5. WASI preview 1 for script-runtime modules, with narrowly scoped read-only
   preopens based on the selected script path.

Mirror the ABI's exact i32/i64/f32/f64 signatures. JavaScript i64 values are
`BigInt`; handles should be opaque nonzero `BigInt` values with generation
checking so stale handles cannot alias newly allocated objects. All pointer and
length accesses must be bounds checked against the current exported memory.

Do not expose process writes. The current ASR ABI only requires reads.

## Implementation milestones

### 0. Prove the risky boundaries

Status: completed on 2026-09-08 for Windows x64. The probe compiled a
SplitScript debug module, instantiated and updated it in a Node worker, stopped
an intentionally infinite update by terminating its worker, loaded the Rust
add-on from the unpacked extension, read a known 32-byte value from a fixture
process, and built a VSIX whose file list includes the native `.node` binary.

- Instantiate one generated SplitScript module in a worker with stub imports.
- Build a Windows x64 N-API prototype that attaches to a fixture process and
  reads a known memory value synchronously.
- Verify the native binary can be loaded from an unpacked extension and a VSIX.
- Verify a deliberately infinite `update` can be stopped by terminating the
  worker without hanging the extension host.

Exit criterion: the chosen runtime/add-on/package design works inside the VS
Code extension development host.

### 1. Runtime skeleton and DAP lifecycle

Status: completed on 2026-09-08. The desktop extension now contributes an
inline debug adapter, compiles `.split` programs in memory (or launches `.wasm`
programs directly), runs them in a dedicated Node worker, and supports restart,
termination, save-triggered hot reload, runtime logs, timer controls, and a
Runtime tree view. The worker implements the runtime and simulated timer imports;
at that milestone, process, settings, and WASI imports remained neutral stubs.
The browser extension exposes the debugger as unavailable instead of registering
commands that cannot work there.

- Add debugger contributions, launch configuration provider, and inline DAP
  adapter.
- Refactor the embedded compiler ownership so a debug session can receive
  artifact bytes without first writing and watching a sibling `.wasm` file.
- Add worker protocol, load/restart/terminate, first-update initialization,
  scheduler, error/trap reporting, and hot reload.
- Implement runtime and simulated timer imports.
- Add the Runtime and Logs surfaces.

Exit criterion: F5 launches a simple SplitScript autosplitter, timer actions and
logs appear in VS Code, saves hot reload it, and Stop always returns control.

### 2. Complete ASR compatibility

Status: completed on 2026-09-10. The worker now implements the ASR settings
map/list/value handle APIs, bool/title/choice/text/file widgets and tooltips,
settings preservation across runtime replacement, and the complete WASI
snapshot-preview1 import surface with an empty argument/environment context and
a read-only `/mnt` filesystem. The dedicated debugger sidebar has
interactive Settings, recursive Settings Map, and timer Variables views. Unit
tests cover the handle and filesystem contracts; generated SplitScript probes
and the reference debugger's settings-heavy Wasm modules validate the actual
import signatures and runtime behavior.

- Implement settings handles/maps/lists/values and all user-setting widgets.
- Add Settings, Settings Map, and Variables views.
- Add hermetic WASI snapshot-preview1 support.
- Add import signature/conformance tests against representative modules.

Exit criterion: the settings-heavy modules used by `asr-debugger` behave the
same in both hosts.

### 3. Native process host

Status: completed on 2026-09-08 for Windows x64. The Rust N-API bridge now
adapts the process discovery, attachment, liveness, read-only memory, module,
and mapped-range behavior from `livesplit-auto-splitting`. The Node worker owns
ASR-compatible 64-bit guest handles, keeps native handles inside Rust, exposes
attached processes in the dedicated debugger sidebar, and releases them during
normal shutdown and traps. The production probe compiles a generated
SplitScript autosplitter, attaches it to the native fixture, reads a known byte,
and starts the simulated timer from that value.
Readable mapped ranges can be selected from each process row and are served to
VS Code's Hex Editor in bounded pages through DAP `readMemory`; the full process
is never copied into the extension host.

- Add the minimal Rust process crate and N-API wrapper.
- Add process import implementations and the Processes view.
- Package Windows x64 first, then add a CI matrix for the agreed platforms and
  architectures.
- Gate debugger activation on desktop, supported native binary, local file
  workspace, and workspace trust. Keep language/compiler functionality
  available in browser, virtual, and untrusted workspaces.

Exit criterion: an end-to-end autosplitter attaches to a fixture/game process,
reads memory, and drives the simulated timer.

### 4. Diagnostics and statistics

Status: completed on 2026-09-10. Runtime timing now uses a bounded 2,048-sample
window and the dedicated Statistics view reports tick rate, average and slowest
update duration, handles, and linear-memory usage. A view-title action resets
timing collection, while the memory row opens the read-only Wasm linear memory
directly in VS Code's Hex Editor. Both Wasm and attached-process memory use the same lazy
DAP `readMemory` path. Wasm requests use zero-based offsets, while process
requests preserve their absolute virtual addresses so the Hex Editor can open
at the selected mapping's base. Sidebar snapshot delivery remains coalesced to
five updates per second; histogram rendering stays deferred.

- Add tick duration sampling, average/slowest tick, handle count, Wasm memory
  size, reset, and lazy read-only memory inspection in VS Code's Hex Editor.
- Add bounded retention and UI throttling.
- Defer the histogram rendering unless profiling proves it valuable.

Exit criterion: all non-performance `asr-debugger` panels have a VS Code-native
equivalent and remain responsive at the normal tick rate.

### 5. SplitScript source debugging

Treat this as a separate compiler/runtime feature. DWARF alone is not enough to
pause a V8 WebAssembly instance through the public JavaScript API.

- Add a debugger-instrumented compiler mode distinct from the normal debug
  artifact, so ordinary ASR hosts do not need to satisfy debugger-only imports.
- Emit stable statement/checkpoint IDs and a compact source/variable manifest.
- Insert a synchronous debugger checkpoint import at stoppable locations.
- Pause inside the worker using a dedicated `SharedArrayBuffer` control word and
  `Atomics.wait`; the extension/DAP side resumes it with `Atomics.notify`.
- Add DAP breakpoint validation, stopped/continued events, stack frames,
  scopes, variables, continue, next, step-in, and step-out.
- Extend instrumentation with compiler-generated variable snapshots/getters;
  the JS WebAssembly API cannot enumerate Wasm locals even though DWARF names
  them.
- Preserve async suspend/resume semantics using the existing debug recorder's
  suspend and resume markers.

Exit criterion: breakpoints bind to SplitScript source, pauses do not freeze VS
Code, locals/globals are inspectable, and stepping is deterministic across
ordinary and async code.

## Verification strategy

- TypeScript unit tests for memory bounds, UTF-8, handle lifetime, settings
  compare-and-swap, timer transitions, scheduling, and worker messages.
- Rust unit/integration tests for process discovery, liveness, module/range
  queries, path conversion, invalid handles, and a controlled memory-reading
  fixture.
- Cross-host conformance fixtures run against both this runtime and
  `livesplit-auto-splitting`, comparing timer operations, logs, variables,
  settings, and failure behavior.
- VS Code integration tests for configuration resolution, session lifecycle,
  view updates, hot reload, workspace-trust refusal, missing native binaries,
  VSIX loading, and infinite-loop termination.
- Compiler tests for checkpoint placement and source/variable metadata before
  implementing DAP stepping.

## Risks and explicit tradeoffs

- **Native packaging:** a native add-on turns one portable VSIX into a platform
  matrix. Starting with Windows x64 contains this cost but must be an explicit
  product decision.
- **Runtime parity:** duplicating the ASR ABI can drift from livesplit-core.
  Keep an import manifest/conformance suite and record the upstream revision
  from which native code was copied.
- **V8 interruption:** worker termination is destructive to the instance. A
  hung module can be killed and restarted, not resumed.
- **Breakpoint implementation:** V8 execution in Node is an enabler for custom
  instrumentation, not free debugging support. Avoid promising source stepping
  in the initial runtime milestone.
- **Security:** process inspection is a privileged desktop feature. Require a
  trusted local workspace and validate every webview message, handle, pointer,
  length, and file path.
- **Remote extension hosts:** process APIs observe the machine running the
  extension host, which may be SSH/WSL/container rather than the machine showing
  the VS Code UI. Either make that behavior explicit or disable unsupported
  remote scenarios initially.

## Decisions needed before implementation

1. Is Windows x64 the acceptable first native target, or must the first usable
   version also cover Linux/macOS and ARM64?
2. Should the default launch target be the active `.split` source (recommended),
   while retaining arbitrary `.wasm` as an advanced mode?
3. Should the debugger always use a simulated timer like `asr-debugger`
   (recommended), or is connecting to a real LiveSplit instance in scope?
4. On hot reload, should only settings survive (recommended), or should the
   simulated timer state also survive?
5. Is generic ASR-Wasm compatibility required for the first release, or can the
   first end-to-end slice support SplitScript-generated modules only and fill
   out the remaining ABI immediately afterward?
6. For the copied native process code, should we track a specific livesplit-core
   commit manually, or is adding a small reusable crate upstream and consuming
   it later an acceptable follow-up?
