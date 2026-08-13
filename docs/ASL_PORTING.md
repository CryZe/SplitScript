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
The [maintained Axiom Verge port](AXIOM_VERGE_PORT.md) combines dynamic UTF-16
event names with declared-setting membership, recursive legacy parent gating,
and optional-module platform detection.

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

SplitScript does not preserve that heuristic. Choose the encoding from the
game's memory layout. For identifiers known to contain valid ASCII or UTF-8,
use a bounded UTF-8 decoder on the state field:

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

For a known native UTF-16LE buffer, use a code-unit bound instead. An ASL
`string32` field reads 32 bytes, so its equivalent bound is 16 UTF-16 code
units:

```splitscript
state "game.exe" {
    chapter at 0x123456 as utf16le(16);
}
```

The dynamic equivalent is `process.readUtf16Le(address, maxUtf16Units)`. Both
forms stop at the first NUL code unit and replace malformed surrogate
sequences. An odd ASL byte bound has no exact UTF-16LE rewrite because its final
byte is an incomplete code unit.

When the parser encounters a type-first field such as
`string50 map : 0x100`, it explains this distinction and offers separate
**maybe-incorrect** UTF-8 and UTF-16LE rewrites. Neither edit is preferred or
machine-applicable because only the autosplitter author can verify the target
encoding. The UTF-16LE action is available only for an even ASL byte bound.

The stricter UTF-8 malformed-input policy is equivalent for A Plague Tale's
ASCII map identifiers. Do not use `readManagedString` for a native buffer: that
method reads the object layout of a Unity managed string rather than text bytes
at the supplied address.

The maintained Arietta of Spirits port uses independent `utf8(128)` and
`utf8(8)` fields for its stage and pause-menu identifiers. Its host fixture
also proves the persistent-watcher rule: a failed stage-string read retains
that field while a successful pause flag from the same poll still advances.

## C# string operations

SplitScript methods use lower camel case, so C# `StartsWith` becomes
`startsWith`. For ASCII game identifiers, C# `ToLower()` becomes the more
explicit `toAsciiLowerCase()`, while `ToUpper()` becomes
`toAsciiUpperCase()`:

```splitscript
let normalizedMap = current.map.toAsciiLowerCase()
let normalizedMission = current.mission.toAsciiUpperCase()
```

These conversions change only `A` through `Z` or `a` through `z` and preserve
all other UTF-8 bytes. They are not culture-sensitive or full Unicode case
conversion. `slice` uses
UTF-8 byte offsets rather than .NET UTF-16 indices and fails when an offset is
out of range or inside a multibyte code point, so do not mechanically translate
`Substring` without checking the target data.

For text proven to be ASCII, translate the overload shapes explicitly. C#
`value.Substring(start, length)` becomes
`value.slice(start, start + length)`, while `value.Substring(start)` becomes
`value.slice(start, value.byteLength())`. Both SplitScript calls are fallible.
For non-ASCII text, first derive UTF-8 byte boundaries rather than copying the
original UTF-16 positions.

C# `value.Trim()` recognizes Unicode whitespace. For game identifiers, log
lines, and configuration text known to use ASCII whitespace, use the deliberately
explicit operation:

```splitscript
let eventName = current.logLine.trimAsciiWhitespace()
```

It removes space, tab, line feed, vertical tab, form feed, and carriage return
from both ends, preserves interior and non-ASCII bytes, and reuses the original
immutable string when nothing changes. The compiler does not rewrite `Trim()`
automatically because Unicode whitespace, character-array overloads,
`TrimStart`, and `TrimEnd` have different semantics.

C# `PadLeft` and `PadRight` map to directionally named immutable operations:

```splitscript
let chapter = chapterNumber as String
let chapterKey = chapter.padStart(2, '0')
let column = chapterKey.padEnd(8, ' ')
```

SplitScript always requires the fill `char`; pass `' '` explicitly for C#
overloads that omit it. Width counts Unicode scalar values rather than .NET
UTF-16 code units or terminal display columns, so copied widths are directly
equivalent only for text proven to be ASCII. An already-wide string is reused;
otherwise the result is allocated once at its exact UTF-8 byte length.

