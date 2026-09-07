# Binaryen reference experiment

Measured 2026-09-07 against SplitScript `72a6a15`, after the shared async
`br_table` change. The immediate opportunities are better direct emission:
avoid unreachable completion tails and redundant GC null checks. Binaryen's
reader/writer plus its instruction peephole pass takes Lunistice below 30,000
bytes without inlining. More involved release passes offer further savings.

Binaryen is an offline reference for our implementation plan. The compiler,
editor, build pipeline, and emitted modules gain no Binaryen dependency.

## Installation and method

Updated `C:\Projekte\binaryen` from 118 to
[the latest stable release, 132](https://github.com/WebAssembly/binaryen/releases/tag/version_132).
The previous installation is preserved at
`C:\Projekte\binaryen-backup-118-20260907`. The installed `wasm-opt --version`
reports `wasm-opt version 132 (version_132)`.

The official Windows x86-64 archive was verified against its GitHub release
asset SHA-256 digest:
`2089428ec98c899b45ee5d00636ddd6e2da8636cc473ef50b165cc25793ef7cb`.
Source inspection uses the matching `version_132` source, commit `79dfe6b`.

All inputs are generated with the Rust release-built `splitc` and SplitScript
`--profile release`. The experiment enables GC, reference types, multivalue,
bulk memory, sign extension, and nontrapping float-to-int instructions, matching
the relevant capabilities of these modules. It does not enable fast-math or
assumptions that traps never occur.

The five variants are:

- **Rewrite:** Binaryen reads and writes the module with no explicit passes.
  Its IR construction and encoding already change the instruction stream.
- **Peephole:** only `--optimize-instructions`, in addition to that read/write
  behavior. This is a family of local simplifications, not constant propagation
  across the whole program or inlining.
- **O4:** Binaryen's standard `-O4` pipeline.
- **Oz:** Binaryen's standard `-Oz` pipeline, focused on size.
- **Closed ceiling:** `-O4 --shrink-level=2 --closed-world --converge`.
  This is a combined experiment, not an isolated measurement of closed-world
  optimization. These fixtures expose numeric host calls and linear memory;
  their GC/function references are internal. Recheck that boundary before
  applying the assumption to a different ABI.

The [versioned pass schedule](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/pass.cpp)
shows that O4 adds IR flattening and local CSE, while size-focused settings
change other decisions. Closed-world mode enables additional type/signature
and GC-field analysis. There is no single flag meaning every possible safe,
profitable optimization, and O4 is not a promise of the smallest binary.

## Size results

Raw module bytes, including custom sections:

| Fixture | SplitScript | Rewrite | Peephole | O4 | Oz | Closed ceiling |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Lunistice | 33,439 | 31,323 | 29,418 | 27,974 | 26,960 | 22,678 |
| Minish Cap | 48,773 | 46,938 | 43,821 | 40,010 | 39,535 | 28,075 |
| settings | 8,790 | 8,792 | 8,589 | 8,130 | 8,112 | 7,691 |
| cancellation | 2,727 | 2,700 | 2,657 | 2,365 | 2,341 | 1,855 |
| managed instances | 16,867 | 15,755 | 14,918 | 13,770 | 13,421 | 9,928 |
| managed instances Mono | 25,454 | 23,174 | 21,879 | 20,018 | 19,557 | 16,985 |
| set runtime | 3,597 | 3,607 | 3,523 | 3,191 | 3,112 | 2,761 |
| map runtime | 5,016 | 5,030 | 4,864 | 4,176 | 4,143 | 3,705 |

Every variant retains 160 bytes of custom sections. These differences are not
debug-metadata stripping. Oz is smaller than O4 on all eight fixtures. The
closed ceiling is an empirical reference point, not a proven minimum or an
immediate promise for our own compiler.

Representative code-section sizes (including section framing) establish where
the simpler savings occur:

| Fixture | Original code | Rewrite code | Peephole code |
| --- | ---: | ---: | ---: |
| Lunistice | 29,870 | 27,722 | 25,817 |
| Minish Cap | 42,862 | 41,027 | 37,910 |
| managed instances Mono | 22,170 | 19,875 | 18,580 |

Rewrite and peephole retain the original defined-function counts: respectively
62, 61, and 54 for these fixtures. O4 reduces them to 21, 23, and 23; Oz to
25, 26, and 29. The simple wins do not depend on function inlining.

## Concrete findings and implementation order

### 1. Stop emitting completion tails after unconditional transfers

WAT comparison shows unreachable frame-completion stores and returns after
unconditional `br` and `return`. For example, a future continuation contains:

```wat
;; The state was set immediately above this sequence.
br 45
local.get 0
ref.as_non_null
i32.const -1
struct.set 84 0
i32.const 1
return
```

The six instructions after `br` cannot execute. The responsible pattern is in
[compile_async_body](src/codegen/async_state.rs): it always appends the default
completion sequence after emitting a state, even when that state's terminator
already branches or returns. Similar terminal fallbacks deserve inspection in
[ordinary function emission](src/codegen/script_functions.rs).

Binaryen's rewrite removes 130 `return` instructions from both Lunistice and
Minish Cap, and 150 from the Mono fixture. It also removes corresponding stores,
constants, and frame loads. The total rewrite savings are 2,116, 1,835, and
2,280 bytes. Those totals include other reader/writer changes, so they are not
an exact forecast for removing our completion tails alone.

Implement a small fallthrough result for body/state emission, using the
existing Wasm IR terminators and nested control flow. Emit a completion tail
only on a path that can reach it. Do not add a second general control-flow
analysis or suppress validation of unreachable source. Keep required structured
`end` instructions and default/out-of-range dispatcher handling.

**Profile:** shared debug/release emission. **Acceptance:** exact runtime traces
for return, retry, suspension, break/continue, failure, and cancellation;
valid branch depths and debug locations; measured encoded-byte savings.

### 2. Avoid redundant null assertions at known GC consumers

The largest isolated local pass is `optimize-instructions`. Much of its change
is removing `ref.as_non_null` before an operation that already traps on null:

```wat
local.get 0
ref.as_non_null
struct.get 84 0
;; becomes:
local.get 0
struct.get 84 0
```

| Fixture | Assertions after rewrite | After instruction pass | Additional module bytes saved |
| --- | ---: | ---: | ---: |
| Lunistice | 1,602 | 23 | 1,905 |
| Minish Cap | 2,474 | 13 | 3,117 |
| managed instances Mono | 1,187 | 2 | 1,295 |

The pass also performs other instruction simplifications; its entire saving
must not be attributed to null assertions. Concrete sources of assertions are
[AsyncFrameRef::emit](src/codegen/async_frame.rs), expression emission, and
[typed frame/array reads](src/codegen.rs), which currently normalize references
before the consumer's requirements are known.

Start with unary consumers such as `struct.get` and `array.len`, and references
whose declared Wasm type is already non-null. Make the distinction explicit in
the emitter's reference/consumer contract, retaining checks when a non-null
local, argument, or result type requires them.

For writes and indexed operations, preserve trap ordering. Given a null receiver,
`ref.as_non_null; call valueProducer; struct.set` traps before the call. Removing
the assertion would move the trap after that call. Binaryen's
[skipNonNullCast](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/OptimizeInstructions.cpp#L1533)
explicitly checks effects in subsequent operands. We should initially limit
removal to provably safe consumers, then reuse our effect information where it
is sufficient. Treat downcasts separately from null assertions.

**Profile:** shared direct emission for these cheap cases. **Acceptance:** null
receivers, side-effecting/trapping indices and values, nullable arrays/fields,
future frames, and required non-null function signatures; measure the actual
subset saved instead of promising the whole Binaryen pass's result.

### 3. Improve GC type planning before adding global GC optimization

[gc_types.rs](src/codegen/gc_types.rs) emits the planned GC types as one recursive
group. An isolated `minimize-rec-groups` saves another 464 bytes in Lunistice,
706 in Minish Cap, and 392 in Mono relative to rewrite. Isolated type pruning
and reordering also save hundreds of bytes. Some benefits include changed type
indices and their encoded operand widths, not just fewer declarations.

Investigate smaller recursive components and tighter type demand using the
existing layout/reachability planning. Preserve type identity, subtyping,
recursive references, casts/tests, and function signatures: splitting groups
must not accidentally make distinct structural types equivalent. Binaryen's
[MinimizeRecGroups](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/MinimizeRecGroups.cpp)
handles identity conflicts and public types; a simple SCC split is not its whole
algorithm.

**Profile:** prefer shared planning if its cost is small; measure before adding
a release-specific rewrite. Defer whole-program field removal and signature
specialization until their remapping and observability contracts are explicit.

### 4. Add bounded release simplification before aggressive inlining

The useful next release layer is typed constant propagation, dead-branch
cleanup, branch simplification, and common branch-tail factoring. Reuse existing
IR, type, and effect information; measure pass costs and avoid repeatedly
rebuilding the entire program's derived products.

An isolated `precompute-propagate` saves 147/299/53 bytes on
Lunistice/Minish Cap/Mono. `code-folding` saves 303/1,760/216, and
`remove-unused-brs` saves 365/558/191. These measurements are independent and
must not be added together. Gains can change substantially after earlier passes.

By comparison, isolated local coalescing saves only 51/125/19 bytes. It is not
the leading size opportunity here, and its
[implementation is nonlinear in local count](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/CoalesceLocals.cpp).
Compiler RAM reduction is not a reason to prioritize it over latency and size.

Plain `--inlining` alone **increases** size over rewrite by 2,946/3,019/609
bytes. The full pipelines profit from inlining together with cleanup,
propagation, dead-argument elimination, and function removal. Start with small
or single-use callees under an encoded-size budget, followed by cleanup and
reachability updates. Reject unrestricted inlining as the first step.

Reference implementations:
[Precompute](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/Precompute.cpp),
[RemoveUnusedBrs](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/RemoveUnusedBrs.cpp),
[CodeFolding](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/CodeFolding.cpp),
[Inlining](https://github.com/WebAssembly/binaryen/blob/version_132/src/passes/Inlining.cpp).
For a future port, keep Apache-2.0 attribution/license requirements with any
adapted Binaryen source. No pass implementation was imported in this experiment.

## Validation and compilation cost

The reproducible matrix validates 48 modules with `wasm-tools` and runs 78
runtime cases: the originals and five variants across 13 scenarios. It compares
the complete harness stdout against the corresponding original and requires
successful exit. Scenarios include Lunistice base/DLC, transient metadata reads,
mixed/inherited layouts, both Minish Cap backends, settings, cancellation,
managed runtimes, sets, and maps. All matched. The 75 isolated pass outputs
(25 passes on three fixtures) also passed Wasm validation; that sweep is size
attribution, not equivalent runtime coverage for every isolated pass.

Node 24.14.0 on Windows hit `UV_HANDLE_CLOSING` during forced process shutdown
in the optimized mixed-layout fixture. The matrix uses `--single-threaded
--no-wasm-async-compilation` for both originals and variants, under which that
scenario passes. These are semantic trace checks, not generated-code runtime
benchmarks or exhaustive proofs of equivalence.

Single-run optimizer wall times, including process startup and I/O, were about
0.64/1.47 seconds for O4 on Lunistice/Minish Cap, 0.33/0.53 seconds for Oz, and
1.19/1.48 seconds for the closed ceiling. Rewrite and the single instruction
pass were about 18–26 ms across fixtures. These are illustrative experiment
costs, not stable benchmarks or estimates of a Rust port. The existing compiler
baseline is roughly 59 ms for Lunistice. Heavy iterative work belongs behind a
release optimization budget; the first two emission fixes need no such pipeline.

This experiment measures generated size. Compiler/editor latency remains a
separate priority, with repeated standard-library frontend work still an open
target in [PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md).

## Reproduction

From the repository root, after building `splitc`:

```powershell
cargo build --profile max-opt --bin splitc
node scripts/binaryen-review.mjs C:\Projekte\binaryen\bin\wasm-opt.exe
```

The script accepts optional compiler and output-directory arguments. It writes
the exact flags, runtime scenarios, section sizes, function-body sizes, and
optimizer wall times to `target/performance-review/binaryen-132/results.json`,
alongside original and optimized modules. It requires Node and `wasm-tools` on
PATH; it does not install tools or alter the compiler.

To reproduce one isolated pass, use the same features and an original module:

```powershell
$dir = 'target/performance-review/binaryen-132'
$features = (Get-Content "$dir/results.json" -Raw | ConvertFrom-Json).features
& C:\Projekte\binaryen\bin\wasm-opt.exe "$dir/lunistice.original.wasm" @features --optimize-instructions -o "$dir/lunistice.single-pass.wasm"
wasm-tools validate --features all "$dir/lunistice.single-pass.wasm"
wasm-tools print "$dir/lunistice.single-pass.wasm" -o "$dir/lunistice.single-pass.wat"
```

Use `--<pass-name>` for the other isolated passes. The `gto`, `cfp`,
`signature-pruning`, `remove-unused-types`, and `reorder-types` experiments also
used `--closed-world`. Compare their non-custom bytes with **rewrite**, not
with the compiler output, to avoid crediting each pass with reader/writer gains.
The ignored `passes.json` and WAT files retain the detailed local investigation.
