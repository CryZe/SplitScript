# Tiberian Sun autosplitter port

This maintained port is based on `TiberianSun.asl` from the reviewed corpus.
It exercises the new ASCII normalization operation against the source's actual
localized mission-completion logic rather than treating a compile probe as
behavioral evidence.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Attach to ASL's extensionless `game` process | `state "game.exe"` for the current Windows host contract |
| Remove loads when `isPlaying` is zero | Original `isLoading` predicate |
| Start on the vanilla or Firestorm menu-to-game transition | Original menu and game-state edge |
| Lowercase localized splash text | `toAsciiLowerCase()` with explicit ASCII semantics |
| Match English, German, and French completion text | Raw UTF-8 byte inspection at offsets 7 and 11 |
| Match Spanish `MISION CUMPLIDA` | Lowercase `c` at byte offset 7 |

The source uses ASL `string20`, whose runtime guesses UTF-16LE from the second
byte. The known splash identifiers used by this script are ASCII, so the port
chooses `utf8(20)` explicitly. This makes the memory contract auditable and
keeps byte offsets equivalent to the source's .NET character indexes for the
supported patterns. A future non-ASCII localization would require revisiting
that evidence rather than silently relying on these offsets.

`byteAt` is fallible because an offset can be outside the decoded string. Both
accesses therefore return `false` on unexpectedly short text instead of
trapping. The source patterns are known ASCII, so inspecting their raw UTF-8
bytes avoids allocating one-byte strings on each matching transition.

## Runtime status

The deterministic host fixture covers the exact `.exe` process/module names,
64-bit pointer traversal, a full-bound 20-byte English string without a NUL,
a shorter NUL-terminated Spanish string, a rejected non-completion message,
vanilla start, loading pause/resume order, two splits, and detach.