C# combines nullability and emptiness in `String.IsNullOrEmpty(value)`.
SplitScript keeps those concerns in the type. A required `String` cannot be
null, so use its source-defined method directly:

```splitscript
let missingCheckpoint = current.checkpoint.isEmpty()
```

When the migrated value is deliberately optional, handle both variants:

```splitscript
let missingCheckpoint = match current.checkpoint {
    None => true,
    Some(text) => text.isEmpty(),
}
```

A failed process read is not automatically an empty or null string. Decide
first whether that boundary should remain a `Result`, become a `String?`, or
retain the last accepted state value. The compiler therefore gives
`String.IsNullOrEmpty` focused guidance without guessing an automatic rewrite.

C# `String.Length` counts UTF-16 code units, so it has no encoding-neutral
rename for SplitScript's immutable UTF-8 strings. Prefer the operation that
expresses the surrounding intent. An emptiness check does not need a numeric
unit:

```splitscript
if current.map.isEmpty() {
    return false
}
```

Use `byteLength()` only for text proven to be ASCII or code intentionally
working with the UTF-8 byte offsets returned by `indexOf`, `lastIndexOf`, and
accepted by `slice`, `byteAt`, and `charAt`. Non-ASCII text can have different
UTF-16 code-unit and UTF-8 byte counts, so the compiler diagnoses `.Length`
without applying a speculative fix.

C# `String.Join` puts the separator first and has many object, enumerable,
variadic, and range overloads. SplitScript accepts one typed string array and
puts the values first:

```splitscript
let routeName = String.join(routeParts, ".")
```

The implementation measures the complete UTF-8 result and allocates it once.
Empty and single-element arrays add no separator; empty elements are preserved.
Convert non-string values explicitly before joining. The compiler does not
swap arguments automatically because a C# call does not prove that its chosen
overload already contains a `[String]`.

C# `value.IndexOf(substring)` returns a UTF-16 code-unit index or `-1`.
SplitScript `value.indexOf(substring)` instead returns a UTF-8 byte offset as
`u32?`; handle `None` directly. The numeric offsets are equivalent only for
text proven to be ASCII:

```splitscript
let separator = current.map.indexOf("_") else return false
```

Comparison-mode and start-index overloads need an explicit rewrite rather than
a method-name substitution.

C# `value.LastIndexOf(substring)` has the same index-unit and absence hazards.
For exact searches over proven ASCII text, use `value.lastIndexOf(substring)`
and handle its `u32?` result directly:

```splitscript
let separator = current.path.lastIndexOf("/") else return false
```

The operation searches the complete string and returns a UTF-8 byte offset.
An empty substring is found at the final byte boundary. C# comparison-mode,
start-index, and count overloads need a deliberate rewrite.

C# `value.Replace(search, replacement)` maps to immutable
`value.replaceAll(search, replacement)` when `search` is non-empty and
`replacement` is not null. The SplitScript operation is fallible, so retain
that policy explicitly rather than discarding the result:

```splitscript
let displayName = current.map.replaceAll("_", " ") else current.map
```

C# permits a null replacement to mean deletion; pass `""` explicitly in
SplitScript only when that was the source's intent. An empty search is an error,
as is a result whose byte length cannot be represented. The compiler therefore
explains `.Replace(...)` but does not offer a blind rename that would leave
Result handling unresolved.

C# `left.Equals(right)` normally becomes `left == right`; SplitScript compares
strings by exact UTF-8 text rather than GC reference identity. If the source
intentionally ignores ASCII letter case, write
`left.equalsIgnoreAsciiCase(right)` instead. The compiler does not rewrite
`Equals` automatically because C#'s static and comparison-mode overloads need
semantic review.

Use `charAt(byteIndex)` for textual character checks. It returns a `char` and
still takes a UTF-8 byte offset; an offset into the middle of a multibyte scalar
is an error. Use `byteAt(byteIndex)` only when the ASL is genuinely inspecting
encoded bytes. Neither operation adopts C#'s or JavaScript's UTF-16 indexing:

