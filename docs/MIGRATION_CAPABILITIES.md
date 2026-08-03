<!-- Generated from the compiler-owned migration catalog. -->
# ASL migration capability index

This index maps common source-language concepts to canonical SplitScript APIs and patterns. It does not redeclare the standard library.

| Foreign concept | Source | Status | SplitScript direction |
| --- | --- | --- | --- |
| `declaration.let` — Variable declarations | C#, JavaScript | Supported directly | Use one inferred `let` declaration; SplitScript has no const/let split. Canonical targets: `let`. |
| `value.none` — Absent optional values | JavaScript | Supported directly | `None` is SplitScript's zero-sized unit value and the absent side of an option. Canonical targets: `None`. |
| `declaration.function` — Function declarations | C#, JavaScript | Supported directly | Functions and methods use the `fn` declaration keyword. Canonical targets: `fn`. |
| `type.string` — String type | C# | Supported directly | The immutable UTF-8 string type is named `String`. Canonical targets: `String`. |
| `type.duration` — Timer durations | C# | Supported directly | Use `Duration` instead of C#'s `TimeSpan`. Canonical targets: `Duration`. |
| `type.fixed-width-number` — Fixed-width numeric types | C# | Supported directly | Memory-facing numbers use explicit signedness and bit widths. Canonical targets: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`. |
| `asl.state.string-n` — Bounded native stringN state | ASL | Use a typed pattern | Use an explicitly decoded state path such as `as utf8(50)`; choose the encoding from evidence. Canonical targets: `state`. [Recipe](ASL_PORTING.md#bounded-native-stringn-state). |
| `asl.state.version-label` — Version-labelled state blocks | ASL | Use a typed pattern | Use named layouts in one state block and return the selected layout from `onAttach`. Canonical targets: `state`. [Recipe](ASL_PORTING.md#version-labelled-asl-states). |
| `asl.memory.deep-pointer` — DeepPointer | ASL | Supported directly | Use typed state paths for polled fields or `process.follow` for discovered paths. Canonical targets: `state`, `Process.follow`. |
| `asl.state.memory-watcher` — MemoryWatcher | ASL | Use a typed pattern | Declare polled memory in `state`; use per-field `normalize` when a transient candidate should retain its last accepted value. Canonical targets: `state`, `normalize`. [Recipe](ASL_PORTING.md#retaining-the-last-accepted-field-value). |
| `asl.timer.on-start` — timer.OnStart | ASL | Use a typed pattern | Observe the `timer.state()` transition in `whileAttached` and reset run-scoped script state there. Canonical targets: `whileAttached`, `timer.state`. [Recipe](ASL_PORTING.md#run-scoped-one-shot-splits). |
| `asl.lifecycle.exit` — exit game-time cleanup | ASL | Supported directly | Use guarded `onDetached` cleanup and `timer.pauseGameTime()`. Canonical targets: `onDetached`, `timer.pauseGameTime`. [Recipe](ASL_PORTING.md#process-exit-game-time-cleanup). |
| `asl.settings.dynamic-lookup` — Dynamic settings lookup | ASL | Supported directly | Declare an exact string key with `key "..."`, then use `settings.enabled(key)` or `oldSettings.enabled(key)` for boolean settings. Choice and file settings remain statically typed. Canonical targets: `settings`, `oldSettings`. |
| `asl.state.mutable-current` — Assignments to current | ASL | Planned | Snapshots stay immutable; a typed retain-last-valid normalization pattern is planned. Canonical targets: `current`. |
| `asl.runtime.refresh-rate` — refreshRate | ASL | Supported directly | Call `setTickRate` on the lifecycle transitions where the polling rate changes. Canonical targets: `setTickRate`. |
| `asr.emulator.gba` — GBA emulator attachment | Rust | Supported directly | Use `state GBA`; the `gba` root reads normalized emulated addresses. Canonical targets: `GBA`. |
