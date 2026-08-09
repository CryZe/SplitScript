<!-- Generated from the compiler-owned migration catalog. -->
# ASL migration capability index

This index maps common source-language concepts to canonical SplitScript APIs and patterns. It does not redeclare the standard library.

| Foreign concept | Source | Status | SplitScript direction |
| --- | --- | --- | --- |
| `declaration.let` — Variable declarations | C#, JavaScript | Supported directly | Use one inferred `let` declaration; SplitScript has no const/let split. Canonical targets: `let`. |
| `value.none` — Absent optional values | JavaScript | Supported directly | `None` is SplitScript's zero-sized unit value and the absent side of an option. Canonical targets: `None`. |
| `declaration.function` — Function declarations | C#, JavaScript | Supported directly | Functions and methods use the `fn` declaration keyword. Canonical targets: `fn`. |
| `type.string` — String type | C# | Supported directly | The immutable UTF-8 string type is named `String`. Canonical targets: `String`. |
| `string.ascii-lowercase` — ASCII string lowercasing | C# | Supported directly | Use `toAsciiLowerCase` when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode lowercasing. Canonical targets: `String.toAsciiLowerCase`. [Recipe](ASL_PORTING.md#c-string-operations). |
| `string.numeric-parse` — Numeric string parsing | C# | Supported directly | Replace static Parse/TryParse calls and output parameters with fallible `text.parse()` and ordinary Result handling. Canonical targets: `String.parse`. [Recipe](ASL_PORTING.md#c-string-operations). |
| `type.duration` — Timer durations | C# | Supported directly | Use `Duration` instead of C#'s `TimeSpan`. Canonical targets: `Duration`. |
| `type.fixed-width-number` — Fixed-width numeric types | C# | Supported directly | Memory-facing numbers use explicit signedness and bit widths. Canonical targets: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`. |
| `asl.state.string-n` — Bounded native stringN state | ASL | Use a typed pattern | Use an explicitly decoded state path such as `as utf8(50)`; choose the encoding from evidence. Canonical targets: `state`. [Recipe](ASL_PORTING.md#bounded-native-stringn-state). |
| `asl.state.version-label` — Version-labelled state blocks | ASL | Use a typed pattern | Use named layouts in one state block and return the selected layout from `onAttach`. Canonical targets: `state`. [Recipe](ASL_PORTING.md#version-labelled-asl-states). |
| `asl.memory.deep-pointer` — DeepPointer | ASL | Supported directly | Use typed state paths for polled fields or `process.follow` for discovered paths. Canonical targets: `state`, `Process.follow`. |
| `asl.process.identity` — Attached process identity | ASL | Supported directly | Use `process.name()` to read the exact process candidate that matched during attachment; use module metadata when the executable name alone does not identify a build. Canonical targets: `Process.name`. [Recipe](ASL_PORTING.md#attached-process-identity). |
| `asl.state.memory-watcher` — MemoryWatcher | ASL | Use a typed pattern | Declare polled memory in `state`; use a trailing field `if` with `value` and return `Err(message)` when a transient candidate should retain its last accepted value. Canonical targets: `state`. [Recipe](ASL_PORTING.md#retaining-the-last-accepted-field-value). |
| `asl.lifecycle.startup` — startup lifecycle block | ASL | Use a typed pattern | Use settings and global declarations for data, then `setup` for remaining process-independent startup statements. Canonical targets: `setup`. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.lifecycle.init` — init lifecycle block | ASL | Use a typed pattern | Use `onAttach` for pre-poll process discovery; legacy post-refresh snapshot work needs a guarded first attached tick. Canonical targets: `onAttach`. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.lifecycle.update` — update lifecycle block | ASL | Use a typed pattern | Use `whileAttached` for ordinary post-refresh work; ASL's false control result has no exact equivalent yet. Canonical targets: `whileAttached`. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.lifecycle.exit` — exit lifecycle block | ASL | Use a typed pattern | Use guarded `onDetached` cleanup because it also runs before the first attachment. Canonical targets: `onDetached`. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.lifecycle.shutdown` — shutdown lifecycle block | ASL | Planned | Exact script teardown needs the planned host shutdown notification; `onDetached` is not equivalent. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.timer.events` — timer event handlers | ASL | Planned | Simple start transitions can be reconstructed in `whileAttached`; exact ordered start, split, and reset events need host support. [Recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks). |
| `asl.settings.dynamic-lookup` — Dynamic settings lookup | ASL | Supported directly | Declare an exact string key with `key "..."`, then use `settings.enabled(key)` or `oldSettings.enabled(key)` for boolean settings. Choice and file settings remain statically typed. Canonical targets: `settings`, `oldSettings`. |
| `asl.settings.finite-family` — Finite startup-generated settings | ASL | Supported directly | Use a compile-time settings family for bounded integer-keyed booleans; it lowers to ordinary declarations and remains available through `settings.enabled(key)`. Canonical targets: `settings family`. [Recipe](ASL_PORTING.md#finite-settings-families). |
| `asl.state.mutable-current` — Assignments to current | ASL | Planned | Snapshots stay immutable; a typed retain-last-valid normalization pattern is planned. Canonical targets: `current`. |
| `asl.runtime.refresh-rate` — refreshRate | ASL | Supported directly | Call `setTickRate` on the lifecycle transitions where the polling rate changes. Canonical targets: `setTickRate`. |
| `asr.emulator.gba` — GBA emulator attachment | Rust | Supported directly | Use `state GBA`; the `gba` root reads normalized emulated addresses. Canonical targets: `GBA`. |