```splitscript
let slash = current.map.charAt(7) else return false
return slash == '/'
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

The compiler recognizes the C# static `Parse` and `TryParse` families and
points to this pattern. It intentionally does not rewrite them: `TryParse`
output parameters become ordinary `Result` control flow, and the receiving
declaration or fallback determines the target type.

Unlike an exception-catching `Parse` call, failure is an ordinary `Result`.
Use `else` for a fallback, `?` to propagate the error from a function or state
field, or `match` when failure needs its own behavior. C# `TryParse` therefore
does not need an output parameter. Parsing consumes the complete ASCII decimal
string and rejects whitespace, separators, and trailing text. Float targets
accept case-insensitive `NaN`, `inf`, and `Infinity`; decimal overflow produces
infinity and underflow produces zero, while integer overflow remains an error.
Float conversion is correctly rounded directly to `f32` or `f64` and does not
inherit C# culture settings.

## C# `Convert` operations

C# `Convert.To*` calls do not all map to one SplitScript cast. Choose the
operation from the source value and the behavior the script needs:

```splitscript
let widened: i32 = byteValue as i32
let rounded: i32 = floatValue.round() as i32
let enabled = numericFlag != 0
let parsed: f64 = text.parse() else 0.0
```

Fixed-width integer `as` casts use SplitScript's numeric cast rules. In
particular, narrowing integers retain their low bits, while C# `Convert` throws
when a narrowing conversion is out of range. A floating-point `as` cast
truncates toward zero, saturates at an integer boundary, and maps NaN to zero.
For a finite, in-range value, `value.round() as i32` preserves the
midpoint-to-even rounding of `Convert.ToInt32`; it still does not reproduce C#
overflow and NaN exceptions.

For strings, infer the intended fixed-width number from the receiving boundary
and use fallible `parse()`. SplitScript parsing is strict, locale-independent
ASCII decimal parsing, unlike C# conversions that may accept surrounding
whitespace and current-culture formatting. A numeric `Convert.ToBoolean(value)`
becomes `value != 0`. For text, trim and compare `true` and `false` explicitly
with `equalsIgnoreAsciiCase`, choosing a Result or fallback for malformed text.

The ordinary one-value `Convert.ToString(value)` maps to Display:

```splitscript
let text = value as String
print(value)
setVariable("Value", value)
```

Interpolation, `print`, and `setVariable` already accept Display values, so
they do not need an intermediate string cast. The integer-radix overload maps
to the fallible `Integer.toString` method:

```splitscript
let hexadecimal = cellId.toString(16) else ""
let uppercase = hexadecimal.toAsciiUpperCase()
```

Radices from 2 through 36 use `0` through `9` and lowercase `a` through `z`.
Negative values retain a leading minus sign, including signed minima. This
differs from C#'s two's-complement rendering of negative values in base 2, 8,
or 16, so review any negative-source call rather than translating it blindly.
An out-of-range radix returns an error. Culture/provider, null, and object
overloads remain separate policies and are not ordinary Display conversions.

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

## Attached process identity

ASL exposes the selected process through `game.ProcessName`. In a native
SplitScript state, use `process.name()`:

```splitscript
state ["game.exe", "game-demo.exe"] {}

