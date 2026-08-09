# Porting ASL to SplitScript

This guide records mappings proven by maintained, host-executed ports. It is
not a token-substitution table: ASL's dynamic state and C# runtime sometimes
need a typed SplitScript design rather than a literal translation.

The complete reference for the recipes below is
[`examples/a_plague_tale_innocence.split`](../examples/a_plague_tale_innocence.split).
`cargo xtask check` compiles that source in release mode and runs its Steam,
Epic, Xbox, and unsupported-build fixtures. The smaller
[`examples/arietta_of_spirits.split`](../examples/arietta_of_spirits.split)
example isolates bounded native strings, lifecycle transitions, and pause-menu
load removal. [`examples/aquanox.split`](../examples/aquanox.split) demonstrates
a nullable native-string watcher whose failed read is observable as `None`.

## Signed pointer offsets

ASL `DeepPointer` paths commonly contain negative offsets. Preserve them
directly; do not cast them through `u64`:

```splitscript
state "game.exe" {
    health: i32 at "game.dll", 0x120, -0x18;
}
```

The absolute root in `at 0xffff_ffff_ffff_fff0` remains an unsigned 64-bit
address. Module-relative roots and every offset after a dereference are signed
`i64`, and arithmetic wraps modulo the address space. The equivalent dynamic
APIs are `process.follow(base, offsets: [i64])`,
`address.offset(displacement)`, which accepts any integer width, and signed
`MemoryPath` offsets.

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
performs one bounded final read, stops at the first NUL byte, and rejects that
field's candidate when UTF-8 is invalid. It deliberately does not expose
`string50` as a type.

When the parser encounters a type-first field such as
`string50 map : 0x100`, it explains this distinction and offers a
**maybe-incorrect** rewrite to `map at 0x100 as utf8(50)`. The edit is not
preferred or machine-applicable because only the autosplitter author can verify
the target encoding.

That stricter malformed-input policy is equivalent for A Plague Tale's ASCII
map identifiers. For a known native UTF-16LE buffer, use
`field at address as utf16le(maxUtf16Units)` or
`process.readUtf16Le(address, maxUtf16Units)`. Both stop at the first NUL code
unit and replace malformed surrogate sequences. Do not use `readManagedString`
as a replacement: that method reads the object layout of a Unity managed
string rather than text bytes at the supplied address.

The maintained Arietta of Spirits port uses independent `utf8(128)` and
`utf8(8)` fields for its stage and pause-menu identifiers. Its host fixture
also proves the persistent-watcher rule: a failed stage-string read retains
that field while a successful pause flag from the same poll still advances.

## C# string operations

SplitScript methods use lower camel case, so C# `StartsWith` becomes
`startsWith`. For ASCII game identifiers, C# `ToLower()` becomes the more
explicit `toAsciiLowerCase()`:

```splitscript
let normalizedMap = current.map.toAsciiLowerCase()
```

This conversion changes only `A` through `Z` and preserves all other UTF-8
bytes. It is not culture-sensitive or full Unicode lowercasing. `slice` uses
UTF-8 byte offsets rather than .NET UTF-16 indices and fails when an offset is
out of range or inside a multibyte code point, so do not mechanically translate
`Substring` without checking the target data.

For a known ASCII character check, use `byteAt(byteIndex)` and compare the
returned `u8` with its byte value. For Unicode text, `codePointAt(byteIndex)`
returns a `u32` Unicode code point but still takes a UTF-8 byte offset. Neither
operation adopts C#'s or JavaScript's UTF-16 indexing:

```splitscript
let slash = current.map.byteAt(7) else return false
return slash == 0x2f
```

C# `Split` maps to fallible, lower-camel-case `split`. SplitScript matches one
exact non-empty delimiter from left to right and preserves leading, adjacent,
and trailing empty segments:

```splitscript
let fileParts = current.levelPicture.split(".") else []
let levelParts = fileParts[0].split("_") else []
```

The empty delimiter is an error rather than a request to split UTF-8 bytes.
[`examples/operation_matriarchy.split`](../examples/operation_matriarchy.split)
uses this mapping to parse names such as `01_02_1.dds`; its host fixture proves
the resulting start, reset, split, and loading transitions.

C# `Int32.Parse`, `Double.Parse`, and their fixed-width relatives map to
`text.parse()`. Put the SplitScript numeric type at the receiving boundary and
let inference flow backward:

```splitscript
let percentage: f64 = current.percentageText.parse() else 0.0
```

Unlike an exception-catching `Parse` call, failure is an ordinary `Result`.
Use `else` for a fallback, `?` to propagate the error from a function or state
field, or `match` when failure needs its own behavior. C# `TryParse` therefore
does not need an output parameter. Parsing consumes the complete ASCII decimal
string and rejects whitespace, separators, and trailing text. Float targets
accept case-insensitive `NaN`, `inf`, and `Infinity`; decimal overflow produces
infinity and underflow produces zero, while integer overflow remains an error.
Float conversion is correctly rounded directly to `f32` or `f64` and does not
inherit C# culture settings.

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

Expression-backed fields are persistent watcher values. The initial snapshot
waits for every required field to succeed together. Afterwards, a failed `T!`
retains that field's last accepted value while successful siblings advance. If
several values must advance atomically, read them as one record- or array-valued
state field. For a field that is semantically absent on some ticks, declare
`T?` and convert just that read:

