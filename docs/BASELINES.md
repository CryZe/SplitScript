# Compiler baselines

SplitScript keeps a small, dependency-free baseline runner for end-to-end
compiler latency and generated WebAssembly size:

```console
cargo run --release --example compiler_baseline -- 200
```

The optional positional argument is the number of measured samples per
fixture. Each fixture receives 20 unmeasured warmup compilations first. Every
sample calls the public one-shot `splitscript::compile` API in-process; the
numbers exclude building the Rust compiler executable, filesystem I/O,
`wasm-tools` validation, and host-runtime execution.

Timing values are diagnostic baselines, not test thresholds. OS scheduling,
CPU power state, Rust updates, and allocator changes can move them without a
compiler regression. Generated Wasm byte counts are deterministic, but should
also be reviewed rather than frozen into brittle assertions because valid
backend changes can alter them intentionally.

## 2026-07-28 baseline

- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6
- Cargo: 1.97.0
- Platform: Windows x86-64, 32 logical CPUs
- Profile: `release`
- Warmups: 20 per fixture
- Samples: 200 per fixture

| Fixture | Source bytes | Wasm bytes | Median | p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 19 | 433 | 8.0 µs | 8.5 µs |
| Lunistice | 6,823 | 11,763 | 716.3 µs | 873.7 µs |
| cancellation | 506 | 2,155 | 50.0 µs | 65.3 µs |
| settings | 2,735 | 4,400 | 92.2 µs | 121.2 µs |

The Lunistice fixture is currently the broadest real autosplitter in the
repository and is the primary trend signal. The smaller fixtures help identify
fixed compiler overhead and regressions isolated to async lowering or settings.

## Tooling and large-source baseline

Reusable editor-query and in-process LSP measurements have a separate runner:

```console
cargo run --release --example tooling_baseline -- 500 200
```

The optional arguments select the number of generated helper functions and
measured samples. The generated source is deterministic and deliberately much
larger than the current real ports. The runner reports initial strict checking,
warm cached checking, hover, semantic highlighting, definition indexing, and a
full JSON-RPC hover through `LanguageServer::handle`. Warm operations receive
20 unmeasured calls first. Keeping this separate from the one-shot compiler
runner distinguishes query-cache regressions from front-end/backend work.

Record results with the toolchain, platform, source size, function count, and
sample count. Timing remains a trend signal rather than a brittle test
threshold. Add a generated large-catalog dimension when `CompilerContext` can
own and inject an alternate validated graph; measuring repeated lookups in the
fixed bundled catalog would not exercise catalog scaling.

### 2026-07-31 tooling baseline

- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6
- Platform: Windows x86-64, 32 logical CPUs
- Profile: `release`
- Fixture: 500 functions, 29,879 source bytes
- Warmups: 20 per warm query
- Samples: 200 (one sample for the intentionally cold check)

| Query | Median | p95 |
| --- | ---: | ---: |
| database cold check | 3,966.7 µs | 3,966.7 µs |
| database warm check | 0.0 µs | 0.1 µs |
| database warm hover | 5.6 µs | 6.1 µs |
| database warm highlights | 0.0 µs | 0.1 µs |
| database warm definitions | 0.0 µs | 0.1 µs |
| in-process LSP warm hover | 31.7 µs | 45.3 µs |
