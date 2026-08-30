# SplitScript

SplitScript is a statically typed language for writing autosplitters. It gives
process memory, settings, timer decisions, errors, asynchronous discovery, and
game-specific providers first-class language and standard-library support, then
compiles each script to a sandboxed WebAssembly GC module for the Auto Splitting
Runtime ABI.

The project is usable for early ports and real-game testing, but it is not yet a
stable release. Source compatibility may change, the VS Code extension is
distributed as a VSIX rather than through the Marketplace, native binaries are
not published, and the generated module still needs a compatible autosplitting
host with WebAssembly GC enabled.

## The language at a glance

A native state declaration attaches to an exact process name and reads all of
its fields into coherent `old` and `current` snapshots. Lifecycle blocks then
return typed timer decisions:

```splitscript
state "game.exe" {
    level: u32 at 0x1234
}

split {
    return current.level != old.level
}
```

Windows process names include `.exe`. The address above is illustrative; a real
script uses verified addresses or pointer paths. Typed emulator and Unity state
providers cover targets that should not be modeled as native pointer paths.

The compiler checks this code, generates the process/timer update loop, and
emits a `.wasm` module. The extension and CLI share the same compiler,
documentation catalog, formatter, diagnostics, and language service.

## Choose your path

### Visual Studio Code authors

Install the [latest SplitScript VSIX][latest-vsix], open a saved `.split` file,
and run **SplitScript: Open Documentation**. The bundled **Getting started**
guide is a self-contained, compiler-checked path through attachment, settings,
state snapshots, builds, diagnostics, and host loading. The package is rebuilt
from every verified push to `master`, so it is an early moving build rather than
a stable release.

Use **SplitScript: Start Debug Watch** while editing and **SplitScript: Build
Release** for the final module. Both write a neighboring `.wasm` file. The
[extension package page](editors/vscode/README.md) documents every command,
requirement, limitation, and recovery step.

### Native CLI users

From a repository checkout with the Rust toolchain installed:

```console
cargo run --bin splitc -- game.split -o game.wasm --profile release
cargo run --bin splitc -- watch game.split -o game.wasm
cargo run --bin splitc -- fmt game.split
cargo run --bin splitc -- docs
```

A failed build leaves the previous successful output untouched. `splitc docs`
renders the same compiler-owned reference used by the extension; add a symbol
or search term to open a focused page. The same hierarchy is published as a
[searchable HTML reference](https://cryze.github.io/SplitScript/) with semantic
SplitScript highlighting and symbol navigation.

### Authors porting ASL

Start with the bundled **Porting ASL to SplitScript** guide or run:

```console
cargo run --bin splitc -- docs "Porting ASL to SplitScript"
```

The [source guide](docs/ASL_PORTING.md) is self-contained and uses focused,
compiler-checked snippets. It covers semantic migration rather than presenting
complete autosplitter files as templates. Diagnostics also recognize common
foreign spellings and link to the corresponding migration concept when a safe
automatic rewrite is not possible.

### Compiler and tooling contributors

The durable contributor references are:

- [Compiler architecture](docs/COMPILER.md) for parsing, checking, semantic
  queries, Wasm IR, code generation, LSP, and extension workers.
- [Language design](docs/LANGUAGE.md) for source semantics.
- [Standard-library design](docs/STANDARD_LIBRARY.md) for providers and public
  value contracts; exact callable signatures come from generated documentation.
- [Generated-module ABI](docs/ABI.md) and [runtime evolution](docs/RUNTIME_EVOLUTION.md)
  for current and proposed host contracts.
- [Runtime conformance](docs/PORT_CONFORMANCE.md) and [measured baselines](docs/BASELINES.md)
  for what repository verification proves.
- [Porting campaign audit](docs/PORTING_CAMPAIGN_AUDIT.md) for durable findings
  from real migration attempts and maintained runtime evidence.
- [Active roadmap](TODO.md) for prioritized remaining work.
- [VS Code development](editors/vscode/DEVELOPMENT.md) for packaging and
  extension-host tests.

Run the complete repository check before committing compiler or tooling work:

```console
cargo xtask check
```

It checks formatting and Clippy, runs Rust and VS Code tests, compiles and
validates debug/release modules, and executes the maintained host-runtime
fixtures. Generated artifacts remain under ignored build directories.
It also renders the documentation site in memory and rejects broken pages,
links, or anchors. `cargo xtask docs` writes a local preview to the ignored
`target/generated-docs` directory; CI regenerates and publishes that output.

## Current product boundaries

SplitScript already supports native process attachment, typed memory paths,
transactional state snapshots, settings, timer actions and observers, failure
and optional values, async discovery, closures and iterators, records and enums,
strings and collections, layouts, source-defined Unity schemas, and typed
emulator providers. The compiler-owned reference is the canonical source for
exact symbols, signatures, effects, runtime availability, and examples.

The compiler emits a Core WebAssembly GC module matching the existing Auto
Splitting Runtime host calls. It does not install that module, configure a timer,
or choose a host UI; load the generated `.wasm` through the host's normal local
module workflow. Older host engines that disable WebAssembly GC cannot
instantiate it.

The repository also contains ports and deterministic runtime fixtures used as
implementation evidence. They are not the user guide, are not compatibility
promises, and should not be treated as templates for new scripts.

[latest-vsix]: https://github.com/CryZe/SplitScript/releases/download/latest/splitscript-latest.vsix
