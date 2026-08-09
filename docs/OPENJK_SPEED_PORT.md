# OpenJK Speed autosplitter port

This maintained port is based on `OpenJK-Speed.asl` from the reviewed ASL
corpus. It is repository-owned migration evidence rather than a compile-only
translation: `tests/openjk_speed_runtime.mjs` executes the generated WebAssembly
against deterministic process, memory, timer, detach, and reattach behavior.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Attach to `openjk_sp.x86` | Exact process identity in the `state` declaration |
| Read `isActive` and the 30-byte map path | Typed `bool` plus an explicit bounded UTF-8 state field |
| Start on `maps/yavin1b.bsp` | `start` action with the original activity guard |
| Reset when the inactive game returns to the opening map | `reset` action with the original old/current transition |
| Remove loads while the game is inactive | `isLoading` action |
| Ignore empty and academy map transitions | Early predicates in `split` |
| Split only once for every other map | One global `Set<String>` retained across ticks |
| Reinitialize the ASL `vars.mapList` on attachment | `onAttach` clears the existing set without allocating every tick |
| Clear visited maps at the stable opening map | The original side effect remains in the `reset` action |

The ASL `string30` values used here are ASCII map identifiers, so choosing the
language's explicit UTF-8 decoder preserves their bytes without relying on
ASL's encoding auto-detection. The process name already contains its native
`.x86` suffix and does not exercise the unresolved cross-platform
extensionless-name policy.

## Runtime status

The host fixture proves the bounded 30-byte read, start/reset/load action
ordering, ignored-map behavior, duplicate suppression, reset-block set clearing
on the stable opening map, and per-process clearing after detach and reattach.
It uses the ordinary single-process lifetime and 64-bit host addresses; the
script itself contains no pointer traversal, settings, unsupported-build
selection, or tick-rate change to validate.
