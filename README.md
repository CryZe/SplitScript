# SplitScript

SplitScript is a statically typed scripting language for LiveSplit auto
splitters. It combines a compact domain-specific syntax with a familiar
JavaScript/C#-style expression language, compiles directly to WebAssembly GC,
and uses the sandboxed Auto Splitting Runtime ABI.

This repository contains a working compiler slice, not just a syntax proposal.
It parses and type-checks source, lowers game state and durations to GC structs,
generates the process/timer loop, and emits a validated `.wasm` module.
The public compiler facade exposes `parse`, `lower`, `check`, `lower_wasm`,
`codegen`, and `format_source` individually for tooling, while `compile`
retains the one-shot workflow. Their `*_with_options` variants select an
explicit debug or release profile.

```text
state ["Lunistice.exe", "Lunistice-Demo.exe"] {
    points: u32 at "GameAssembly.dll", 0x02dd53f0, 0xb8, 0x20, 0x12c;
    loading: bool at "GameAssembly.dll", 0x02dd5480, 0x10;
}

settings {
    /// Splits whenever the point counter changes.
    "Split on Points" => splitOnPoints: true,
}

onAttach {
    let gameAssembly = await process.module("GameAssembly.dll")
    let metadata = await gameAssembly.scan(sig"48 8B ?? ?? 89")
}

split {
    let changed = current.points != old.points
    return settings.splitOnPoints && changed;
}

isLoading {
    return current.loading;
}
```

Expressions intentionally feel familiar to JavaScript and C# authors: `let`,
property access, `==`/`!=`, `&&`/`||`, and ordinary control flow. Repetitive
autosplitter concepts use their own syntax: `state`, `settings`, `onAttach`,
`start`, `split`, `reset`, `isLoading`, and `gameTime` are top-level constructs,
not disguised function exports.

The examples include a full port of the currently implemented behavior in the
unfinished Rust Minish Cap autosplitter. It exercises GBA emulator discovery,
hardware-address translation, inferred generic reads, settings, timer
variables, delayed splits, and frame-based game time:

```console
cargo run --bin splitc -- examples/minish_cap.split -o target/minish_cap.wasm
```

The maintained Operation Matriarchy port demonstrates bounded native strings
and exact structured-name parsing with the immutable `String.split` API:

```console
cargo run --bin splitc -- examples/operation_matriarchy.split -o target/operation_matriarchy.wasm
```

## Build and use

```console
cargo test
cargo run --bin splitc -- examples/lunistice.split -o target/lunistice.wasm
wasm-tools validate --features all target/lunistice.wasm
```

Before submitting compiler or tooling changes, run the complete repository
matrix:

```console
cargo xtask check
```

It checks formatting and Clippy, runs every Rust and VS Code test, compiles and
validates the release/debug fixture modules, and executes every Node host
runtime—including both Lunistice layouts. Generated modules stay under the
ignored `target/verify` directory.

Maintained ports use the shared deterministic
[`SplitScriptHost`](docs/PORT_CONFORMANCE.md) fixture instead of duplicating
the runtime ABI. It models exact process attachment, modules and memory,
settings snapshots, timer state, detach/restart transitions, and bounded async
polling; the guide explains what runtime-verified evidence does and does not
prove.

The public standard library is authored in
[`stdlib/standard.split`](stdlib/standard.split). A small privileged loader
uses the same syntax crate and lexer as ordinary programs, then generates the
stable symbol IDs and typed catalog consumed by the compiler, documentation,
and editor tooling. Rust owns
only core representations and the closed intrinsic/runtime trust contracts;
ordinary `.split` programs cannot enable the privileged decorators.

After installation, the CLI is `splitc`:

```console
splitc game.split -o game.wasm
splitc game.split -o game.wasm --profile release
splitc game.split -o game.wasm --deny warnings --allow SS1003
splitc fmt game.split
splitc fmt game.split --check
splitc --help
splitc --version
```