```splitscript
state "game.exe" {
    requiredLevel: u32 at "game.exe", 0x1000;
    optionalBonus: u32? at "game.exe", 0x2000;
}
```

For a static pointer path, the explicit `T?` annotation maps a failed module
lookup, pointer traversal, final read, or decoder to a successfully accepted
`None`; success produces `Some(T)`. This differs from a required `T` field,
whose failed result retains the last accepted value after initialization. The
maintained Aquanox port uses `String? at ... as utf8(32)` because the original
ASL watcher becoming `null` is itself the manual level-end signal.

The same policy remains available to a discovered-address expression with
`process.read<T>(address).toOption()`. Prefer direct `T? at` syntax when the
path is static so the declaration shows both the memory layout and its absence
semantics in one place.

Pointer width is a property of traversal. Static `at` fields use the attached
process's native width. When a 64-bit host reads a PE32 or other 32-bit target,
construct an explicit path with
`base.memoryPath(offsets, finalOffset, PointerSize.Bit32)` and resolve it before
the final read. This keeps mixed-width discovery auditable without an `at32`
pseudo-keyword. See the maintained ABZÛ and Borderlands examples for the full
discovery and PE32 forms.

## Retaining the last accepted field value

Some ASL `update` blocks overwrite one newly read watcher with its old value to
filter a transient sentinel. Do not make `current` mutable. Reject that field's
candidate instead; its accepted value stays unchanged while unrelated fields
can advance.

Use an ordinary trailing `if` on that pointer-path field instead:

```splitscript
state "game.exe" {
    scene: i32 at "engine.dll", 0x1000 if value == 7 || value == 8 {
        Err("transient loading scene")
    } else {
        value
    };
    entities: i32 at "engine.dll", 0x2000;
}
```

`value` is the successfully read candidate and has the field's inferred type.
A plain value accepts the candidate; `Err(message)` rejects it. Before the
first snapshot, rejection leaves state uninitialized, so no fabricated old
value or stale value from another process is observable. Afterwards, the field
retains its accepted value and successful sibling fields continue to advance.

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

## Finite settings families

Prefer direct `settings.name` access when the setting is known statically. For
data tables whose entries select among declared boolean settings, give each
declaration its exact host-map string with `key "..."` and use
`settings.enabled(key)`. This remains boolean-only and returns false for an
unknown key; it is not a dynamically typed replacement for choice or file
settings. If the original settings have a boolean parent, gate the child result
explicitly; a quoted SplitScript heading is visual only. The complete A Plague
Tale example preserves its **All Chapters** parent semantics this way.

When legacy `startup` creates a bounded numbered family, declare it at compile
time rather than expanding dozens of source members or mutating the settings
map:

```splitscript
settings {
    "Levels" {
        for level in 2..=36 {
            `{level}` key `{level}`: true,
        },
    },
}
```

This registers the exact stable keys `"2"` through `"36"`. The generated
entries are intentionally available only through `settings.enabled(key)`; they
do not create artificial members such as `settings.level17`. Label and key
templates may interpolate only the range binding, and a documentation comment
on the family becomes every generated tooltip. See the maintained Drug Dealer
Simulator port for registration and runtime evidence.

Use a growable `Set<T>` only when the keys are genuinely discovered or
unbounded. A fixed 16-chapter route does not justify per-tick collection
allocation or a new compiler special case.

## Legacy ASL lifecycle blocks

The similarly shaped block names are not interchangeable. The original
LiveSplit component invokes them at different boundaries:

| ASL construct | Exact legacy timing | SplitScript direction |
| --- | --- | --- |
| `startup` | Once when the script is loaded, before process attachment | Put settings in `settings`, constant data in global initializers, and remaining process-independent statements in `setup`. |
| `init` | Once for each found process, after one legacy state refresh; a failure retries attachment initialization | Put suspending discovery and layout selection in `onAttach`. Code that truly requires the first `current`/`old` snapshot has no exact direct block yet. |
| `update` | After each refresh and before all timer decisions; `false` skips the remaining decisions for that tick | Put ordinary per-tick work in `whileAttached`. There is not yet an exact equivalent for the `false` control result. |
| `exit` | When the attached process exits | Use guarded `onDetached` cleanup as shown below. `onDetached` also runs once before the first attachment. |
| `shutdown` | When the script is disabled, reloaded, dropped, or LiveSplit exits | No exact host callback exists yet; do not approximate it with `onDetached`. |
| `timer.OnStart`, `timer.OnSplit`, `timer.OnReset` | LiveSplit timer events, which may be raised independently of this script's decision blocks | Reconstruct only simple observable transitions in `whileAttached`. Exact lossless events require the planned host contract. |

For example, process-independent ASL startup statements belong in `setup`, not
`onAttach`:

```splitscript
setup {
    setTickRate(30.0)
    print("Autosplitter loaded")
}
```

`setup` runs at the beginning of the module's first interruptible host update,
after settings are available, but cannot use `process`, `gba`, `current`,
`old`, `await`, or `retry`. A debug-watch replacement loads a new module and
therefore runs it again on that module's first update.

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