onAttach {
    if process.name() == "game-demo.exe" {
        print("Attached to the demo")
    }
}
```

The returned string is the exact candidate from the `state` declaration that
matched during attachment. It is not the executable path and does not perform
another host lookup. Use it when multiple executable names genuinely select
different behavior or layouts. When several builds share a name, discriminate
with reliable evidence such as `process.mainModule().size`, `process.path()`,
`Module.fileVersion()`, `Module.productVersion()`, or a signature instead.

Legacy `modules.Any(...)` checks often test for one optional module rather than
requiring full enumeration. Use the synchronous optional probe in that case:

```splitscript
let steam = process.loadedModule("steam_api.dll") != None
```

Use `await process.module("GameAssembly.dll")` when attachment must wait until
a required module loads. Do not use that waiting form for optional platform or
mod-loader detection: an absent module would keep `onAttach` pending forever.

The two version methods return typed four-part `FileVersion` values rather than
the punctuation-dependent strings exposed by C# `FileVersionInfo`. Use
`Module.versionInfo()` when both identities are needed, so the PE resource is
only traversed once.

The compiler recognizes the exact legacy path `game.ProcessName` and offers a
machine-applicable `process.name()` rewrite where the native `process` value is
in scope. Ordinary functions do not implicitly capture an attachment; pass the
name as a parameter when helper logic needs it.

## Timer state

ASL's `timer.CurrentPhase` maps directly to `timer.state()`. Compare the
resulting exhaustive enum by name:

```splitscript
reset {
    return timer.state() == TimerState.NotRunning && current.inMenu
}
```

The familiar variants retain their meanings as `TimerState.NotRunning`,
`Running`, `Paused`, and `Ended`. SplitScript additionally exposes
`TimerState.Unknown`; the runtime maps an unrecognized future host value there
instead of fabricating a known state. Use an explicit wildcard or `Unknown`
arm when matching according to the behavior the script needs.

Do not preserve integer comparisons such as `timer.CurrentPhase > 0`.
`TimerState` is an enum, not an ordered number. Compare or match the named
states that made the original condition true. The compiler offers
machine-applicable rewrites for `timer.CurrentPhase` and the four legacy
`TimerPhase` variants. A legacy enum name in a type or match pattern receives
focused guidance without reserving `TimerPhase` as a source identifier.

## Timer split index

ASL exposes `timer.CurrentSplitIndex` as a signed integer. SplitScript uses
`timer.currentSplitIndex()` and makes the no-attempt state explicit as `None`:

```splitscript
split {
    let index = timer.currentSplitIndex() else return false
    return match index {
        0 => current.level == 2,
        1 => current.level == 7,
        _ => false,
    }
}
```

The result type is `u64?`. Every negative value from the host ABI maps to
`None`; nonnegative values map to the corresponding `u64`. Do not cast the
signed sentinel into an unsigned index. A skipped segment advances the index,
and after the final split the index equals the route's segment count. The index
is therefore authoritative route progress, not a count of splits requested by
this autosplitter.

The compiler recognizes `timer.CurrentSplitIndex`, but deliberately does not
rewrite it automatically because the correct `None` behavior depends on the
surrounding control flow. Use `else` for an early fallback or `match` when the
absent state needs distinct behavior.

## LiveSplit timer metadata and control

Several legacy paths inspect host-owned timer data rather than the attached
game. They do not currently have faithful SplitScript replacements:

- `timer.CurrentTime.GameTime` reads LiveSplit's optional game-time clock. If
  this autosplitter computes the value, keep that `Duration` in script-owned
  state and also return it from `gameTime`; the host's possibly externally
  changed value cannot yet be read back.
- `timer.CurrentSplit.Name`, indexed `timer.Run[...]` segments,
  `timer.Run.Count`, `CategoryName`, `GameName`, and `FilePath` require a typed
  read-only run snapshot. Do not silently copy route metadata into the script
  unless that fixed route is intentionally owned by the autosplitter.
- `timer.Run.Offset` and timing-method access are user-visible configuration.
  Reads and writes need defined ordering, persistence, reset/undo behavior,
  precision, and conflict handling with LiveSplit's UI.

The current Wasm host exposes none of those observations or mutations. The
compiler emits focused diagnostics for each category instead of a generic
unknown-name error or a misleading automatic rewrite. Mark a port as
behavior-limited when it genuinely depends on one of them. The planned host
contract will expose optional values from one coherent snapshot per update and
will keep read-only metadata separate from controlled mutation authority.

## Monotonic delays and debouncing

ASL scripts often use `DateTime.Now`, `DateTime.Now.TimeOfDay`, or a
`Stopwatch` only to measure time since a game event. Use `Instant` for that
process-independent elapsed time:

```splitscript
let enteredMenuAt: Instant? = None

whileAttached {
    if current.inMenu && !old.inMenu {
        enteredMenuAt = Instant.now()
    } else if !current.inMenu {
        enteredMenuAt = None
    }
}