`splitc --help` documents compilation, watch, formatting, profiles, and warning
policy. Command-specific help is available through `splitc help watch`,
`splitc watch --help`, and the corresponding `fmt` forms. Help and version
queries write to standard output and exit successfully; invalid invocations
retain a distinct usage-error exit status.

`splitc fmt` formats a syntactically valid source file in place. `--check`
performs the same operation without writing and succeeds only when the file is
already canonical. Syntax errors are rendered normally and never cause a
partial rewrite.

Compile and watch commands accept repeatable `--allow`, `--warn`, and `--deny`
warning selectors. A selector is a stable warning code such as `SS1002`, or
`warnings` for every warning; later selectors take precedence. Denied warnings
fail the build without changing their `SS100x` diagnostic identity.

The initial language server is available over standard LSP stdio framing:

```console
cargo run --bin splitls
```

It currently supports full document synchronization, versioned diagnostics,
whole-document formatting, and full-document semantic highlighting for both
ordinary language constructs and autosplitter-specific domains such as state
fields, settings, lifecycle blocks, signature literals, and debug-only code.
It also provides context-sensitive completion for language constructs,
catalog-backed standard-library calls, state and setting snapshots, records,
enums, inferred methods, and visible lexical bindings such as parameters and
preceding local or awaited variables. Hover exposes inferred types for source
variables, parameters, fields, and functions, plus transitive effects and
runtime constraints for user functions. User-authored `///` documentation on
source declarations is included in those hovers. Catalog-backed hover and
signature help also include resolved generic types, parameter documentation,
runtime effects, and examples. Inlay hints show inferred types beside unannotated globals,
locals, parameters, function return types, state fields, and awaited or retried
bindings while leaving explicit annotations uncluttered.
Go-to-definition and find-references cover source functions, records and their
fields, enums and variants, globals, locals, state fields, and settings. Source
navigation is identity-based, so shadowed or same-spelling declarations remain
separate; catalog symbols intentionally have no source location. Prepared
rename uses that same identity index and rejects invalid, reserved, conflicting,
or reference-capturing names before returning a same-document workspace edit.
The document outline groups state fields and nested settings alongside records,
enums, functions, globals, methods, and lifecycle blocks. Diagnostics with
structured compiler fixes are also exposed as LSP quick-fix code actions.
Editor integrations should launch it directly and communicate over
stdin/stdout; stdout is reserved for protocol messages.

An initial VS Code client lives in [`editors/vscode`](editors/vscode). It
associates `.split` files and runs the same Rust language-server implementation
from the compiler WebAssembly bundled with the extension. A dedicated worker
forwards all advertised LSP features including formatting, while the extension
supplies language editing rules plus a fast TextMate fallback while semantic
tokens are loading. During repository development, install and compile it with:

```console
cd editors/vscode
npm install
npm run check
npm run compile
```

Open the repository in VS Code and launch **Run SplitScript Extension** to build
the bundled WebAssembly adapter and TypeScript client before starting an
Extension Development Host. No separate `splitls` or server-path setting is
needed. **SplitScript: Start Debug Watch** compiles immediately in debug mode
and rebuilds the neighboring `.wasm` whenever the source is saved;
its status-bar item stops the watcher. **SplitScript: Build Release** saves and
performs a one-shot release build for the final module. Both are also editor
title and context-menu actions. These build workflows use the Rust compiler
WebAssembly bundled with the extension in a dedicated worker; they do not
discover, download, or spawn `splitc`.

The same extension also has a bundled browser entry. Launch **Run SplitScript
Web Extension** to exercise it in VS Code's web extension host. Both compilation
and language tooling use browser workers, `ExtensionContext.extensionUri`, and
`vscode.workspace.fs`, so the package has no platform-specific executable path
and can operate on virtual workspace providers.

During development, use the `watch` subcommand. It compiles immediately and
then recompiles whenever the source contents change:

```console
splitc watch game.split -o game.wasm
splitc watch game.split -o game.wasm --profile release
```

