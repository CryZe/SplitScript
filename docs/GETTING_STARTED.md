# Getting started

This guide takes a new SplitScript file from its first declaration to a
WebAssembly autosplitter. It uses small, focused fragments rather than a
complete game autosplitter.

## Choose the extension or CLI

The Visual Studio Code extension is the shortest path. Install the supplied
`splitscript-*.vsix` with **Extensions: Install from VSIX**, open a folder, and
create a file such as `game.split`. The extension includes the compiler and
language server; it does not need a separate `splitc` installation.

When working from this repository instead, the native CLI exposes the same
compiler:

```console
cargo run --bin splitc -- game.split -o game.wasm --profile release
```

The project does not yet publish a Marketplace extension or native binary
release. A contributor can build a shareable VSIX with the repository's
**package SplitScript VSIX** task.

## Attach to the game

Every autosplitter declares one [`state`] provider. A native Windows game uses
the process name as the host reports it, including `.exe`:

```splitscript
state "game.exe" {
    level: u32 at 0x1234
}
```

The runtime waits for that process and reads `level` transactionally on every
attached tick. `0x1234` is only an illustrative address; replace it with an
address or pointer path verified for the game. Other operating systems use the
exact process identity reported by their host. Emulator and Unity games use
their typed providers instead of this native form.

## Add one setting

A boolean setting has a visible label, a source name, and a typed default:

```splitscript
# state "game.exe" {}
settings {
    /// Allows the level-change split to be disabled.
    "Split on level change" => splitOnLevelChange: true,
}
```

The documentation comment becomes the setting's tooltip. Script code reads the
current value through [`settings`].

## Compare snapshots and request a split

After the first complete state read, [`current`] contains the new snapshot and
[`old`] contains the preceding one. A [`split`] block returns a boolean timer
decision:

```splitscript
# state "game.exe" {
#     level: u32 at 0x1234
# }
# settings {
#     "Split on level change" => splitOnLevelChange: true,
# }
split {
    return settings.splitOnLevelChange && current.level != old.level
}
```

The compiler seeds [`old`] and [`current`] with the same first snapshot, so merely
attaching does not look like a level change. Failed state reads do not expose a
partially updated snapshot.

## Build while editing

With a `.split` editor active, use the buttons in the editor title or the
Command Palette:

- **SplitScript: Start Debug Watch** saves the file, builds immediately, and
  rebuilds after later saves. Click the watch status item or run
  **SplitScript: Stop Debug Watch** to stop it.
- **SplitScript: Build Release** saves the file and performs one optimized
  build for distribution.

Both commands write `game.wasm` beside `game.split`. A failed build preserves
the last successful module. The CLI equivalents are:

```console
splitc watch game.split -o game.wasm
splitc game.split -o game.wasm --profile release
```

The generated module targets the Auto Splitting Runtime ABI and requires a host
with WebAssembly GC enabled. SplitScript currently produces the module but does
not install or select it in a timer. Load `game.wasm` using the autosplitting
host's normal local-Wasm workflow; the exact selection and reload UI belongs to
that host.

## Use the first diagnostic

Diagnostics appear as editor underlines and in **Problems**. For example, a
misspelled state field is rejected at the use site:

```text
split {
    return current.levle != old.level
}
```

Hover the underline to read the diagnostic. When the compiler can perform the
change safely, **Quick Fix** offers a machine-applicable edit; otherwise its
related labels and help explain where the expected declaration came from and
what to change. The CLI renders the same source labels and help in the terminal.

## Find the next concept

**SplitScript: Open Documentation** opens this compiler-owned reference beside
the source editor. **SplitScript: Search Documentation** searches language,
standard-library, and migration concepts. **SplitScript: Open Documentation for
Current Symbol** opens the page for the symbol at the caret and can be assigned
a keyboard shortcut.

From the CLI, use `splitc docs`, `splitc docs search`, or a specific symbol such
as `splitc docs Process.read`. Continue with the [`state`] and [`settings`]
reference pages for native memory paths, then use the provider documentation
only when the target actually needs an emulator or Unity.
