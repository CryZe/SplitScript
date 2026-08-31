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
cargo run --release --example tooling_baseline -- 500 100
```

The optional arguments select the number of generated helper functions and
measured samples. The runner uses a focused small script, the maintained
Lunistice source as a real medium script, and a deterministic generated large
source. It measures cold and post-edit diagnostics, root and member completion,
hover, semantic tokens, a warm multi-query sequence, full-sync `didChange` to
published diagnostics, and in-process language-server restart to hover. Warm
operations receive 20 unmeasured calls first.

A counting system allocator remains active for the whole run. Every latency row
therefore also reports net retained growth after its warmup and the largest
transient heap increase during its measured samples. A second table constructs
one fresh database per query shape and reports the complete retained cache and
peak heap attributable to that live database. The allocation atomics add a
small constant measurement cost, so compare latency only with runs from this
same runner. The restart row covers rebuilding the Rust language service; the
separate portable-toolchain work still owns browser/desktop Worker startup and
transport measurements.

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

### 2026-08-31 interactive-query baseline

- Rust: `rustc 1.98.0`, release profile
- Platform: Windows x86-64, 32 logical CPUs
- Fixtures: 171-byte focused source, 4,554-byte Lunistice source, and a
  29,990-byte generated source with 500 helpers
- Warmups: 20 per warm query
- Samples: 30 per latency row

| Fixture | Query | Median | p95 | Peak heap delta |
| --- | --- | ---: | ---: | ---: |
| small | cold diagnostics | 47.5 µs | 67.9 µs | 55.6 KiB |
| small | edit → diagnostics | 44.6 µs | 61.2 µs | 13.8 KiB |
| small | edit → root completion | 49.8 ms | 51.0 ms | 3.1 MiB |
| small | edit → member completion | 161.1 ms | 166.4 ms | 5.5 MiB |
| small | edit → hover | 85.7 ms | 90.3 ms | 3.1 MiB |
| small | edit → semantic tokens | 84.9 ms | 92.4 ms | 3.1 MiB |
| small | warm multi-query sequence | 159.5 ms | 166.1 ms | 5.5 MiB |
| small | LSP `didChange` → diagnostics | 102.0 µs | 105.3 µs | 21.9 KiB |
| small | LSP restart → hover | 79.7 ms | 85.4 ms | 5.5 MiB |
| Lunistice | cold diagnostics | 92.0 ms | 98.5 ms | 6.2 MiB |
| Lunistice | edit → diagnostics | 92.1 ms | 96.4 ms | 1.5 MiB |
| Lunistice | edit → root completion | 98.4 ms | 104.1 ms | 1.5 MiB |
| Lunistice | edit → member completion | 251.5 ms | 262.4 ms | 6.3 MiB |
| Lunistice | edit → hover | 92.5 ms | 99.0 ms | 1.4 MiB |
| Lunistice | edit → semantic tokens | 92.1 ms | 97.3 ms | 1.4 MiB |
| Lunistice | warm multi-query sequence | 249.7 ms | 259.2 ms | 6.3 MiB |
| Lunistice | LSP `didChange` → diagnostics | 92.7 ms | 95.8 ms | 1.5 MiB |
| Lunistice | LSP restart → hover | 91.5 ms | 96.7 ms | 6.2 MiB |
| generated large | cold diagnostics | 13.5 ms | 17.2 ms | 6.3 MiB |
| generated large | edit → diagnostics | 12.1 ms | 13.8 ms | 1.0 MiB |
| generated large | edit → root completion | 147.0 ms | 155.4 ms | 8.6 MiB |
| generated large | edit → member completion | 382.0 ms | 412.9 ms | 18.0 MiB |
| generated large | edit → hover | 142.3 ms | 154.4 ms | 7.9 MiB |
| generated large | edit → semantic tokens | 151.4 ms | 163.2 ms | 8.0 MiB |
| generated large | warm multi-query sequence | 378.2 ms | 385.3 ms | 18.0 MiB |
| generated large | LSP `didChange` → diagnostics | 12.1 ms | 12.7 ms | 1.0 MiB |
| generated large | LSP restart → hover | 153.1 ms | 167.8 ms | 18.6 MiB |

The steady live-database cache after the complete warm query sequence is
2.5 MiB for the small fixture, 4.8 MiB for Lunistice, and 10.7 MiB for the
generated large fixture. The corresponding one-shot peak deltas are 8.0 MiB,
11.0 MiB, and 28.4 MiB.

Initial optimization targets for this fixed runner are:

- p95 below 100 ms for each individual post-edit query on every fixture;
- p95 below 150 ms for the complete warm multi-query sequence;
- at most 8 MiB retained by that complete sequence; and
- at most 20 MiB transient heap growth during one sequence.

Diagnostics already satisfy the interaction target. Completion—especially
member completion—and the semantic products it requests are the first measured
bottleneck. Optimize those paths before changing full-sync transport or adding
incremental invalidation machinery.
