# ADR 0001: Bundle direct core-Wasm services in isolated workers

- Status: accepted
- Date: 2026-08-18

## Context

The VS Code extension must provide compilation and the complete language
service on desktop, remote, and web extension hosts without downloading or
spawning platform-specific executables. The native `splitc` and `splitls`
programs must remain first-class products for command-line workflows and other
editors.

The compiler and language server already share an editor-neutral Rust library.
Their embedded boundaries need byte-oriented requests, deterministic compiler
identity, revision-safe responses, and isolation from long-running compiler
work. Neither service needs ambient filesystem, process, socket, clock, or
terminal access: hosts supply source snapshots and own every external effect.

## Decision

The extension bundles an optimized `wasm32-unknown-unknown` core WebAssembly
module produced by `splitscript-vscode-wasm`. It is instantiated twice in
separate long-lived workers:

1. The compiler worker owns the versioned in-memory compiler service. Requests
   contain a URI, optional native path, source revision, complete source text,
   build profile, and warning policy. Responses contain structured diagnostics,
   canonical compiler identity, and transferable artifact bytes.
2. The language worker owns the Rust language server. It exchanges direct JSON
   messages with `vscode-languageclient` and keeps its document database
   isolated from compilation latency and compiler-worker restarts.

Desktop hosts use Node `worker_threads`; web hosts use Web Workers. Both use the
same core Wasm module, service protocol, and LSP implementation. Host adapters
are deliberately thin: they load packaged bytes, translate worker transports,
write artifacts through VS Code's workspace filesystem, and enforce
revision/task ownership.

Native `splitc` and `splitls` link the same compiler and language-service
library. They add native concerns only at the edge: file I/O, watch polling,
terminal diagnostics, and stdio LSP framing. They do not call through the
extension adapter.

## Why direct core Wasm

WASI is not part of the extension boundary. Supplying a WASI runtime would add
an operating-system-shaped capability layer even though the embedded services
intentionally receive all inputs as values. It would also complicate browser
and virtual-workspace hosts without improving compiler semantics.

The WebAssembly Component Model is likewise not required for this package
boundary. The adapter has one private, versioned byte protocol, raw artifact
transfer matters for build size and latency, and TypeScript owns both sides of
the embedding. A future standardized component interface may still be useful
for the LiveSplit host ABI; that is a separate runtime decision and does not
replace the extension's workers.

Core Wasm keeps the portable artifact small and universally instantiable while
preserving the ordinary native Rust products.

## Conformance boundary

One implementation does not mean one test layer. Conformance is divided by
responsibility:

- Rust compiler, documentation, and LSP tests prove semantic behavior shared
  by native and embedded frontends.
- Adapter tests validate request envelopes, compiler identity, structured
  diagnostics, artifact lengths, and direct JSON-message LSP behavior against
  the generated core Wasm.
- Worker tests exercise initialization, transferable buffers, request
  ownership, failure propagation, restart behavior, and stale-result
  suppression for Node and browser adapters.
- Packaged desktop and web-host tests prove manifest wiring, virtual workspace
  access, language-server restart, release builds, debug watch, and artifact
  writes using the actual extension bundle.
- Native CLI/LSP tests cover native-only file, terminal, watch, and stdio
  behavior.

Passing only the Rust library suite is therefore insufficient evidence for a
VSIX, and passing only extension tests cannot redefine compiler semantics.
Generated modules and service responses carry the same package version and
optional Git revision so results can be compared across these layers.

## Consequences

- The VSIX is platform-neutral and has no native binary, network bootstrap, or
  compiler-path setting.
- Compilation cannot block language queries because each service owns a worker
  and Wasm instance.
- Worker startup and duplicate Wasm instances cost memory; packaging and
  performance audits must measure that cost.
- Compiler requests yield between analysis, Wasm-IR lowering, and encoding.
  Debug watch cancels superseded staged products while revision checks remain
  the final guard against stale publication.
- Native releases and the VSIX are packaged separately, but must run the same
  conformance corpus and expose the same compiler identity.
- WASI or components should be reconsidered only after a measured requirement
  cannot be expressed through the current value-oriented service boundary.
