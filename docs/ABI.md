# Generated WebAssembly ABI

SplitScript primarily targets the `env` imports used by the ASL v2 prototype
and the LiveSplit sandboxed Auto Splitting Runtime. Features that need a
standardized host facility may also use a narrowly scoped WASI import; the
monotonic `Instant` clock uses
`wasi_snapshot_preview1.clock_time_get`.

This is an internal compiler/runtime contract, not part of the source-language
standard library or its editor documentation. `src/abi.rs` is the source of
truth for import IDs, Wasm signatures, ownership, lifetime rules, effects, and
summaries. The compiler emits its import section from that catalog, and the test
suite verifies the table below against the catalog renderer.

Proposed host capabilities and limitations discovered during real ports are
tracked separately in [`RUNTIME_EVOLUTION.md`](RUNTIME_EVOLUTION.md). This file
describes only the contract that generated modules implement today.

## Exports

| Export | Type | Purpose |
| --- | --- | --- |
| `_start` | `() -> ()` | Initialize module state and run the source `setup` block |
| `update` | `() -> ()` | Host ABI polling entry point; drives SplitScript lifecycle blocks and timer actions |
| `memory` | dynamically sized linear memory | Named runtime scratch regions followed by immutable UTF-8/signature data and growable host-string staging |

## Imports

| Import | WebAssembly type |
| --- | --- |
| `timer_get_state` | `() -> i32` |
| `timer_current_split_index` | `() -> i64` |
| `timer_segment_splitted` | `(i64) -> i32` |
| `timer_start` | `() -> ()` |
| `timer_split` | `() -> ()` |
| `timer_skip_split` | `() -> ()` |
| `timer_undo_split` | `() -> ()` |
| `timer_reset` | `() -> ()` |
| `timer_set_game_time` | `(i64, i32) -> ()` |
| `timer_pause_game_time` | `() -> ()` |
| `timer_resume_game_time` | `() -> ()` |
| `timer_set_variable` | `(i32, i32, i32, i32) -> ()` |
| `runtime_set_tick_rate` | `(f64) -> ()` |
| `clock_time_get` | `(i32, i64, i32) -> i32` |
| `process_attach` | `(i32, i32) -> i64` |
| `process_detach` | `(i64) -> ()` |
| `process_is_open` | `(i64) -> i32` |
| `process_read` | `(i64, i64, i32, i32) -> i32` |
| `process_get_module_address` | `(i64, i32, i32) -> i64` |
| `process_get_module_size` | `(i64, i32, i32) -> i64` |
| `process_get_module_path` | `(i64, i32, i32, i32, i32) -> i32` |
| `process_get_path` | `(i64, i32, i32) -> i32` |
| `process_get_memory_range_count` | `(i64) -> i64` |
| `process_get_memory_range_address` | `(i64, i64) -> i64` |
| `process_get_memory_range_size` | `(i64, i64) -> i64` |
| `process_get_memory_range_flags` | `(i64, i64) -> i64` |
| `runtime_print_message` | `(i32, i32) -> ()` |
| `runtime_get_os` | `(i32, i32) -> i32` |
| `runtime_get_arch` | `(i32, i32) -> i32` |
| `user_settings_add_bool` | `(i32, i32, i32, i32, i32) -> i32` |
| `user_settings_add_title` | `(i32, i32, i32, i32, i32) -> ()` |
| `user_settings_add_choice` | `(i32, i32, i32, i32, i32, i32) -> ()` |
| `user_settings_add_choice_option` | `(i32, i32, i32, i32, i32, i32) -> i32` |
| `user_settings_add_file_select` | `(i32, i32, i32, i32) -> ()` |
| `user_settings_add_file_select_name_filter` | `(i32, i32, i32, i32, i32, i32) -> ()` |
| `user_settings_add_file_select_mime_filter` | `(i32, i32, i32, i32) -> ()` |
| `user_settings_set_tooltip` | `(i32, i32, i32, i32) -> ()` |
| `settings_map_load` | `() -> i64` |
| `settings_map_free` | `(i64) -> ()` |
| `settings_map_get` | `(i64, i32, i32) -> i64` |
| `setting_value_free` | `(i64) -> ()` |
| `setting_value_get_bool` | `(i64, i32) -> i32` |
| `setting_value_get_string` | `(i64, i32, i32) -> i32` |

