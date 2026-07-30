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
When the compiler gains reusable per-file/editor queries, add separate warm and
incremental query measurements rather than folding them into this one-shot
baseline.
