# Porting ASL to SplitScript

This guide records mappings proven by maintained, host-executed ports. It is
not a token-substitution table: ASL's dynamic state and C# runtime sometimes
need a typed SplitScript design rather than a literal translation.

The complete reference for the recipes below is
[`examples/a_plague_tale_innocence.split`](../examples/a_plague_tale_innocence.split).
`cargo xtask check` compiles that source in release mode and runs its Steam,
Epic, Xbox, and unsupported-build fixtures.

## Bounded native `stringN` state

The original ASL runtime implements `stringN` by:

1. resolving the complete `DeepPointer` path;
2. reading exactly `N` bytes;
3. choosing UTF-16LE when the second byte is zero, otherwise UTF-8;
4. decoding with .NET replacement behavior; and
5. truncating at the first decoded NUL character.

For game identifiers known to contain valid ASCII or UTF-8, use a bounded UTF-8
decoder on the state field:

```splitscript
state "game.exe" {
    map at "game.exe", 0x123456, 0x20, 0x18 as utf8(50)
}
```

The number is a maximum byte count, not part of the field's type. The result is
an ordinary immutable `String`. SplitScript resolves every pointer offset first,
performs one bounded final read, stops at the first NUL byte, and rejects invalid
UTF-8 transactionally. It deliberately does not expose `string50` as a type.

When the parser encounters a type-first field such as
`string50 map : 0x100`, it explains this distinction and offers a
**maybe-incorrect** rewrite to `map at 0x100 as utf8(50)`. The edit is not
preferred or machine-applicable because only the autosplitter author can verify
the target encoding.

That stricter malformed-input policy is equivalent for A Plague Tale's ASCII
map identifiers. Do not use `readManagedString` as a replacement for native
UTF-16: that method reads the object layout of a Unity managed string. A future
port that genuinely stores native UTF-16 or depends on replacement decoding
should drive a distinct bounded decoder.

## Version-labelled ASL states

The second argument in `state("game.exe", "Steam")` is a layout label, not
another executable candidate. Co-locate layouts in one state and return the
selected generated variant from `onAttach`:

```splitscript
state "game.exe" {
    layout Steam {
        loading: bool at "engine.dll", 0x1000
    }

    layout Epic {
        loading: bool at "engine.dll", 0x2000
    }
}

onAttach {
    let executable = await process.mainModule()
    if executable.size == 10_000 {
        return StateLayout.Steam
    } else if executable.size == 20_000 {
        return StateLayout.Epic
    }
    await process.closed()
}
```

Detection is ordinary typed code and may use module size, `FileVersion`,
signatures, process identity, or discovered memory. `await process.closed()`
keeps an unsupported attachment inert without detaching and immediately
reattaching to the same process.

Named layouts currently expose one common field interface. When an ASL layout
omits a field because that feature is unavailable on a build, an honest typed
default can preserve the common interface when consumers already treat it as
unavailable. The A Plague Tale Xbox layout uses `cutsceneState: i32 = 0`, which
preserves the original absence of its ending trigger. Do not invent a default
when it could be mistaken for real game state; that case belongs in the planned
non-uniform layout design.

## Run-scoped one-shot splits

ASL frequently uses a `List<string>` or dictionary to prevent checkpoint loads
from splitting the same chapter twice. When the set of chapters is closed and
small, model it as an enum plus a typed bit set instead of introducing dynamic
string keys:

```splitscript
enum Chapter {
    Village
    Farm
}

let completedChapters: u32 = 0

fn chapterMask(chapter: Chapter) -> u32 {
    return match chapter {
        Chapter.Village => 1u32 << 0,
        Chapter.Farm => 1u32 << 1
    }
}
```

Detect the timer transition in `whileAttached`, clear the bit set, and mark the
starting map before `split` is evaluated. This reproduces an ASL `timer.OnStart`
handler without a separate event API. The generated update loop runs
`whileAttached` before timer-decision actions.

Static settings should likewise use typed enum matching rather than a dynamic
`settings[mapName]` lookup. If the original settings have a boolean parent,
gate the child result explicitly; a quoted SplitScript heading is visual only.
The complete A Plague Tale example preserves its **All Chapters** parent
semantics this way.

Use a growable `Set<T>` only when the keys are genuinely discovered or
unbounded. A fixed 16-chapter route does not justify per-tick collection
allocation or a new compiler special case.

## Process-exit game-time cleanup

ASL commonly pauses game time in `exit`. SplitScript's `onDetached` also runs
once at initial detached startup, so guard cleanup that should happen only
after a real attachment:

```splitscript
let attachedOnce = false

onAttach {
    attachedOnce = true
    // Perform discovery and return a layout when applicable.
}

onDetached {
    if attachedOnce {
        timer.pauseGameTime()
        attachedOnce = false
    }
}
```

Use `isLoading` for ordinary load removal. `timer.pauseGameTime()` and
`timer.resumeGameTime()` are explicit lifecycle tools, not a replacement for
that declarative action.
