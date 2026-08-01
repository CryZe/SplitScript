# Lunistice port parity

The authoritative source for this port is
`C:\Projekte\lunistice-auto-splitter\src\lib.rs`. The SplitScript port is
[`examples/lunistice.split`](../examples/lunistice.split).

## Feature mapping

| Rust autosplitter behavior | SplitScript implementation |
| --- | --- |
| Attach to `Lunistice.exe`, then `Lunistice-Demo.exe` | Ordered process list in `state` |
| Cancel initialization when the process closes | Generated process-lifetime attach continuation |
| Unity IL2CPP V2020 registration discovery | `await Unity.il2cpp(2020)` using ASR signatures |
| `Assembly-CSharp`, `GameManager`, and `Timer` lookup | Awaitable image/class metadata APIs |
| Original/DLC GameManager binding race | `fieldAny(["currentLevel", "_currentScene"])` and alternative game-state fields |
| C# property backing fields | Transparent `<Name>k__BackingField` matching |
| `Instance` / legacy `_instance` singleton race | `staticInstance(["Instance", "_instance"])` |
| Derived IL2CPP class field bindings | Discovered offsets stored once and consumed by expression-backed state fields |
| `Watcher<GameManager>`, `Watcher<Timer>` | Transactional GC `current`/`old` snapshots; failed reads skip the complete tick |
| Adjacent minutes, seconds, and hundredths reads | One naturally laid-out `LevelTimeParts` record read and local GC deserialization |
| DLC managed scene name | Bounded UTF-16 `process.readManagedString` with surrogate-pair decoding |
| Points, resets, level time, level/scene, character runtime variables | GC string formatting plus `setVariable` |
| 1 Hz detached polling and 120 Hz attached polling | `setTickRate` calls in `onDetached` and `onAttach`; process close returns to 1 Hz immediately |
| Auto-start when the in-game timer begins in the first level | `timerStopped` transition plus base level / DLC Shrine checks |
| Runner-started timer reset of accumulated state | `timer.state()` transition tracking in `whileAttached` |
| Game-time accumulation across level-clock rollovers | `runTimeSeconds` plus `Duration.fromSeconds` |
| Reset on a rollover before leaving the first level | `reset` action |
| Results and final-level-to-credits splits | `split` action with both original predicates |
| Permanently paused LiveSplit game time with explicit game time | `isLoading { return true }` plus `gameTime` |
| Base and DLC character labels | `characterName` mapping with all Rust variants |
| No user-configurable settings in the Rust splitter | No settings declared; the separate `examples/lso_desktop_settings.split` port exercises the complete live settings API |

## Verification

`tests/lunistice_runtime.mjs` constructs synthetic but structurally accurate
V2020 IL2CPP assembly, image, class, field, static-table, singleton, and managed
string data. It executes the generated WebAssembly GC module in both base and
DLC configurations and verifies:

- class-layout and executable fallback selection;
- automatic and runner-initiated starts;
- Results and credits splits;
- first-level rollback reset;
- accumulated game time after a level-time rollover;
- all runtime variable values and character/level formatting;
- 1/120/1 Hz detached, attached, and process-close tick-rate changes;
- atomic suppression of a deliberately torn process-read tick.
- a single 12-byte host read for the three adjacent level-time components.

Run the complete verification with:

```console
cargo test
cargo clippy --all-targets -- -D warnings
cargo run --bin splitc -- examples/lunistice.split -o target/lunistice.wasm
wasm-tools validate --features all target/lunistice.wasm
node tests/lunistice_runtime.mjs target/lunistice.wasm
node tests/lunistice_runtime.mjs target/lunistice.wasm --dlc
```