reset {
    let detectedAt = enteredMenuAt else return false
    return detectedAt.hasElapsed(Duration.fromMilliseconds(500))
}
```

`Instant.now()` reads a monotonic host clock. It never moves backwards during
one autosplitter instance and has no meaningful absolute or calendar value.
`elapsed()`, `durationSince(...)`, and `hasElapsed(...)` produce or compare
exact `Duration` values, making them appropriate for cooldowns, debouncing,
delayed splits, and retry deadlines. They continue advancing independently of
LiveSplit's loading and pause state.

Do not mechanically replace `timer.CurrentTime.RealTime`. That ASL expression
may mean the current LiveSplit attempt's run-relative time rather than an
independent delay. If the original logic starts its measurement at a game event,
capture an `Instant` there. If it needs the actual timer phase for an offset,
run-age check, or custom game-time calculation, the current host contract does
not expose equivalent metadata. The compiler diagnoses these paths separately
so this distinction is not hidden by a convenient but incorrect rewrite.

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

## Background signature scans

Legacy ASL often starts a C# `Thread` or task for signature discovery so a
large scan does not block LiveSplit's update loop. Do not translate that worker
or its cancellation token. SplitScript scans are already asynchronous: they
inspect only a bounded memory window per tick, yield to the host between
windows, preserve their cursor, and are discarded automatically if the
attached process closes.

Choose the narrowest range justified by the source:

```splitscript
onAttach {
    let executable = await process.mainModule()
    let code = await executable.scan(sig"48 8B 05 ?? ?? ?? ??")
    let table = retry process.readRelative32(code.offset(3))

    let heapMarker = await process.scanMemory(
        sig"54 49 4D 52 ?? ?? ?? ??",
    )
    print(`table {table}, marker {heapMarker}`)
}
```

Use `Module.scan` when a pattern belongs to one known image and `process.scan`
for another explicit address range. Use `process.scanMemory` only when the
legacy source genuinely enumerates readable mappings or the target may live
outside known modules. `scanAny` and `scanMemoryAny` accept an array of
signatures and return both the address and selected index, which keeps fallback
layout selection in one cooperative pass.

When a port needs the mapping metadata itself, take a typed snapshot rather
than reproducing the host's numeric count/index ABI:

```splitscript
let ranges = process.memoryRanges()
for range in ranges {
    if range.readable && range.executable {
        debug print(`executable mapping at {range.address}`)
    }
}
```

`memoryRanges` is synchronous because it only copies cheap host metadata.
Searching the contents of those ranges should still use the suspending scan
APIs so large reads yield between bounded windows.

An awaited scan remains pending when no signature is present; it does not
produce a temporary zero address. This matches attach-time discovery that
should wait for a module or runtime allocation to become ready. If an absent
pattern must instead select an unsupported-build path after a deadline, keep
that as an explicit timeout/race requirement—the language does not silently
turn a retrying scan into a one-shot result.

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

## Collection search and run-scoped sets

C# array `.Length` maps directly to the SplitScript method `.length()`. It
returns the `u32` element count for both `[T]` and fixed `[T; N]` arrays. The
compiler can apply this rename automatically, after which signed C# index
arithmetic may still need an explicit width cast.

After choosing a SplitScript collection shape, C# `.Count` also becomes
`.length()`. For an array this is the element count; for `Set<T>` it is the
number of unique stored values.

C# `List<T>` maps to SplitScript's `[T]` array type. SplitScript will not add a
separate compatibility-shaped `List<T>`. `[T]` is the variable-length ordered
sequence, while `[T; N]` carries an exact fixed length for layouts and other
code where the size is part of the type. Use `Set<T>` only when the original
data is genuinely an unordered collection of unique values, not merely because
its current API happens to be mutable.

Arrays provide `contains` and `indexOf` when their elements support equality:

```splitscript
let levelRoute = [12, 5, 6, 7, 9, 10, 11, 14]

