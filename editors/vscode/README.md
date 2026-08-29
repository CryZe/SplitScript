# SplitScript for Visual Studio Code

Write statically typed autosplitters with diagnostics, completion, navigation,
documentation, formatting, and one-click WebAssembly builds. The extension
contains the SplitScript compiler and language server, so it does not require a
separate native executable.

## Start a script

1. Install the supplied `splitscript-*.vsix` with **Extensions: Install from
   VSIX**.
2. Open a folder and create a saved file ending in `.split`.
3. Run **SplitScript: Open Documentation** and open **Getting started**. That
   compiler-checked guide introduces process attachment, a typed setting, one
   memory field, `old` / `current` snapshots, and the first timer decision.
4. Use **SplitScript: Start Debug Watch** while editing. The first build happens
   immediately and later saves rebuild the script.
5. Use **SplitScript: Build Release** when the module is ready to distribute.

Both build commands create a `.wasm` file beside the source file. For example,
`game.split` produces `game.wasm`. A failed or superseded build leaves the last
successful module intact.

SplitScript generates a module for the Auto Splitting Runtime ABI. Selecting or
installing that module is handled by the autosplitting host, not this extension.
Use the host's normal local-Wasm workflow to load the generated file.

## Commands

| Command | Result |
| --- | --- |
| **SplitScript: Open Documentation** | Opens the compiler-owned reference index beside the active script. |
| **SplitScript: Open Documentation for Current Symbol** | Opens the exact language or standard-library page for the symbol at the caret. Assign it a shortcut through Keyboard Shortcuts for quick reference navigation. |
| **SplitScript: Search Documentation** | Searches symbols, concepts, signatures, summaries, and migration terms. |
| **SplitScript: Start Debug Watch** | Saves and builds the active script with the debug profile, then rebuilds it after later saves. |
| **SplitScript: Stop Debug Watch** | Stops the watcher shown in the status bar. |
| **SplitScript: Build Release** | Saves and performs one optimized build of the active script. |
| **SplitScript: Restart Language Server** | Replaces the language-service worker without reloading the editor window. |

Documentation and build commands are available from the Command Palette. The
documentation and build/watch actions also appear in the `.split` editor title;
symbol documentation and build/watch actions are in the editor context menu.

## Language support

The extension provides:

- diagnostics with related source locations and quick fixes;
- context-sensitive completion and signature help;
- hover, inferred-type hints, semantic highlighting, and document symbols;
- go to definition, references, rename, and selection ranges;
- whole-document formatting, including Format on Save;
- compiler-owned language, lifecycle, migration, and standard-library pages.

The language server and build worker use separate compiler instances. A long or
failed build therefore does not replace the language server. Source and output
files are accessed through the VS Code workspace filesystem, including in
virtual workspaces.

## Requirements and current limits

- VS Code 1.125 or newer is required.
- The extension supports desktop and browser extension hosts.
- Building requires a saved `.split` resource and write access beside it.
- Generated modules require an autosplitting host with WebAssembly GC enabled.
  Older host engines that disable WebAssembly GC cannot instantiate them.
- The extension compiles and documents the autosplitter; it does not configure a
  timer, install a host component, or select the resulting `.wasm` file.
- This early package is distributed as a VSIX rather than through the Visual
  Studio Marketplace.

## Troubleshooting

If a build fails, open **SplitScript Compiler** in the Output panel and inspect
the source diagnostics in **Problems**. Fixing the source and saving it is
enough to retry an active debug watch.

If hover, completion, highlighting, and navigation all stop together, run
**SplitScript: Restart Language Server**. If the problem is reproducible, keep
the smallest source that triggers it and the language-server output so it can be
reported without losing the failing state.

If a build targets the wrong file, make sure the intended `.split` editor is
active when starting the command. Untitled files prompt for a save location;
the resulting saved document is the one compiled.

Contributor setup, packaging, worker architecture, and web-host testing live in
the [extension development guide](https://github.com/CryZe/SplitScript/blob/master/editors/vscode/DEVELOPMENT.md).
