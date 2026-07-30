# Generated WebAssembly ABI

SplitScript targets the `env` imports used by the ASL v2 prototype and the
LiveSplit sandboxed Auto Splitting Runtime.

This is an internal compiler/runtime contract, not part of the source-language
standard library or its editor documentation. `src/abi.rs` is the source of
truth for import IDs, Wasm signatures, ownership, lifetime rules, effects, and
summaries. The compiler emits its import section from that catalog, and the test
suite verifies the table below against the catalog renderer.

## Exports

| Export | Type | Purpose |
| --- | --- | --- |
| `_start` | `() -> ()` | Allocate GC state snapshots and register settings |
| `update` | `() -> ()` | Host ABI polling entry point; drives SplitScript lifecycle blocks and timer actions |
| `memory` | one-page linear memory | Static UTF-8 strings and process-read scratch space |

## Imports

| Import | WebAssembly type |
| --- | --- |
| `timer_get_state` | `() -> i32` |
| `timer_start` | `() -> ()` |
| `timer_split` | `() -> ()` |
| `timer_reset` | `() -> ()` |
| `timer_set_game_time` | `(i64, i32) -> ()` |
| `timer_pause_game_time` | `() -> ()` |
| `timer_resume_game_time` | `() -> ()` |
| `timer_set_variable` | `(i32, i32, i32, i32) -> ()` |
| `runtime_set_tick_rate` | `(f64) -> ()` |
| `process_attach` | `(i32, i32) -> i64` |
| `process_detach` | `(i64) -> ()` |
| `process_is_open` | `(i64) -> i32` |
| `process_read` | `(i64, i64, i32, i32) -> i32` |
| `process_get_module_address` | `(i64, i32, i32) -> i64` |
| `process_get_module_size` | `(i64, i32, i32) -> i64` |
| `runtime_print_message` | `(i32, i32) -> ()` |
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

The compiler currently imports the complete core table even if a particular
source file does not use every action. This makes the ABI deterministic and easy
to inspect. Dead-import elimination can be added later.

`_start` registers the complete settings GUI, including nested titles,
tooltips, choices, and file filters. Every exported `update` call loads a
settings map before running `onDetached` or attached user code, decodes it into typed GC/global values,
and frees all temporary host handles. Choice strings become payloadless enum
variants and selected paths become GC strings. The preceding tick remains
available as `oldSettings`.

## GC types

Type index 0 is the source-specific state struct. Eight- and sixteen-bit fields
use packed GC fields; wider numbers and references use their native value
types. The remaining built-in recursive-group types are `Duration` (1),
`String` (2), `Module` (3), `UnityModule` (4), `UnityImage` (5), `UnityClass`
(6), `UnityField` (7), and the attach continuation frame (8). User records,
enums, and arrays follow at index 9. String backing arrays are internally
mutable so decoders and formatters can construct dynamically sized values, but
the source language exposes strings as immutable values.

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

Every state refresh is transactional. Reads populate a new GC state object;
the runtime commits it as `current` and moves the prior object to `old` only if
all primitive reads succeeded. A failed read skips the complete watcher tick,
including user `whileAttached` code and timer actions.

The module includes a `splitscript` custom section identifying compiler version,
GC target, and host ABI.