split {
    let oldIndex = levelRoute.indexOf(old.level) else return false
    let currentIndex = levelRoute.indexOf(current.level) else return false
    return currentIndex == oldIndex + 1
}
```

Unlike C# `List<T>.IndexOf`, `indexOf` returns `u32?`; absence is `None`, never
a signed `-1` sentinel. Replace an existing element with ordinary indexed
assignment; the index is `u32`, aliases observe the change, and an out-of-range
index traps just like an indexed read:

```splitscript
route[currentIndex] = nextLevel
route[currentIndex] += 1
```

Plain indexed assignment evaluates the collection and index once. Compound
forms such as `route[nextIndex()] += 1` additionally evaluate the right operand
once and use the same typed operator as an ordinary `+=`; `nextIndex()` is not
called twice. Growable `[T]` supports `push`, `extend`, indexed `removeAt`,
optional `pop`, first-match `remove(value)`, and capacity-preserving `clear`. C#
`list.AddRange(values)` becomes
`list.extend(values)` once both collections are represented as typed arrays;
self-extension duplicates the original elements once. Successful structural
operations invalidate active iteration. C# `RemoveAt(index)` maps directly to
`removeAt(index)`; an out-of-range `u32` index traps just like array indexing.
Use `let last = values.pop() else ...` where C# removes a final list or stack
element: SplitScript returns `None` for an empty array instead of throwing.
`List<T>.Remove(value)` maps to `remove(value)`, removes only the first equal
element, and returns whether a match existed. Ignoring that boolean is valid
when the source does not distinguish absence.
`[T; N]` remains fixed-length and supports none of these operations.

Use `Set<T>` when values are discovered while the run progresses and only
membership matters:

```splitscript
let visitedMaps = Set.new<String>()

onAttach {
    visitedMaps.clear()
}

split {
    if current.map == old.map {
        return false
    }
    return visitedMaps.insert(current.map)
}
```

`insert` returns true only for a new value. The set object and its contents
persist across ticks and detachments until explicitly cleared or the script is
unloaded. Clear it at the lifecycle boundary that matches the original source:
`onAttach` for per-process state, or a detected timer-start transition for
per-attempt state. The maintained OpenJK-Speed port exercises the former.

When membership comes from a small closed enum, a typed bit set remains more
compact and makes the finite domain explicit:

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

Growable ordered storage, insertion order, and repeated equal values all belong
to `[T]`. They do not justify another collection type. Record any still-missing
specific operation rather than describing `List<T>` itself as missing. Indexed
insertion remains deferred until a maintained port demonstrates that it is
needed.

## Finite settings families

Prefer direct `settings.name` access when the setting is known statically. For
data tables whose entries select among declared boolean settings, give each
declaration its exact host-map string with `key "..."` and use
`settings.enabled(key)`. This remains boolean-only and returns false for an
unknown key; it is not a dynamically typed replacement for choice or file
settings. If the original settings have a boolean parent, gate the child result
explicitly; a quoted SplitScript heading is visual only. The complete A Plague
Tale example preserves its **All Chapters** parent semantics this way.

When cursor advancement depends on declaration membership rather than whether
the split is enabled, keep the two questions separate:

```splitscript
if settings.contains(checkpointKey) {
    checkpointIndex += 1
    return settings.enabled(checkpointKey)
}
```

`contains` recognizes declared boolean, choice, and file keys, including
explicit `key "..."` spellings. It returns false for visual headings and unknown
keys. This matches legacy `Settings.ContainsKey` without exposing a dynamically
typed host map.

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

## Snapshot-dependent helper functions

Ordinary helper functions may refer to `old` and `current` directly, just as a
process-dependent helper may refer to `process`:

```splitscript
state "game.exe" {
    level: u32 at 0x1000
}

fn enteredLevel(level) {
    return old.level != level && current.level == level
}

