# Star Wars: Battlefront II autosplitter port

This maintained port is based on `swbf2_loadremover_v2.asl` from the reviewed
ASL corpus. The repository runs `tests/battlefront_ii_runtime.mjs` against the
compiled WebAssembly in both supported settings configurations; it is not a
compile-only encoding probe.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Attach to `BattlefrontII` | Exact process identity in the `state` declaration |
| Read victory and loading integers | Explicit `i32` state fields at the original absolute addresses |
| Read the Galactic Conquest end screen | `utf16le(16)` reads exactly 16 little-endian UTF-16 code units |
| Split on entry to a Victory/Defeat screen | Original zero-to-nonzero transition |
| Optionally split on `ifs_freeform_end` | UTF-16 string transition gated by the stable `gc` setting key |
| Use Galactic Conquest loading rules in preference to Other | Ordered `if` / `else if` settings branches |
| Preserve both defaults and tooltips | Boolean settings with stable keys and documentation comments |

ASL's `string16` decoder selected an encoding heuristically. This field is
known to contain the 16-code-unit ASCII sentinel `ifs_freeform_end` in a
little-endian UTF-16 buffer, so the port makes that representation explicit.
Shorter fixture strings include a NUL terminator and ignored non-BMP data after
it; the sentinel fills all 16 code units and therefore requires no terminator.

The executable name is extensionless in the original source. The fixture proves
the current host's exact-name contract only; it does not settle the deferred
cross-platform process-name policy.

## Runtime status

The deterministic host fixture covers default and overridden live settings,
tooltip registration, both split paths, both loading rulesets, Galactic
Conquest precedence, a failed required UTF-16 refresh retaining its last value,
and process detach cleanup. The script has no pointer traversal, build layouts,
start/reset actions, or tick-rate changes to validate.