Successful builds replace the output only after the complete module is ready,
so a debugger or runtime can reload it without observing a partial file. A
compiler error is printed normally and leaves the last successful `.wasm`
untouched. Press Ctrl+C to stop watching.

The default profile is `debug`. `--profile release` is the canonical release
spelling for both one-shot and watch builds. Statements, bindings, globals, and
functions prefixed with `debug` are checked in both profiles and erased from
release semantic lowering before backend reachability analysis, so their
storage, strings, and imports are omitted. Debug modules include symbolic names
for every imported and generated WebAssembly function. They also embed initial
DWARF v5 metadata for source-backed functions and expression-level line
stepping, including explicit rows for statement and control-flow boundaries.
Async bodies additionally distinguish suspension and resumption at the
original `await`/`retry` source span, including suspensions nested in larger
expressions.
Direct synchronous functions also expose source parameter/local names,
primitive scalar types, lexical visibility ranges, and Wasm-local locations.
Reachable source globals expose their names and Wasm-global locations as well.
Executable enum and aggregate global initialization in `_start` maps back to
the declaration; primitive constant initializers remain non-executable Wasm
global expressions and therefore do not fabricate breakpoint addresses.
The CLI records the absolute `.split` path and the extension records VS Code's
native file path; explicitly in-memory compiler calls use `input.split`.
Release modules strip both the name section and all `.debug_*` sections
entirely.

The full example is in [`examples/lunistice.split`](examples/lunistice.split).
See [`docs/LANGUAGE.md`](docs/LANGUAGE.md) for the language,
[`docs/STANDARD_LIBRARY.md`](docs/STANDARD_LIBRARY.md) for the growing reusable
ASR surface, [`docs/LUNISTICE_PORT.md`](docs/LUNISTICE_PORT.md) for the parity
audit, [`docs/ASL_PORTING.md`](docs/ASL_PORTING.md) for compiler-checked
migration recipes, the generated
[`docs/MIGRATION_CAPABILITIES.md`](docs/MIGRATION_CAPABILITIES.md) index for
foreign-language support status, [`docs/COMPILER.md`](docs/COMPILER.md) for
compiler architecture, and
[`docs/ABI.md`](docs/ABI.md) for the generated module contract. Reproducible
compile-time and output-size measurements are recorded in
[`docs/BASELINES.md`](docs/BASELINES.md).

Reviewed production-port evidence also includes the run-scoped visited-map
translation in [`docs/OPENJK_SPEED_PORT.md`](docs/OPENJK_SPEED_PORT.md) and the
native UTF-16 case in
[`docs/BATTLEFRONT_II_PORT.md`](docs/BATTLEFRONT_II_PORT.md). The
timer-metadata and monotonic-delay case is recorded in
[`docs/DARK_SASI_PORT.md`](docs/DARK_SASI_PORT.md), and the ASL process-name
migration and multi-layout case in
[`docs/NIOH_RTA_NO_LOAD_PORT.md`](docs/NIOH_RTA_NO_LOAD_PORT.md). The focused
ASCII string-normalization case is recorded in
[`docs/TIBERIAN_SUN_PORT.md`](docs/TIBERIAN_SUN_PORT.md), and the compile-time
numbered-settings case in [`docs/DDS_PORT.md`](docs/DDS_PORT.md).

## What works now

- A small scripting expression language with optional semicolons at line endings.
- `bool`, `i8`/`u8`, `i16`/`u16`, `i32`/`u32`, `i64`/`u64`, `f32`, and `f64`.
- Contextual inference for integer literals and local variables.
- Reusable functions with forward calls and calls from actions or other
  functions. Unannotated parameters and returns form inferred generic schemes,
  so one function can be instantiated independently for different caller types;
  inferred numeric and memory-reading constraints are checked at every call.
- Named immutable GC records with nested field access, checked literals,
  function parameters and returns, and persistence across suspension.
- Exhaustive `match` expressions for enums, Options, and Results, plus
  structural `==` / `!=` for enums, records, Options, and Results whose
  contained values support equality.
