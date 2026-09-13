# Installing SplitScript

Choose the Visual Studio Code extension unless an editor or automated workflow
specifically needs native command-line tools. Both paths use the same compiler,
formatter, diagnostics, documentation catalog, and language service.

| Path | Includes | Best for |
| --- | --- | --- |
| VS Code VSIX | Editor support, embedded compiler and language server, documentation, build commands, and the desktop autosplitter debugger | Writing and testing autosplitters without installing a compiler |
| Native `splitc` and `splitls` | Command-line compilation, formatting, documentation, watch builds, and a standard-input/output language server | Other editors, scripts, and build automation |

SplitScript is still an early moving language. The `latest` extension release
follows each verified `master` build, and source compatibility can change.

## Visual Studio Code extension

1. Download `splitscript-latest.vsix` from the [latest SplitScript
   release][latest-release].
2. In VS Code, run **Extensions: Install from VSIX** and select the downloaded
   file.
3. Open a folder, save a file with the `.split` extension, and run
   **SplitScript: Open Documentation**.

Installing a newer `splitscript-latest.vsix` updates the existing extension.
The package is not currently published through the Visual Studio Marketplace,
so VS Code does not automatically update it from the Marketplace.

The VSIX is batteries-included: do not install `splitc`, `splitls`, Rust, Node,
or a WebAssembly toolchain merely to use the extension. Release packaging
rejects a compiler Wasm module larger than 8 MiB or a complete VSIX larger than
12 MiB. The package contains native debugger bridges for Windows x64, Linux x64
and ARM64, and macOS Intel and Apple Silicon; only the bridge for the running
desktop host is loaded.

The language and build services run in separate workers with independent
embedded compiler instances. A long build therefore does not replace the
language server, although both compiler instances contribute to the extension's
memory use. The autosplitter runtime starts in another worker only while a
debug session is active. Runtime statistics retain a bounded 2,048-tick window,
and sidebar updates are coalesced to at most five per second. No fixed peak-RAM
guarantee has been established yet; report a reproducible source file and the
operation being performed if memory does not recover after a build, language
server restart, or stopped debug session.

### Supported extension hosts

- Desktop VS Code 1.125 or newer provides language tooling, builds, and the
  autosplitter debugger.
- Browser, remote, virtual, and untrusted workspaces retain language tooling and
  builds through the VS Code workspace filesystem.
- Running an autosplitter currently requires a trusted local desktop workspace.
- Native process debugging supports Windows x64, Linux x64 and ARM64, and macOS
  Intel and Apple Silicon. macOS process attachment remains subject to
  `task_for_pid` authorization and target code-signing policy.

Generated autosplitters are WebAssembly GC modules for the Auto Splitting
Runtime ABI. The extension creates the module but does not install it into a
timer. Load the neighboring `.wasm` file through the autosplitting host's normal
local-module workflow; hosts without WebAssembly GC support cannot instantiate
it.

### Builds, output, and recovery

**SplitScript: Build Release** saves the active script and atomically replaces
a neighboring release module: `game.split` produces `game.wasm`.
**SplitScript: Start Debug Watch** writes the same neighboring path with the
debug profile and rebuilds after later saves. Build diagnostics and progress
appear in the **SplitScript Compiler** Output channel. A failed, cancelled, or
superseded build leaves the previous successful module intact and removes its
temporary output.

If every language feature becomes unresponsive together, run **SplitScript:
Restart Language Server**. If a debug session fails, inspect **Auto Splitting
Runtime** in the Output panel, stop the session, and start it again after fixing
the reported source or host error. A stopped compiler worker rejects its pending
requests; a later command creates a fresh worker rather than reusing failed
state.

## Native command-line tools

Prebuilt native `splitc` and `splitls` archives are not published yet. Build
them from a repository checkout with the latest stable Rust toolchain:

```console
cargo build --profile max-opt --bin splitc --bin splitls
```

The executables are written to `target/max-opt` (`.exe` is added on Windows).
Keep them there, copy them to a directory already on `PATH`, or configure an
editor with their absolute paths. The `max-opt` profile is the distribution
profile; it favors compiler execution speed and executable size at the cost of
a slower initial build.

The complete repository check runs on Windows in CI. The Rust tools are not
intentionally tied to Windows, but source builds on other desktop targets are
early-user territory until native archives and the cross-platform smoke matrix
are published. This is separate from the five-platform debugger bridges inside
the VSIX.

Compile, watch, format, or browse documentation with:

```console
target/max-opt/splitc game.split -o game.wasm --profile release
target/max-opt/splitc watch game.split -o game.wasm
target/max-opt/splitc fmt game.split
target/max-opt/splitc docs
```

On Windows, use `target\max-opt\splitc.exe` instead. `splitc watch` performs an
initial build and then rebuilds after source changes. Compilation failures keep
the last successful output. `splitc fmt` follows applicable `.editorconfig`
formatting properties. Run `splitc --help` or a subcommand's `--help` for the
current command surface.

`splitls` is a Language Server Protocol server over standard input and output.
For another editor, configure the server command as the absolute path to
`target/max-opt/splitls` with no arguments and associate it with `.split` files.
Do not configure `splitls` as a TCP server, and do not parse its standard output
as logs: that stream carries framed LSP messages.

Native CLI builds produce the same WebAssembly GC and Auto Splitting Runtime ABI
as extension builds. They likewise do not install, register, or execute the
result in a timer host.

[latest-release]: https://github.com/CryZe/SplitScript/releases/tag/latest
