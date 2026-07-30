# SplitScript for Visual Studio Code

This extension associates `.split` files with SplitScript and connects VS Code
to the `splitls` language server. Formatting, diagnostics, completion, hover,
signature help, semantic highlighting, navigation, rename, document symbols,
and quick fixes are provided by the Rust compiler.

## Build workflows

The extension exposes two explicit workflows from the editor title, context
menu, and Command Palette:

- **SplitScript: Start Debug Watch** starts `splitc watch` in the debug profile.
  It compiles immediately and recompiles the neighboring `.wasm` whenever the
  source file is saved. A status-bar indicator remains visible and can be
  clicked to stop the watcher.
- **SplitScript: Build Release** performs a one-shot release build intended for
  the final module. It saves pending edits, reports progress, and offers to
  reveal the resulting `.wasm` file.

Starting a release build while debug watch is active first offers to stop the
watcher, preventing a later debug rebuild from overwriting the release module.
Both workflows stream output to **SplitScript Compiler**. Failed builds preserve
the last successful Wasm module.

The client discovers `splitc` beside a configured `splitls`, in the packaged
extension, in this repository's debug target, or on `PATH`;
`splitScript.compiler.path` overrides that discovery.

## Language server discovery

The client tries these locations in order:

1. `splitScript.server.path` from VS Code settings;
2. `server/splitls` inside a packaged extension;
3. `target/debug/splitls` in this repository during extension development;
4. `splitls` from `PATH`.

Run `cargo build --bins` before launching the development extension, or set the
server and compiler paths to other builds. Use **SplitScript: Restart Language
Server** after replacing the server executable or changing its configuration.

## Development

From this directory:

```console
npm install
npm run check
npm run compile
```

When the full repository is open in VS Code, the checked-in **Run SplitScript
Extension** launch configuration compiles both the Rust server and TypeScript
client before opening an Extension Development Host.