- Explicit `None`/`Some(value)` and `Ok(value)`/`Err(error)` wrapper syntax,
  while plain values still lift automatically when `T?` or `T!` is expected.
- Type-directed methods with an implicit `self` and nested receiver calls.
- Demand-monomorphized generic function bodies and GC arrays, including the
  general `[T]` and exact-length `[T; N]` forms, inferred literals, and typed
  `array[index]` access and plain or compound indexed mutation plus growable
  `push`, source-defined bulk `extend`, in-place indexed and first-value
  removal, optional `pop`, and capacity-preserving `clear`. Fixed arrays of
  readable elements support one transactional typed process-memory read.
- Run-scoped `Set<T>` values with source-declared `Equatable` constraints,
  persistent mutation, containment, insertion/removal, clearing, and length.
- Inferred `for value in collection` loops over `[T]`, `[T; N]`, and `Set<T>`,
  with read-only scoped bindings, `break`/`continue`, single evaluation of the
  iterable, suspension-safe `await`/`retry` bodies in `onAttach`, and fail-fast
  detection of structural mutation through any alias.
- Strict width checking: a `u16` is never silently treated as a `u32`.
- Deterministic integer formatting in bases 2 through 36, with typed
  invalid-radix errors.
- Module-relative and absolute 64-bit pointer paths.
- Ordered fallback process attachment with one or more executable names.
- Suspension-safe `await` and `retry` bindings, including retrying arbitrary
  user-defined `T!` expressions, plus GC `Module` values with typed `address`
  and `size` fields.
- Compile-time checked `sig"..."` literals with byte/nibble wildcards and
  overlapping page-based module scans.
- Checked `v"1.2.3.4"` Windows file-version literals and source-defined PE
  resource parsing through `Module.fileVersion()`.
- Nominal `address` values, typed synchronous/retried primitive reads,
  RIP-relative 32-bit address decoding, arbitrary-range scans, and reusable
  64-bit pointer traversal.
- Awaitable Unity IL2CPP module, image, class, inherited-field, C# backing-field,
  and static-table discovery for 64-bit Unity layouts.
- GC structs for the state snapshots and `Duration` values.
- First-class immutable GC `String` values with inference, content equality,
  UTF-16 managed-string decoding, backtick `{...}` interpolation,
  numeric formatting, concatenation, dynamic printing, and persistence across
  suspension.
- Automatic process attachment, detachment, memory reads, and old/current
  transactional snapshot rotation that skips torn read ticks.
- Tick-yielding `await process.module(...)` barriers in `onAttach`, automatically
  cancelled and reset if the process closes.
- A GC-backed continuation frame that preserves `onAttach` locals and resumes
  ordinary statements exactly once across suspension boundaries.
- Typed `print` calls in any action block.
- Runtime variables, timer-state inspection, explicit game-time pause/resume,
  tick-rate control, and normalized duration conversion.
- Live typed settings: nested titles and tooltips, booleans, compile-time finite
  integer-keyed families, stable string host-map keys, enum-backed choices,
  file selectors with glob/MIME filters, and automatic `settings`/`oldSettings`
  tick snapshots.
- `whileAttached`, `start`, `split`, `reset`, `isLoading`, and `gameTime`
  actions, plus one-shot `onAttach` and `onDetach` lifecycle blocks.
- LiveSplit timer-state ordering matching the ASL v2 prototype.
- Source spans and concise diagnostics.
- Tooling-facing standard-library and language catalogs with stable IDs,
  shared documentation metadata, and compiler-checked examples.
- Unit tests plus validation of emitted WebAssembly with GC enabled and disabled.

## Current limits

The implemented Lunistice target uses 64-bit IL2CPP layouts. Package imports and
general-purpose suspending user functions and race combinators remain future
language-library work.

The emitted module requires a WebAssembly engine with the GC proposal enabled.
The host calls themselves match the existing Auto Splitting Runtime ABI, but an
older LiveSplit/runtime build that disables WebAssembly GC cannot instantiate the
module.
