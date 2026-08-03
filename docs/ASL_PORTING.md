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
    map at "game.exe", 0x123456, 0x20, 0x18 as utf8(50);
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
        loading: bool at "engine.dll", 0x1000;
    }

    layout Epic {
        loading: bool at "engine.dll", 0x2000;
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

Compatible fields declared by every named layout expose one common interface.
When a field is missing or the same name has a conflicting type, match the
generated `layout` value and access the field only in the corresponding arm.
The compiler gives each incompatible declaration its real physical type rather
than turning it into an option or inventing a default. An honest typed default
is still appropriate when consumers already define that value as unavailable;
the A Plague Tale Xbox layout uses `cutsceneState: i32 = 0` for exactly that
reason.

## Attach-time-discovered addresses

Keep discovery in `onAttach` and polling in the state declaration. Polling does
not begin until `onAttach` completes, so a global address initializer is never
observed when every completing path assigns the discovered address. An
unsupported build should await process closure instead of completing:

```splitscript
let loadingAddress: address = 0x0

state "game.exe" {
    loading: i32 = process.read(loadingAddress);
}

onAttach {
    let executable = await process.mainModule()
    if executable.size == 47_570_944 {
        loadingAddress = executable.address + 0x029020f4
    } else {
        print(`unsupported module size {executable.size}`)
        await process.closed()
    }
}
```

Expression-backed fields remain one atomic snapshot. Leave required reads as
`T!`; if any of them fails, `current` and `old` do not rotate and no lifecycle
action observes a partial update. For a field that is semantically absent on
some ticks, declare `T?` and convert just that read:

```splitscript
state "game.exe" {
    requiredLevel: u32 = process.read(levelAddress);
    optionalBonus: u32? = process.read<u32>(bonusAddress).toOption();
}
```

`toOption()` discards the read error, maps it to `None`, and lets the rest of a
valid transaction commit. Do not use it for a field whose failure invalidates
the snapshot.

Pointer width is a property of traversal. Static `at` fields use the attached
process's native width. When a 64-bit host reads a PE32 or other 32-bit target,
construct an explicit path with
`base.memoryPath(offsets, finalOffset, PointerSize.Bit32)` and resolve it before
the final read. This keeps mixed-width discovery auditable without an `at32`
pseudo-keyword. See the maintained ABZÛ and Borderlands examples for the full
discovery and PE32 forms.

## Retaining the last accepted field value

Some ASL `update` blocks overwrite one newly read watcher with its old value to
filter a transient sentinel. Do not make `current` mutable and do not convert
the sentinel into a failed state read: a failed read rejects every field in the
transaction, while the original script may still accept unrelated values from
that tick.

Use an ordinary trailing `if` on that pointer-path field instead:

```splitscript
state "game.exe" {
    scene: i32 at "engine.dll", 0x1000 if value == 7 || value == 8 {
        old
    } else {
        value
    };
    entities: i32 at "engine.dll", 0x2000;
}
```

`value` is the successfully read candidate and `old` is the last value
accepted for that field. Both are read-only and have the field's inferred
type. On the first successful poll after each attachment, both names contain
the candidate, so no stale value leaks across processes. Each field is
filtered independently and then the complete resulting snapshot commits
atomically.

The maintained
[`examples/aawcb.split`](../examples/aawcb.split) port uses this to retain its
scene during loading scenes 7 and 8 while the entity count continues to
advance. By contrast, an ASL `update` block that returns `false` does not roll
back state at all; it skips lifecycle decisions after the refresh. SplitScript
does not add a separate lifecycle concept for that behavior until a maintained
port demonstrates that ordinary field expressions and `whileAttached` cannot
represent the required result clearly.

## Run-scoped one-shot splits

ASL frequently uses a `List<string>` or dictionary to prevent checkpoint loads
from splitting the same chapter twice. When the set of chapters is closed and
small, model it as an enum plus a typed bit set instead of introducing dynamic
string keys:

```splitscript
enum Chapter {
    Village,
    Farm,
}

let completedChapters: u32 = 0

fn chapterMask(chapter: Chapter) -> u32 {
    return match chapter {
        Chapter.Village => 1u32 << 0,
        Chapter.Farm => 1u32 << 1,
    }
}
```

Detect the timer transition in `whileAttached`, clear the bit set, and mark the
starting map before `split` is evaluated. This reproduces an ASL `timer.OnStart`
handler without a separate event API. The generated update loop runs
`whileAttached` before timer-decision actions.

Prefer direct `settings.name` access when the setting is known statically. For
data tables whose entries select among declared boolean settings, give each
declaration its exact host-map string with `key "..."` and use
`settings.enabled(key)`. This remains boolean-only and returns false for an
unknown key; it is not a dynamically typed replacement for choice or file
settings. If the original settings have a boolean parent, gate the child result
explicitly; a quoted SplitScript heading is visual only. The complete A Plague
Tale example preserves its **All Chapters** parent semantics this way.

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