split {
    return enteredLevel(7u32)
}
```

The compiler derives a state-snapshot requirement from the helper body and
propagates it through every calling helper. Such functions are offered only in
contexts where a complete pair of snapshots exists: `whileAttached` and the
timer-decision actions. Calling one from `setup`, `onAttach`, `onDetach`, a
state source, or a state filter produces a focused diagnostic. This keeps the
concise ASL helper shape without exposing default-initialized or stale state.

Explicit snapshot parameters remain useful when a helper should operate on an
arbitrary snapshot supplied by its caller, but they are not required merely to
move a lifecycle condition into a named function.

## Legacy ASL lifecycle blocks

The similarly shaped block names are not interchangeable. The original
LiveSplit component invokes them at different boundaries:

| ASL construct | Exact legacy timing | SplitScript direction |
| --- | --- | --- |
| `startup` | Once when the script is loaded, before process attachment | Put settings in `settings`, constant data in global initializers, and remaining process-independent statements in `setup`. |
| `init` | Once for each found process, after one legacy state refresh; a failure retries attachment initialization | Put suspending discovery and layout selection in `onAttach`. Put synchronous work that consumes the first complete snapshot in `onStateReady`. |
| `update` | After each refresh and before all timer decisions; `false` skips the remaining decisions for that tick | Put ordinary per-tick work in `whileAttached`. An explicit `return false` preserves the legacy control result exactly. |
| `exit` | When the attached process exits | Use `onDetach`. It runs exactly once for a real process closure and never at initial detached startup. |
| `shutdown` | When the script is disabled, reloaded, dropped, or LiveSplit exits | No exact host callback exists yet; do not approximate it with `onDetach`. |
| `timer.OnStart`, `timer.OnSplit`, `timer.OnReset` | LiveSplit timer events, which may be raised independently of this script's decision blocks | Reconstruct only simple observable transitions in `whileAttached`. Exact lossless events require the planned host contract. |

For example, process-independent ASL startup statements belong in `setup`, not
`onAttach`:

```splitscript
setup {
    print("Autosplitter loaded")
}
```

`setup` runs at the beginning of the module's first interruptible host update,
after settings are available, but cannot use `process`, `gba`, `current`,
`old`, `await`, or `retry`. A debug-watch replacement loads a new module and
therefore runs it again on that module's first update.

Legacy `init` combines two boundaries that SplitScript keeps explicit. Use
`onAttach` for discovery that may suspend and for layout selection. Use
`onStateReady` for synchronous initialization that needs polled state:

```splitscript
onAttach {
    let image = await Unity.i12cpp()
    gameManager = await image.class("GameManager").staticInstance("Instance")
}

onStateReady {
    print(`Initial level: {current.level}`)
}
```

`onStateReady` runs once per attachment only after every field in the first
snapshot was read and accepted. `old` and `current` are both that snapshot, so
initialization cannot look like a transition from default values. It cannot
suspend. `whileAttached` and timer-decision actions begin on the next update.

Legacy `update { return false; }` maps directly to `whileAttached`. The state
snapshot has already refreshed, but the remaining timer decisions are skipped
for that update:

```splitscript
whileAttached {
    if !helperLoaded {
        return false
    }

    // Per-update bookkeeping.
}
```

Falling through, a bare `return`, or `return true` continues to `start`,
`isLoading`, `gameTime`, `reset`, and `split` as applicable. This control result
does not reject or roll back the refreshed snapshot.

ASL `refreshRate` is a frequency, so migrate `refreshRate = 60` to
`setTickRate(60)`. The host converts it to the wait interval `1 / hz` after the
current update returns. The chosen rate persists across process closure; it is
not reset by attachment management. If `onAttach` increases the rate, restore
the script's baseline explicitly:

```splitscript
setup {
    setTickRate(60)
}

onDetach {
    setTickRate(60)
}

onAttach {
    setTickRate(120)
}
```

The two calls represent different boundaries: `setup` establishes the initial
policy, while `onDetach` restores it after each real process closure.

## Process-exit game-time cleanup

ASL commonly pauses game time in `exit`. Map that cleanup directly to
`onDetach`:

```splitscript
onDetach {
    timer.pauseGameTime()
}
```

The compiler invokes this block once after clearing the closed handle, provider
state, selected layout, and pending process-lifetime continuations. Neither
`process` nor state snapshots are available in `onDetach`: a process may close
before attachment initialization or the
first state poll completes.

Use `isLoading` for ordinary load removal. `timer.pauseGameTime()` and
`timer.resumeGameTime()` are explicit lifecycle tools, not a replacement for
that declarative action.