The signed type of `timer_current_split_index` is a host encoding, not its
source-language type. `timer.currentSplitIndex()` maps every negative import
result to `None` and exposes every nonnegative result as `Some(u64)`.

The compiler plans imports from reachable operations. Every autosplitter gets
the small lifecycle baseline; optional facilities such as process reads,
settings, logging, and the monotonic clock are imported only when reachable
source needs them. Import identities and ordering remain deterministic through
the ABI catalog.

`_start` initializes globals and GC state, registers the complete settings GUI
including nested titles, tooltips, choices, and file filters, loads the initial
settings snapshot, and then invokes the source `setup` block exactly once.
The LiveSplit runtime retains this export during instantiation and calls it at
the beginning of the first controlled `update`, when its interrupt handle is
already available. `setup` is compiled as a synchronous `() -> ()` internal
function and cannot observe a process provider or state snapshot. Every
exported `update` call loads a settings map before running `onDetached` or
attached user code, decodes it into typed GC/global values,
and frees all temporary host handles. Choice strings become payloadless enum
variants and selected paths become GC strings. The preceding tick remains
available as `oldSettings`.

When process liveness fails, `update` detaches and clears the process handle,
provider-specific state, selected layout, ready flags, and process-lifetime
continuations. It then invokes `onProcessExit` exactly once, followed by
`onDetached`, and returns. The process-exit action is compiler-generated
lifecycle behavior and requires no additional host callback or ABI import.

These explicit frees are an implementation detail of the current C-shaped
`env` ABI, not a desired SplitScript ownership model. New host-owned collection
and value APIs should follow the GC-managed `externref` direction recorded in
`RUNTIME_EVOLUTION.md` instead of expanding the manual-handle surface.

## GC types

The compiler derives GC layouts and type indices from the reachable catalog and
source program; consumers must not rely on fixed numeric indices. Eight- and
sixteen-bit fields use packed GC fields, while wider numbers and references use
their native value types. Standard-library records such as `Duration` and
`Instant`, source records and enums, constructed arrays, wrapper types, state
snapshots, and an attach continuation frame are included only as required.
String backing arrays are internally mutable so decoders and formatters can
construct dynamically sized values, but the source language exposes strings as
immutable values.

Signature needles and masks are parsed by the compiler and stored in static
linear-memory data. A generated scanner reads the target module through
`process_read` in overlapping 4 KiB chunks. They overlap by the pattern length
minus one so matches spanning a page boundary are not lost.

An `onAttach` block lowers to an internal poll function `(process: i64) -> i32`.
It returns zero while an await barrier is pending and one when initialization is
complete. Its continuation frame program counter selects generated entry, poll,
and continuation states; successful polls redispatch within the same call.
The exported `update` loop checks process liveness before every poll
and consumes the lowered process-lifetime cancellation region on detach,
providing structured cancellation without exposing continuation management to
the source language.

When `onAttach` contains locals, the compiler adds a mutable GC continuation
struct. Its first field is the resume program counter and its remaining fields
hold the deterministic union of values live across individual suspension
points. Replacing this frame on process exit cancels the old continuation and
clears its state.

State initialization requires one poll in which every required field succeeds.
The resulting GC object initializes both `old` and `current`, then the compiler
invokes synchronous `onStateReady` once and returns without running
`whileAttached` or timer-decision actions. Later refreshes populate a new GC
state object field by field. Successful results advance, while failed results
copy that field from `current`; unrelated fields can therefore advance
independently. Detachment clears the ready flag, so a later attachment repeats
this initialization boundary.

The internal `whileAttached` function returns an `i32` continuation flag. Its
fallthrough default is one. An explicit source `false` returns zero, causing the
generated update export to return before reading timer state or invoking any
timer-decision action. The committed state and settings snapshots remain
advanced.

The module includes a `splitscript` custom section containing UTF-8 JSON with
the compiler package version, optional full Git revision, GC target, and host
ABI. The same compiler identity is reported by native frontends and the
embedded compiler service, so an artifact can be traced back to its producer.
