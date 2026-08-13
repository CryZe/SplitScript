# Nioh RTA No Load autosplitter port

This maintained port is based on `NiohRTANoLoad.asl` from the reviewed ASL
corpus. It covers all three builds named by the source instead of treating the
campaign's compiling translation as behavioral evidence.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Attach to ASL's extensionless process name `Nioh` | `state "Nioh.exe"` for the current Windows host contract |
| Select versions 1.21.04, 1.21.05, and 1.21.06 by main-module size | Three named layouts selected in `onAttach` |
| Poll at 29 Hz | `tickRate { attached: 29 }` lifecycle policy |
| Remove loads while the mission timer is unchanged and the game is off-map | Original `isLoading` predicate |
| Read the pointer paths of each build | Typed `u8` and `f32` state fields using 64-bit process traversal |

Versions 1.21.05 and 1.21.06 intentionally remain separate layouts despite
sharing addresses: their distinct module sizes are part of the source's
supported-build contract and remain visible in tooling and diagnostics.

The source defaulted every unknown module size to the 1.21.06 addresses. The
maintained port deliberately tightens that unsafe fallback: it reports the
unsupported size and awaits process closure without polling state. This avoids
silently reading arbitrary memory after a game update. The declarative policy
restores the ordinary 1 Hz cadence on detach.

## Runtime status

The deterministic host fixtures execute all three supported layouts and an
unsupported build. They verify the required `Nioh.exe` process identity,
module-size selection, 64-bit pointer reads, direct and indirect state fields,
29 Hz attach and 1 Hz detach cadence, load pause/resume order, persistent state
when one field read fails while another succeeds, inert unsupported builds,
and process detach. The source has no settings, start, split, reset, or game-
time action to validate.

The `.exe` translation is deliberately local to this Windows port. Whether a
future cross-platform runtime should normalize ASL-style names is still an
open host-contract question; the compiler does not warn about extensionless
names while valid Linux and macOS process names remain indistinguishable from
an accidental Windows omission.
