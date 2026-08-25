# Porting ASL to SplitScript

This self-contained guide records mappings proven by maintained, host-executed
ports. It is not a token-substitution table: ASL's dynamic state and C# runtime
sometimes need a typed SplitScript design rather than a literal translation.
Every required semantic distinction and canonical pattern is explained here;
it has no source-file or repository-document prerequisites. Examples are small,
focused snippets that explain one concept rather than complete autosplitters.
Every SplitScript snippet is compiled as an independent focused program during
repository verification. The rendered guide omits its rustdoc-style hidden
setup, so visible code stays limited to the concept while types, effects,
lifecycle availability, and current API spellings remain checked.

## Attachment state declarations

Every SplitScript file is one executable autosplitter and declares exactly one
attachment provider. A native Windows game names the exact process candidate
that the host must match:

```splitscript
state "game.exe" {
    health: i32 at 0x1000;
}
```

Use an array when editions have different executable names but share the same
state shape:

```splitscript
state ["game.exe", "game-demo.exe"] {
    health: i32 at 0x1000;
}
```

These strings are exact host process identities, not portable paths. The
current Windows host reports the executable filename including `.exe`, so a
Windows candidate must include that extension; `state "game"` will not attach
to `game.exe`. Other host platforms use their exact runtime identity. The array
contains alternate names for one attachment, not several processes to attach
to concurrently. Build-specific addresses belong in named layouts selected
from [`onAttach`], rather than in multiple ASL-style state blocks.

Typed emulator support replaces the native process root. For example, a GBA
autosplitter declares `state GBA` and reads emulated addresses through `gba`:

```splitscript
state GBA {
    room: u8 at 0x03000010;
}
```

The state declaration also defines the transactional snapshots. After the
first complete poll, [`current`] contains the latest accepted values and [`old`]
contains the preceding accepted values. `process` or the provider-specific
root is available only during attachment-owned lifecycle phases; [`old`] and
[`current`] are unavailable before the first complete snapshot and are not
guaranteed during [`onDetach`].

## Translating statement-heavy expressions

ASL and C# helpers often need several statements to choose one value. In an
expression position, SplitScript braces form a value block: local statements
run first and the final expression supplies the value. This works in [`if`]
branches, [`match`] arms, fallback [`else`] expressions, arguments, and state
initializers:

```splitscript
# state "game.exe" {}
fn category(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}
# setup { print(category(true)) }
```

The final expression is local to the nested block and has no [`return`] keyword.
Functions and lifecycle actions remain statement bodies and require explicit
[`return`]. A value block with no final expression yields [`None`]; a block that
always returns, throws, breaks, or continues has type [`Never`]. A trailing
semicolon after the final expression is accepted, but the compiler warns and
the formatter removes it because it is still the block's value.

## Infinite loops and value-carrying breaks

Use [`loop`] for unconditional repetition. A loop without a reachable
[`break`] has type [`Never`]. Within a [`loop`] expression, `break value`
supplies its result and all break values are inferred together. A bare break
supplies [`None`].

```splitscript
# state "game.exe" {}
fn chooseImage(vulkan: bool) -> String {
    return loop {
        if vulkan {
            break "EngineWin64sv.dll"
        }
        break "EngineWin64s.dll"
    }
}
# setup { print(chooseImage(false)) }
```

Legacy ASL and C# `while (true)` and JavaScript `while (true)` normally become
[`loop`] when they are intentionally unconditional. Keep [`while`] when the
condition carries real policy. Unlike Rust, SplitScript functions do not
implicitly return a final loop expression: write `return loop { ... }`.
Value-carrying [`break`] is limited to [`loop`]; a nested [`while`] or runtime
[`for`] always captures its own bare break.

## Signed pointer offsets

ASL `DeepPointer` paths commonly contain negative offsets. Preserve them
directly; do not cast them through [`u64`]:

```splitscript
state "game.exe" {
    health: i32 at "game.dll", 0x120, -0x18;
}
```

The absolute root in `at 0xffff_ffff_ffff_fff0` remains an unsigned 64-bit
address. Module-relative roots and every offset after a dereference are signed
[`i64`], and arithmetic wraps modulo the address space. The equivalent dynamic
APIs are `process.follow(base, offsets: [i64])`,
`address.offset(displacement)`, which accepts any integer width, and signed
[`MemoryPath`] offsets.

## Composing dynamic pointer paths

ASL scripts often copy a selected `DeepPointer` offset array and append one
runtime-specific final offset. SplitScript's growable [`[T]`] arrays support
that operation directly; do not branch over every possible path length. Create
a fresh array, [`extend`] it with the selected path, [`push`] the final offset,
then pass the complete path to [`Process.follow`]:

```splitscript
fn readDynamic(base: address, path: [i64], finalOffset: i64) -> f64! {
    let fullPath: [i64] = []
    fullPath.extend(path)
    fullPath.push(finalOffset)
    let target = process.follow(base, fullPath)?
    return process.read<f64>(target)
}

state "game.exe" {
    value: f64 = readDynamic(0x1000, [0x10, 0x20], 0x30);
}
```

[`extend`] copies the elements into the new array, so the selected source path
is not mutated. [`push`] adds one element and keeps the resulting value an
ordinary `[i64]`; [`Process.follow`] therefore handles every source length with
the same function.

An ASL background task that repeatedly retries the same `DeepPointer` usually
does not need a task or dynamically replaced watcher in SplitScript. Retain a
[`MemoryPath`] after module discovery and resolve it from an expression-backed
state field on every poll:

```splitscript
# let loadingPath: MemoryPath? = None
fn readLoading(path: MemoryPath?) -> bool {
    let selectedPath = path else return false
    let address = selectedPath.resolve() else return false
    return process.read<bool>(address) else false
}

# state "game.exe" {
#     loading: bool = readLoading(loadingPath);
# }
# onAttach {
#     let module = await process.module("game.dll")
#     loadingPath = module.address.memoryPath(
#         [0x20, 0x18],
#         0x4,
#         PointerSize.Bit32,
#     )
# }
```

[`MemoryPath.resolve`] follows the retained chain afresh, so an object that is
created or replaced later becomes visible without guest-managed cancellation.
The explicit `else false` matches ASL `ReadFailAction.SetZeroOrNull`. Letting a
fallible read escape the field instead keeps its last accepted value, matching
the default persistent-watcher behavior. Use the target's actual
[`PointerSize`]; do not infer it from the host running the timer.

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
an ordinary immutable [`String`]. SplitScript resolves every pointer offset first,
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
ASCII map identifiers. Do not use [`readManagedString`] for a native buffer: that
method reads the object layout of a Unity managed string rather than text bytes
at the supplied address.

The maintained Arietta of Spirits port uses independent `utf8(128)` and
`utf8(8)` fields for its stage and pause-menu identifiers. Its host fixture
also proves the persistent-watcher rule: a failed stage-string read retains
that field while a successful pause flag from the same poll still advances.

## Contiguous records and fixed arrays

Several separate ASL watchers may describe one physically contiguous native
value. When the target layout is known, a SplitScript record reads the complete
value once and gives each component a name. Fields use declaration order with
natural alignment:

```splitscript
record LevelTimeParts {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

state "game.exe" {
    levelTime: LevelTimeParts at "game.exe", 0x1200;
}
```

This record is 12 bytes because all three fields have four-byte size and
alignment. In a mixed record, each field starts at the next multiple of its
own alignment and the final size is rounded to the largest field alignment.
SplitScript currently reads these values as little-endian. Do not use a record
for a packed, explicitly padded, or differently endian target layout; exact
layout controls remain intentionally deferred until maintained-port evidence
requires them.

For a contiguous homogeneous region, [`[T; N]`] carries the physical element
count in the type and also performs one host read:

```splitscript
state "game.exe" {
    inventory: [u8; 6] at "game.exe", 0x2b32;
}

split {
    return old.inventory[0] != current.inventory[0]
}
```

Use [`[T; N]`], not growable [`[T]`], for process-memory layout. The fixed array
still supports indexing and iteration, while its exact length prevents a port
from silently reading a different number of bytes. A record or fixed array is
[`MemoryReadable`] only when every contained field or element has a fixed
readable layout.

## C# string operations

SplitScript methods use lower camel case, so C# `StartsWith` becomes
[`startsWith`]. For ASCII game identifiers, C# `ToLower()` becomes the more
explicit [`toAsciiLowerCase()`], while `ToUpper()` becomes
[`toAsciiUpperCase()`]:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
#     mission at 0x1100 as utf8(64);
# }
# whileAttached {
let normalizedMap = current.map.toAsciiLowerCase()
let normalizedMission = current.mission.toAsciiUpperCase()
# print(normalizedMap)
# print(normalizedMission)
# }
```

These conversions change only `A` through `Z` or `a` through `z` and preserve
all other UTF-8 bytes. They are not culture-sensitive or full Unicode case
conversion. [`slice`] uses
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
# state "game.exe" {
#     logLine at 0x1000 as utf8(64);
# }
# whileAttached {
let eventName = current.logLine.trimAsciiWhitespace()
# print(eventName)
# }
```

It removes space, tab, line feed, vertical tab, form feed, and carriage return
from both ends, preserves interior and non-ASCII bytes, and reuses the original
immutable string when nothing changes. The compiler does not rewrite `Trim()`
automatically because Unicode whitespace, character-array overloads,
`TrimStart`, and `TrimEnd` have different semantics.

C# `PadLeft` and `PadRight` map to directionally named immutable operations:

```splitscript
# state "game.exe" {}
# onAttach {
# let chapterNumber: u32 = 7
let chapter = chapterNumber as String
let chapterKey = chapter.padStart(2, '0')
let column = chapterKey.padEnd(8, ' ')
# print(column)
# }
```

SplitScript always requires the fill [`char`]; pass `' '` explicitly for C#
overloads that omit it. Width counts Unicode scalar values rather than .NET
UTF-16 code units or terminal display columns, so copied widths are directly
equivalent only for text proven to be ASCII. An already-wide string is reused;
otherwise the result is allocated once at its exact UTF-8 byte length.

C# combines nullability and emptiness in `String.IsNullOrEmpty(value)`.
SplitScript keeps those concerns in the type. A required [`String`] cannot be
null, so use its source-defined method directly:

```splitscript
# state "game.exe" {
#     checkpoint at 0x1000 as utf8(64);
# }
# whileAttached {
let missingCheckpoint = current.checkpoint.isEmpty()
# print(missingCheckpoint)
# }
```

When the migrated value is deliberately optional, handle both variants:

```splitscript
# state "game.exe" {
#     checkpoint: String? at 0x1000 as utf8(64);
# }
# whileAttached {
let missingCheckpoint = match current.checkpoint {
    None => true,
    Some(text) => text.isEmpty(),
}
# print(missingCheckpoint)
# }
```

A failed process read is not automatically an empty or null string. Decide
first whether that boundary should remain fallible ([`T!`]), become a `String?`, or
retain the last accepted state value. The compiler therefore gives
`String.IsNullOrEmpty` focused guidance without guessing an automatic rewrite.

C# `String.Length` counts UTF-16 code units, so it has no encoding-neutral
rename for SplitScript's immutable UTF-8 strings. Prefer the operation that
expresses the surrounding intent. An emptiness check does not need a numeric
unit:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# whileAttached {
if current.map.isEmpty() {
    return false
}
# }
```

Use [`byteLength()`] only for text proven to be ASCII or code intentionally
working with the UTF-8 byte offsets returned by `indexOf`, [`lastIndexOf`], and
accepted by [`slice`], [`byteAt`], and [`charAt`]. Non-ASCII text can have different
UTF-16 code-unit and UTF-8 byte counts, so the compiler diagnoses `.Length`
without applying a speculative fix.

C# `String.Join` puts the separator first and has many object, enumerable,
variadic, and range overloads. SplitScript accepts one typed string array and
puts the values first:

```splitscript
# state "game.exe" {}
# onAttach {
# let routeParts = ["chapter", "level"]
let routeName = String.join(routeParts, ".")
# print(routeName)
# }
```

The implementation measures the complete UTF-8 result and allocates it once.
Empty and single-element arrays add no separator; empty elements are preserved.
Convert non-string values explicitly before joining. The compiler does not
swap arguments automatically because a C# call does not prove that its chosen
overload already contains a `[String]`.

C# `value.IndexOf(substring)` returns a UTF-16 code-unit index or `-1`.
SplitScript `value.indexOf(substring)` instead returns a UTF-8 byte offset as
`u32?`; handle [`None`] directly. The numeric offsets are equivalent only for
text proven to be ASCII:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# split {
let separator = current.map.indexOf("_") else return false
# return separator > 0
# }
```

Comparison-mode and start-index overloads need an explicit rewrite rather than
a method-name substitution.

C# `value.LastIndexOf(substring)` has the same index-unit and absence hazards.
For exact searches over proven ASCII text, use `value.lastIndexOf(substring)`
and handle its `u32?` result directly:

```splitscript
# state "game.exe" {
#     path at 0x1000 as utf8(64);
# }
# split {
let separator = current.path.lastIndexOf("/") else return false
# return separator > 0
# }
```

The operation searches the complete string and returns a UTF-8 byte offset.
An empty substring is found at the final byte boundary. C# comparison-mode,
start-index, and count overloads need a deliberate rewrite.

C# `value.Replace(search, replacement)` maps to immutable
`value.replaceAll(search, replacement)` when `search` is non-empty and
`replacement` is not null. The SplitScript operation is fallible, so retain
that policy explicitly rather than discarding the result:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# whileAttached {
let displayName = current.map.replaceAll("_", " ") else current.map
# print(displayName)
# }
```

C# permits a null replacement to mean deletion; pass `""` explicitly in
SplitScript only when that was the source's intent. An empty search is an error,
as is a result whose byte length cannot be represented. The compiler therefore
explains `.Replace(...)` but does not offer a blind rename that would leave
[`T!`] handling unresolved.

C# `left.Equals(right)` normally becomes `left == right`; SplitScript compares
strings by exact UTF-8 text rather than GC reference identity. If the source
intentionally ignores ASCII letter case, write
`left.equalsIgnoreAsciiCase(right)` instead. The compiler does not rewrite
`Equals` automatically because C#'s static and comparison-mode overloads need
semantic review.

Use `charAt(byteIndex)` for textual character checks. It returns a [`char`] and
still takes a UTF-8 byte offset; an offset into the middle of a multibyte scalar
is an error. Use `byteAt(byteIndex)` only when the ASL is genuinely inspecting
encoded bytes. Neither operation adopts C#'s or JavaScript's UTF-16 indexing:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# split {
let slash = current.map.charAt(7) else return false
return slash == '/'
# }
```

C# `Split` maps to fallible, lower-camel-case [`String.split`]. SplitScript matches one
exact non-empty delimiter from left to right and preserves leading, adjacent,
and trailing empty segments:

```splitscript
# state "game.exe" {
#     levelPicture at 0x1000 as utf8(64);
# }
# whileAttached {
let fileParts = current.levelPicture.split(".") else []
let levelParts = fileParts[0].split("_") else []
# print(levelParts.length())
# }
```

The empty delimiter is an error rather than a request to split UTF-8 bytes.
For a name such as `01_02_1.dds`, the two calls above produce `01`, `02`, and
`1` without discarding meaningful empty fields in other inputs.

C# `Int32.Parse`, `Double.Parse`, and their fixed-width relatives map to
`text.parse()`. Put the SplitScript numeric type at the receiving boundary and
let inference flow backward:

```splitscript
# state "game.exe" {
#     percentageText at 0x1000 as utf8(16);
# }
# whileAttached {
let percentage: f64 = current.percentageText.parse() else 0.0
# }
```

The compiler recognizes the C# static `Parse` and `TryParse` families and
points to this pattern. It intentionally does not rewrite them: `TryParse`
output parameters become ordinary [`T!`] control flow, and the receiving
declaration or fallback determines the target type.

Unlike an exception-catching `Parse` call, failure is an ordinary [`T!`] value.
Use [`else`] for a fallback, [`?`] to propagate the error from a function or state
field, or [`match`] when failure needs its own behavior. C# `TryParse` therefore
does not need an output parameter. Parsing consumes the complete ASCII decimal
string and rejects whitespace, separators, and trailing text. Float targets
accept case-insensitive `NaN`, `inf`, and `Infinity`; decimal overflow produces
infinity and underflow produces zero, while integer overflow remains an error.
Float conversion is correctly rounded directly to [`f32`] or [`f64`] and does not
inherit C# culture settings.

## C# `Convert` operations

C# `Convert.To*` calls do not all map to one SplitScript cast. Choose the
operation from the source value and the behavior the script needs:

```splitscript
# state "game.exe" {}
# onAttach {
# let byteValue: u8 = 7
# let floatValue: f64 = 3.5
# let numericFlag: i32 = 1
# let text = "42.5"
let widened: i32 = byteValue as i32
let rounded: i32 = floatValue.round() as i32
let enabled = numericFlag != 0
let parsed: f64 = text.parse() else 0.0
# }
```

Fixed-width integer [`as`] casts use SplitScript's numeric cast rules. In
particular, narrowing integers retain their low bits, while C# `Convert` throws
when a narrowing conversion is out of range. A floating-point [`as`] cast
truncates toward zero, saturates at an integer boundary, and maps NaN to zero.
For a finite, in-range value, `value.round() as i32` preserves the
midpoint-to-even rounding of `Convert.ToInt32`; it still does not reproduce C#
overflow and NaN exceptions.

For strings, infer the intended fixed-width number from the receiving boundary
and use fallible [`parse()`]. SplitScript parsing is strict, locale-independent
ASCII decimal parsing, unlike C# conversions that may accept surrounding
whitespace and current-culture formatting. A numeric `Convert.ToBoolean(value)`
becomes `value != 0`. For text, trim and compare `true` and `false` explicitly
with [`equalsIgnoreAsciiCase`], choosing a [`T!`] value or fallback for malformed text.

The ordinary one-value `Convert.ToString(value)` maps to [`Display`]:

```splitscript
# state "game.exe" {}
# onAttach {
# let value: i32 = 7
let text = value as String
print(value)
setVariable("Value", value)
# print(text)
# }
```

Interpolation, [`print`], and [`setVariable`] already accept [`Display`] values, so
they do not need an intermediate string cast.

A port-defined record or enum is a [`Display`] value automatically and receives
a stable multiline structural representation. Define
`fn Type.toString() -> String` only when the timer-facing text should use a
custom format; the result may be inferred. No `impl` block or annotation is
necessary.

```splitscript
# state "game.exe" {}
record Position {
    x: i32,
    y: i32,
}
fn Position.toString() { return `({self.x}, {self.y})` }
# onAttach {
setVariable("Position", Position { x: 3, y: 5 })
# }
```

The integer-radix overload maps to the fallible [`Integer.toString`] method:

```splitscript
# state "game.exe" {}
# onAttach {
# let cellId: u32 = 0x2a
let hexadecimal = cellId.toString(16) else ""
let uppercase = hexadecimal.toAsciiUpperCase()
# print(uppercase)
# }
```

Radices from 2 through 36 use `0` through `9` and lowercase `a` through `z`.
Negative values retain a leading minus sign, including signed minima. This
differs from C#'s two's-complement rendering of negative values in base 2, 8,
or 16, so review any negative-source call rather than translating it blindly.
An out-of-range radix returns an error. Culture/provider, null, and object
overloads remain separate policies and are not ordinary [`Display`] conversions.

## Version-labelled ASL states

The second argument in `state("game.exe", "Steam")` is a layout label, not
another executable candidate. Co-locate layouts in one state and return the
selected generated variant from [`onAttach`]:

```splitscript
state "game.exe" {
    layout Steam {
        loading: bool at "engine.dll", 0x1000;
        checkpoint: u8 at "engine.dll", 0x1100;
    },

    layout Epic {
        loading: bool at "engine.dll", 0x2000;
        checkpoint: u16 at "engine.dll", 0x2100;
    },
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

split {
    return match layout {
        StateLayout.Steam => old.checkpoint != current.checkpoint,
        StateLayout.Epic => old.checkpoint != current.checkpoint,
    }
}
```

Detection is ordinary typed code and may use module size, [`FileVersion`],
signatures, process identity, or discovered memory. `await process.closed()`
keeps an unsupported attachment inert without detaching and immediately
reattaching to the same process.

Compatible fields declared by every named layout expose one common interface.
When a field is missing or the same name has a conflicting type, match the
generated [`layout`] value and access the field only in the corresponding arm.
The compiler gives each incompatible declaration its real physical type rather
than turning it into an option or inventing a default. An honest typed default
is still appropriate when consumers already define that value as unavailable;
the A Plague Tale Xbox layout uses `cutsceneState: i32 = 0` for exactly that
reason.

When the original script has several independent build facts, avoid turning
their cartesian product into many version-labelled states. Declare enum-valued
dimensions in one unnamed state [`layout`] block and return the generated
`Layout` record from [`onAttach`]. The selected [`layout`] value can guard native
state fields and managed class fields with the same predicate:

```splitscript
enum Edition {
    Full,
    Demo,
}

enum Storefront {
    Steam,
    GOG,
}

state Unity ["game.exe"] {
    layout {
        edition: Edition,
        storefront: Storefront,
    }

    if layout.edition == Edition.Full {
        level: u32 at 0x1000;
    }
}

onAttach {
    return Layout {
        edition: Edition.Full,
        storefront: Storefront.Steam,
    }
}
```

Use a named `layout Steam { ... }` state when one selection genuinely chooses
the complete memory shape. Use dimensions when edition, storefront, renderer,
or another fact can vary independently or is shared with managed metadata.

## Attached process identity

ASL exposes the selected process through `game.ProcessName`. In a native
SplitScript state, use `process.name()`:

```splitscript
state ["game.exe", "game-demo.exe"] {
    layout FullGame {},
    layout Demo {},
}

onAttach {
    return match process.name() {
        "game.exe" => StateLayout.FullGame,
        "game-demo.exe" => StateLayout.Demo,
        _ => await process.closed(),
    }
}
```

The returned string is the exact candidate from the [`state`] declaration that
matched during attachment. It is not the executable path and does not perform
another host lookup. String literals are first-class [`match`] patterns and
compare text contents, so this is the direct selector when executable names map
to named layouts. Keep the wildcard arm because strings are an open-ended
domain. When several builds share a name, discriminate with reliable evidence
such as `process.mainModule().size`, `process.path()`,
[`Module.fileVersion()`], [`Module.productVersion()`], or a signature instead.

Legacy `modules.Any(...)` checks often test for one optional module rather than
requiring full enumeration. Use the synchronous optional probe in that case:

```splitscript
# state "game.exe" {}
# onAttach {
let steam = match process.loadedModule("steam_api.dll") {
    Some(_) => true,
    None => false,
}
# print(steam)
# }
```

Use `await process.module("GameAssembly.dll")` when attachment must wait until
a required module loads. Do not use that waiting form for optional platform or
mod-loader detection: an absent module would keep [`onAttach`] pending forever.

The common ASL expression `modules.First()` normally refers to the executable
itself. Discover that value once in [`onAttach`], then use its typed fields:

```splitscript
# state "game.exe" {}
onAttach {
    let executable = await process.mainModule()
    print(`executable size: {executable.size}`)
}
```

This replaces `ModuleMemorySize` with `size` and `BaseAddress` with [`address`].
It also makes the suspension visible: module discovery happens before polling,
not implicitly whenever a property is read.

The two version methods return typed four-part [`FileVersion`] values rather than
the punctuation-dependent strings exposed by C# `FileVersionInfo`. Use
[`Module.versionInfo()`] when both identities are needed, so the PE resource is
only traversed once.

```splitscript
# state "game.exe" {}
onAttach {
    let executable = await process.mainModule()
    let product = executable.productVersion() else return
    let label = match product {
        v"1.2.3.4" => "recognized build",
        _ => "unsupported build",
    }
    print(label)
}
```

[`FileVersion`] literals are also first-class [`match`] patterns. Always add a
wildcard arm because executable versions are an open set.

Only preserve full enumeration when the source genuinely needs to inspect
unknown module names. SplitScript does not currently expose that host operation.
Do not replace it with `process.memoryRanges()`: mapped memory ranges and loaded
modules have different identities and lifetime semantics. Record such a port as
host-limited instead of silently changing its behavior.

The compiler recognizes the exact legacy path `game.ProcessName` and offers a
machine-applicable `process.name()` rewrite where the native `process` value is
in scope. Ordinary functions do not implicitly capture an attachment; pass the
name as a parameter when helper logic needs it.

## Timer state

ASL's `timer.CurrentPhase` maps directly to [`timer.state()`]. Compare the
resulting exhaustive enum by name:

```splitscript
# state "game.exe" {
#     inMenu: bool at 0x1000;
# }
reset {
    return timer.state() == TimerState.NotRunning && current.inMenu
}
```

The familiar variants retain their meanings as [`TimerState.NotRunning`],
`Running`, `Paused`, and `Ended`. SplitScript additionally exposes
[`TimerState.Unknown`]; the runtime maps an unrecognized future host value there
instead of fabricating a known state. Use an explicit wildcard or `Unknown`
arm when matching according to the behavior the script needs.

Do not preserve integer comparisons such as `timer.CurrentPhase > 0`.
[`TimerState`] is an enum, not an ordered number. Compare or match the named
states that made the original condition true. The compiler offers
machine-applicable rewrites for `timer.CurrentPhase` and the four legacy
`TimerPhase` variants. A legacy enum name in a type or match pattern receives
focused guidance without reserving `TimerPhase` as a source identifier.

## Timer split index

ASL exposes `timer.CurrentSplitIndex` as a signed integer. SplitScript uses
[`timer.currentSplitIndex()`] and makes the no-attempt state explicit as [`None`]:

```splitscript
# state "game.exe" {
#     level: u32 at 0x1000;
# }
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
[`None`]; nonnegative values map to the corresponding [`u64`]. Do not cast the
signed sentinel into an unsigned index. A skipped segment advances the index,
and after the final split the index equals the route's segment count. The index
is therefore authoritative route progress, not a count of splits requested by
this autosplitter.

The compiler recognizes `timer.CurrentSplitIndex`, but deliberately does not
rewrite it automatically because the correct [`None`] behavior depends on the
surrounding control flow. Use [`else`] for an early fallback or [`match`] when the
absent state needs distinct behavior.

## Load removal and computed game time

Use [`isLoading`] when the game exposes whether its own clock should be paused.
Return `true` while loading and `false` while gameplay is advancing. When a
sentinel means the script has no trustworthy observation for this tick, fall
through or return [`None`] so the timer keeps its current pause state:

```splitscript
state "game.exe" {
    loadingState: i32 at 0x1000;
}

isLoading {
    if current.loadingState < 0 {
        return None
    }
    return current.loadingState != 0
}
```

This third state is intentionally different from `false`: `false` actively
resumes game time, while [`None`] leaves the previous host state unchanged. A
bare [`return`] and ordinary fallthrough also produce [`None`]. Prefer this action
for regular load removal rather than repeatedly calling
[`timer.pauseGameTime()`] and [`timer.resumeGameTime()`].

When the game exposes its own elapsed time, return a typed [`Duration`] from
[`gameTime`]. Direct values automatically become the present side of the
optional action result; no `Some(...)` constructor is needed:

```splitscript
state "game.exe" {
    elapsedFrames: i64 at 0x1008;
}

gameTime {
    if current.elapsedFrames < 0 {
        return None
    }
    return Duration.fromFrames(current.elapsedFrames, 60)
}
```

Use the constructor that matches the game's representation, such as
[`Duration.fromSeconds`], [`fromMilliseconds`], [`fromFrames`], or `fromParts`.
Falling through from [`gameTime`] leaves the host's last value unchanged; it
does not set zero. [`isLoading`] runs before [`gameTime`] on each eligible update,
so the two actions may be combined when the game provides both an independent
loading flag and an authoritative elapsed clock.

These actions report script-owned observations to the timer host. They do not read
back `timer.CurrentTime.GameTime`, which may have been changed by the host or
another component and remains a separate host-contract requirement below.

## LiveSplit timer metadata and control

Several legacy paths inspect host-owned timer data rather than the attached
game. They do not currently have faithful SplitScript replacements:

- `timer.CurrentTime.GameTime` reads LiveSplit's optional game-time clock. If
  this autosplitter computes the value, keep that [`Duration`] in script-owned
  state and also return it from [`gameTime`]; the host's possibly externally
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
`Stopwatch` only to measure time since a game event. Use [`Instant`] for that
process-independent elapsed time:

```splitscript
# state "game.exe" {
#     inMenu: bool at 0x1000;
# }
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

[`Instant.now()`] reads a monotonic host clock. It never moves backwards during
one autosplitter instance and has no meaningful absolute or calendar value.
[`elapsed()`], `durationSince(...)`, and `hasElapsed(...)` produce or compare
exact [`Duration`] values, making them appropriate for cooldowns, debouncing,
delayed splits, and retry deadlines. They continue advancing independently of
LiveSplit's loading and pause state.

Do not mechanically replace `timer.CurrentTime.RealTime`. That ASL expression
may mean the current LiveSplit attempt's run-relative time rather than an
independent delay. If the original logic starts its measurement at a game event,
capture an [`Instant`] there. If it needs the actual timer phase for an offset,
run-age check, or custom game-time calculation, the current host contract does
not expose equivalent metadata. The compiler diagnoses these paths separately
so this distinction is not hidden by a convenient but incorrect rewrite.

## Retrying an attach-time read transaction

ASL ports sometimes use a hand-written `while (true)` loop to repeat several
dependent reads until a complete pointer chain or metadata group becomes
available. In SplitScript, use one [`retry`] expression instead. Braces are an
ordinary value-producing expression, so the block naturally creates one local
failure boundary for every [`?`] inside it:

```splitscript
# state "game.exe" {}
onAttach {
    let healthAddress = retry {
        let manager = process.read<address>(0x1000)?
        let player = process.read<address>(manager.add(0x20))?
        player.add(0x18)
    }
    print(`health address {healthAddress}`)
}
```

A failed read starts the complete block again on the next attached update; no
locals from the failed attempt survive. A final [`T!`] error or [`throw`] has
the same retry behavior. [`return`] still exits the surrounding function, and
[`break`] or [`continue`] still target their lexical loop. Keep one attempt
synchronous and bounded: [`await`] and nested [`retry`] are rejected inside the
operand. Await scans, module discovery, and other intrinsically asynchronous
operations before entering the retry block.

## Attach-time-discovered addresses

Keep discovery in [`onAttach`] and polling in the state declaration. Declare an
attachment-discovered value with a bare top-level [`let`], without a fake zero
or [`None`] initializer. Its type is inferred from assignments and uses. The
compiler proves that every successful attachment path initializes it before
polling begins, and the runtime clears its storage when that process detaches.
An unsupported build should await process closure instead of completing:

```splitscript
let loadingAddress

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

With named layouts, a bare global may belong to only the layouts whose return
paths initialize it. Access it under the same direct [`match`] on [`layout`]
that refines layout-specific state fields. Helpers inherit these requirements,
so a helper reading a Steam-only value is callable from the
`StateLayout.Steam` arm but not from unrefined polling code. Values assigned on
every successful layout path remain available everywhere while attached.

Expression-backed fields are persistent watcher values. The initial snapshot
waits for every required field to succeed together. Afterwards, a failed [`T!`]
retains that field's last accepted value while successful siblings advance. If
several values must advance atomically, read them as one record- or array-valued
state field. For a field that is semantically absent on some ticks, declare
[`T?`] and convert just that read:

```splitscript
state "game.exe" {
    requiredLevel: u32 at "game.exe", 0x1000;
    optionalBonus: u32? at "game.exe", 0x2000;
}
```

For a static pointer path, the explicit [`T?`] annotation maps a failed module
lookup, pointer traversal, final read, or decoder to a successfully accepted
[`None`]; success produces `Some(T)`. This differs from a required `T` field,
whose failed result retains the last accepted value after initialization. The
maintained Aquanox port uses `String? at ... as utf8(32)` because the original
ASL watcher becoming `null` is itself the manual level-end signal.

The same policy remains available to a discovered-address expression with
`process.read<T>(address).discardError()`. Prefer direct `T? at` syntax when the
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
large scan does not block the autosplitting runtime's update loop. Do not translate that worker
or its cancellation token. SplitScript scans are already asynchronous: they
inspect only a bounded memory window per tick, yield to the host between
windows, preserve their cursor, and are discarded automatically if the
attached process closes.

Choose the narrowest range justified by the source:

```splitscript
# state "game.exe" {}
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

Use [`Module.scan`] when a pattern belongs to one known image and `process.scan`
for another explicit address range. Use `process.scanMemory` only when the
legacy source genuinely enumerates readable mappings or the target may live
outside known modules. [`scanAny`] and [`scanMemoryAny`] accept an array of
signatures and return both the address and selected index, which keeps fallback
layout selection in one cooperative pass.

When Windows-native discovery starts from an exported runtime entry point,
resolve that exact symbol through the module instead of scanning the entire
process:

```splitscript
# state "game.exe" {}
# onAttach {
let mono = await process.module("mono-2.0-bdwgc.dll")
let assemblyForeach = mono.peExport("mono_assembly_foreach") else return
# print(assemblyForeach)
# }
```

[`peExport`] validates PE table bounds and rejects forwarded exports. It is
deliberately PE-specific; ELF and Mach-O symbols need their own proven parser or
a future portable host contract rather than being mislabeled as PE exports.

When an ASL helper enables `LoadSceneManager` and reads `Scenes.Active` or
`Scenes.Loaded`, discover SplitScript's typed scene manager once during
attachment and poll immutable snapshots through the state block:

```splitscript
let sceneManager

state "game.exe" {
    activeScene = sceneManager.activeScene();
    loadedScenes = sceneManager.loadedScenes();
}

onAttach {
    sceneManager = await Unity.sceneManager()
}

isLoading {
    if current.loadedScenes.isEmpty() {
        return None
    }
    let firstLoaded = current.loadedScenes[0]
    return current.activeScene.name != firstLoaded.name
}
```

[`Unity.sceneManager`](fn@Unity.sceneManager) supports the UnityPlayer layouts
covered by ASR: 32-bit and 64-bit Windows players and 64-bit Linux and macOS
players. [`UnitySceneManager.activeScene`](method@UnitySceneManager.activeScene)
and [`UnitySceneManager.loadedScenes`](method@UnitySceneManager.loadedScenes)
copy each scene's native address, signed build index, asset path, and name. This
is intentionally a snapshot rather than a live scene handle: [`old`] remains
stable even after Unity unloads or reuses the original native object. Failed or
incomplete reads reject that state field update and retain its preceding value;
they do not publish a partially populated loaded-scene array. The signed index
preserves Unity's initialization value of `-1`.

For modern 64-bit Windows Unity games using Mono, prefer the typed metadata
provider over copying `asl-help` callbacks or raw class-layout traversal into
the script:

```splitscript
# state "game.exe" {}
# onAttach {
let mono = await Unity.mono(MonoVersion.V2)
let image = await mono.image("Assembly-CSharp")
let data = await image.class("AutoSplitterData")
let runningAddress = await data.staticField("isRunning")
# print(runningAddress)
# }
```

[`MonoVersion.V3`] selects the Unity 2021.2-and-newer PE64 layout; `V2` selects
the preceding modern layout. These are explicit target-memory contracts, not
automatically detected marketing versions. When a static field holds a
replaceable managed singleton, retain the slot as a path and append the
instance field:

```splitscript
# state "game.exe" {}
# let valuePath: MemoryPath? = None
# onAttach {
# let mono = await Unity.mono(MonoVersion.V2)
# let image = await mono.image("Assembly-CSharp")
# let data = await image.class("AutoSplitterData")
let singleton = await data.staticFieldPath("script")
let valueOffset = await data.field("value")
valuePath = singleton.dereference(valueOffset as i64)
# }
```

This rereads the singleton pointer whenever the state field resolves the path,
rather than caching an attachment-time object address.

Metadata offsets may come from different classes. For example, a generic base
class can declare a static singleton slot while the concrete derived class owns
the static storage and declares the instance value. Compose those facts
explicitly instead of requiring one class wrapper to hide the inheritance:

```splitscript
# state "game.exe" {}
# let levelStatePath: MemoryPath? = None
# onAttach {
let mono = await Unity.mono(MonoVersion.V2)
let core = await mono.image("com.unity-common.core")
let service = await core.class("Service`1")
let game = await mono.image("Assembly-CSharp")
let levelFlow = await game.class("LevelFlowService")
let staticTable = await levelFlow.staticTable()
let instanceOffset = await service.field("_instance")
let stateOffset = await levelFlow.field("_state")
levelStatePath = staticTable
    .memoryPath([], instanceOffset as i64, mono.pointerSize)
    .dereference(stateOffset as i64)
# }
```

[`MonoClass.staticTable`](method@MonoClass.staticTable) selects the concrete
class's storage, [`MonoClass.field`](method@MonoClass.field) supplies each
declaring class's offset, and
[`MemoryPath.dereference`](method@MemoryPath.dereference) performs the managed
reference read. Older V1, 32-bit, ELF, and Mach-O Mono targets remain future
layout families rather than falling back to a guessed offset set.

When a port needs the mapping metadata itself, take a typed snapshot rather
than reproducing the host's numeric count/index ABI:

```splitscript
# state "game.exe" {}
# onAttach {
let ranges = process.memoryRanges()
for range in ranges {
    if range.readable && range.executable {
        debug print(`executable mapping at {range.address}`)
    }
}
# }
```

[`memoryRanges`] is synchronous because it only copies cheap host metadata.
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
filter a transient sentinel. The direct port is valid in [`whileAttached`]:

```splitscript
# state "game.exe" {
#     scene: i32 at 0x1000;
# }
whileAttached {
    if current.scene == 7 || current.scene == 8 {
        current.scene = old.scene
    }
}
```

[`current`] is the one mutable snapshot root; [`old`] remains read-only history.
An assignment is visible to later code and later lifecycle actions in the same
tick, and the mutated snapshot naturally becomes [`old`] on the next successful
poll. Compound forms such as `current.count += 1` use the field's ordinary
typed operator.

Use a state-field filter instead when the candidate itself is invalid. This is
the stronger transactional form: it can reject the first candidate before any
snapshot is published, and it keeps the acceptance rule beside the read.

Add an ordinary trailing [`if`] to that pointer-path field:

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

For example, the filter can retain a scene during loading scenes 7 and 8 while
an independently read entity count continues to advance. By contrast, an ASL
`update` block that returns `false` does not roll back state at all; it skips
lifecycle decisions after the refresh. SplitScript does not add a separate
lifecycle concept for that behavior until a maintained port demonstrates that
ordinary field expressions and [`whileAttached`] cannot represent the required
result clearly.

## Collection search and run-scoped sets

C# array `.Length` maps directly to the SplitScript method `.length()`. It
returns the [`u32`] element count for both [`[T]`] and fixed [`[T; N]`] arrays. The
compiler can apply this rename automatically, after which signed C# index
arithmetic may still need an explicit width cast.

After choosing a SplitScript collection shape, C# `.Count` also becomes
`.length()`. For an array this is the element count; for [`Set<T>`] it is the
number of unique stored values.

C# `List<T>` maps to SplitScript's [`[T]`] array type. SplitScript will not add a
separate compatibility-shaped `List<T>`. [`[T]`] is the variable-length ordered
sequence, while [`[T; N]`] carries an exact fixed length for layouts and other
code where the size is part of the type. Use [`Set<T>`] only when the original
data is genuinely an unordered collection of unique values, not merely because
its current API happens to be mutable.

Arrays provide `contains` and `indexOf` when their elements support equality:

```splitscript
# state "game.exe" {
#     level: i32 at 0x1000;
# }
let levelRoute = [12, 5, 6, 7, 9, 10, 11, 14]

split {
    let oldIndex = levelRoute.indexOf(old.level) else return false
    let currentIndex = levelRoute.indexOf(current.level) else return false
    return currentIndex == oldIndex + 1
}
```

Unlike C# `List<T>.IndexOf`, `indexOf` returns `u32?`; absence is [`None`], never
a signed `-1` sentinel. Replace an existing element with ordinary indexed
assignment; the index is [`u32`], aliases observe the change, and an out-of-range
index traps just like an indexed read:

```splitscript
# state "game.exe" {}
# onAttach {
# let route = [1, 2, 3]
# let currentIndex: u32 = 1
# let nextLevel = 7
route[currentIndex] = nextLevel
route[currentIndex] += 1
# print(route[currentIndex])
# }
```

Plain indexed assignment evaluates the collection and index once. Compound
forms such as `route[nextIndex()] += 1` additionally evaluate the right operand
once and use the same typed operator as an ordinary `+=`; `nextIndex()` is not
called twice. Growable [`[T]`] supports [`push`], [`extend`], indexed [`removeAt`],
optional [`pop`], first-match `remove(value)`, and capacity-preserving `clear`. C#
`list.AddRange(values)` becomes
`list.extend(values)` once both collections are represented as typed arrays;
self-extension duplicates the original elements once. Successful structural
operations invalidate active iteration. C# `RemoveAt(index)` maps directly to
`removeAt(index)`; an out-of-range [`u32`] index traps just like array indexing.
Use `let last = values.pop() else ...` where C# removes a final list or stack
element: SplitScript returns [`None`] for an empty array instead of throwing.
`List<T>.Remove(value)` maps to `remove(value)`, removes only the first equal
element, and returns whether a match existed. Ignoring that boolean is valid
when the source does not distinguish absence.
[`[T; N]`] remains fixed-length and supports none of these operations.

Use [`Set<T>`] when values are discovered while the run progresses and only
membership matters:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
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

[`insert`] returns true only for a new value. The set object and its contents
persist across ticks and detachments until explicitly cleared or the script is
unloaded. Clear it at the lifecycle boundary that matches the original source:
[`onAttach`] for per-process state, or a detected timer-start transition for
per-attempt state. The maintained OpenJK-Speed port exercises the former.

## Bounded integer iteration

For bounded integer iteration, use an explicit SplitScript range instead of
constructing an array of indices:

```splitscript
# state "game.exe" {}
# let count: u32 = 4
# whileAttached {
for index in 0u32..<count {
    print(index)
}
# }
```

[`..<`](syntax@range) excludes the upper endpoint and
[`..=`](syntax@range) includes it. This differs from
Rust, where bare `..` is the exclusive spelling, and from C# range syntax,
where `..` describes slicing bounds rather than iteration. SplitScript requires
the `<` or `=` marker; writing bare `..` produces a diagnostic with both
machine-applicable choices. The type itself uses the same explicit shape,
[`T..<T`](syntax@range) or [`T..=T`](syntax@range), when a range is stored or
passed to a helper. A direct loop
does not allocate a collection or range object.

When membership comes from a small closed enum, a typed bit set remains more
compact and makes the finite domain explicit:

```splitscript
# state "game.exe" {}
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

Detect the timer transition in [`whileAttached`], clear the bit set, and mark the
starting map before [`split`] is evaluated. This reproduces an ASL `timer.OnStart`
handler without a separate event API. The generated update loop runs
[`whileAttached`] before timer-decision actions.

Growable ordered storage, insertion order, and repeated equal values all belong
to [`[T]`]. They do not justify another collection type. Record any still-missing
specific operation rather than describing `List<T>` itself as missing. Indexed
insertion remains deferred until a maintained port demonstrates that it is
needed.

## Static settings declarations

Move ASL `settings.Add(...)` registration into the declarative [`settings`]
block. The quoted text before `=>` is the user-facing label, the identifier is
the statically typed source member, and an optional `key "..."` preserves the
exact stable string stored in the host settings map:

```splitscript
enum Route {
    AnyPercent,
    AllBosses,
}

settings {
    /// Splits when a configured game event occurs.
    "Enable Auto Splitting" => autoSplit key "auto-split": true,
    /// Selects the route-specific split rules.
    "Route" => route: choice {
        "Any%" => Route.AnyPercent default,
        "All Bosses" => Route.AllBosses,
    },
}

state "game.exe" {
    bossDefeated: bool at 0x1000;
}

split {
    return settings.autoSplit
        && settings.route == Route.AllBosses
        && !old.bossDefeated
        && current.bossDefeated
}
```

Consecutive [`///`] documentation comments become the setting tooltip. The
legacy comma-separated tooltip string is deliberately not accepted. Boolean
settings are [`bool`]; a `choice` is the source enum named by its variants. One
choice entry must carry `default`. Quoted groups add visual hierarchy only:
they do not disable their children, so preserve an ASL parent checkbox by
testing that boolean explicitly in the split condition.

File selectors produce a [`String`] path and can declare extension globs, a
catch-all `_`, and MIME filters. They remain typed settings rather than direct
filesystem access:

```splitscript
state "game.exe" {}

settings {
    "Paths" {
        /// Selects the layout consumed by the host integration.
        "Layout File" => layoutFile: file {
            "Layout files" => "*.json *.yaml",
            _ => "*.*",
            mime => "application/json",
        },
    },
    /// Reloads the selected layout after the user changes this option.
    "Live Reload" => liveReload: false,
}

whileAttached {
    if settings.liveReload != oldSettings.liveReload {
        print(`Live reload: {settings.liveReload}`)
    }
    setVariable("Layout", settings.layoutFile)
}
```

[`settings`] and [`oldSettings`] are complete current and previous views refreshed
once per update, so a comparison detects user changes without caching a second
copy. Statically known values should use their typed members. When a data table
selects among boolean keys, use `settings.enabled(key)`; use
`settings.contains(key)` when declaration membership and a disabled value must
remain distinct. Literal keys are checked and completed against this file's
declarations, including explicit `key` strings.

## Finite settings families

Prefer direct `settings.name` access when the setting is known statically. For
data tables whose entries select among declared boolean settings, give each
declaration its exact host-map string with `key "..."` and use
`settings.enabled(key)`. This remains boolean-only and is not a dynamically
typed replacement for choice or file settings. Literal keys are validated and
completed against the declarations; computed unknown keys return false. If the
original settings have a boolean parent, gate the child result explicitly; a
quoted SplitScript heading is visual only. The complete A Plague Tale example
preserves its **All Chapters** parent semantics this way.

When cursor advancement depends on declaration membership rather than whether
the split is enabled, keep the two questions separate:

```splitscript
# state "game.exe" {}
# settings {
#     "Checkpoint" => checkpoint: true,
# }
# let checkpointIndex = 0
# split {
# let checkpointKey = "checkpoint"
if settings.contains(checkpointKey) {
    checkpointIndex += 1
    return settings.enabled(checkpointKey)
}
# return false
# }
```

`contains` recognizes declared boolean, choice, and file keys, including
explicit `key "..."` spellings. It returns false for visual headings and unknown
keys. This matches legacy `Settings.ContainsKey` without exposing a dynamically
typed host map.

When legacy `startup` creates a bounded numbered family, declare it at compile
time rather than expanding dozens of source members or mutating the settings
map:

```splitscript
# state "game.exe" {}
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

A source loop with a small number of exceptional defaults is still finite
declaration data, not runtime settings registration. Partition the uniform
ranges and declare each exception directly:

```splitscript
# state "game.exe" {}
settings {
    "Levels" {
        for level in 1..=2 {
            `Level {level}` key `{level}`: false,
        },
        "Level 3" => level3 key "3": true,
        for level in 4..=5 {
            `Level {level}` key `{level}`: false,
        },
    },
}
```

All five host keys remain stable and available to
[`SettingsView.enabled`](method@SettingsView.enabled). Runtime registration is
needed only when the set of keys itself is discovered after compilation, not
merely because the ASL source used a [`for`] loop to declare a bounded table.

## Snapshot-dependent helper functions

Ordinary helper functions may refer to [`old`] and [`current`] directly, just as a
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
contexts where a complete pair of snapshots exists: [`whileAttached`] and the
timer-decision actions. Calling one from [`setup`], [`onAttach`], [`onDetach`], a
state source, or a state filter produces a focused diagnostic. This keeps the
concise ASL helper shape without exposing default-initialized or stale state.

Pass snapshots explicitly when the helper should operate on caller-selected
history or when its argument order is part of the helper's meaning:

```splitscript
state "game.exe" {
    level: u32 at 0x1000
}

fn levelChanged(before, after) {
    return before.level != after.level
}

fn enteredLevel(before, after, level) {
    return levelChanged(before, after) && after.level == level
}

split {
    return enteredLevel(old, current, 7u32)
}
```

The field accesses and calls infer `before` and `after` as the generated
`StateSnapshot` type. They are ordinary read-only values, so the caller may
choose which available snapshots represent each role and may forward them
through more helpers. Passing snapshots removes the helper's implicit snapshot
dependency; only evaluating [`old`] and [`current`] at the call site requires a
committed pair. Direct access remains the concise choice when a helper always
means this tick's transition.

## Legacy ASL lifecycle blocks

The similarly shaped block names are not interchangeable. The original
LiveSplit component invokes them at different boundaries:

| ASL construct | Exact legacy timing | SplitScript direction |
| --- | --- | --- |
| `startup` | Once when the script is loaded, before process attachment | Put settings in [`settings`], constant data in global initializers, and remaining process-independent statements in [`setup`]. |
| `init` | Once for each found process, after one legacy state refresh; a failure retries attachment initialization | Put suspending discovery and layout selection in [`onAttach`]. Put synchronous work that consumes the first complete snapshot in [`onStateReady`]. |
| `update` | After each refresh and before all timer decisions; `false` skips the remaining decisions for that tick | Put ordinary per-tick work in [`whileAttached`]. An explicit `return false` preserves the legacy control result exactly. |
| `exit` | When the attached process exits | Use [`onDetach`]. It runs exactly once for a real process closure and never at initial detached startup. |
| `shutdown` | When the script is disabled, reloaded, dropped, or LiveSplit exits | No exact host callback exists yet; do not approximate it with [`onDetach`]. |
| `timer.OnStart`, `timer.OnSplit`, `timer.OnReset` | LiveSplit timer events, which may be raised independently of this script's decision blocks | Reconstruct only simple observable transitions in [`whileAttached`]. Exact lossless events require the planned host contract. |

For example, process-independent ASL startup statements belong in [`setup`], not
[`onAttach`]:

```splitscript
# state "game.exe" {}
setup {
    print("Autosplitter loaded")
}
```

[`setup`] runs at the beginning of the module's first interruptible host update,
after settings are available, but cannot use `process`, `gba`, [`current`],
[`old`], [`await`], or [`retry`]. A debug-watch replacement loads a new module and
therefore runs it again on that module's first update.

Legacy `init` combines two boundaries that SplitScript keeps explicit. Use
[`onAttach`] for discovery that may suspend and for layout selection. Use
[`onStateReady`] for synchronous initialization that needs polled state:

```splitscript
# state "game.exe" {
#     level: u32 at 0x1000;
# }
# let gameManager: address = 0x0
onAttach {
    let unity = await Unity.il2cpp(2020)
    let image = await unity.image("Assembly-CSharp")
    let gameManagerClass = await image.class("GameManager")
    gameManager = await gameManagerClass.staticInstance(["Instance"])
}

onStateReady {
    print(`Initial level: {current.level}`)
}
```

[`onStateReady`] runs once per attachment only after every field in the first
snapshot was read and accepted. [`old`] and [`current`] are both that snapshot, so
initialization cannot look like a transition from default values. It cannot
suspend. [`whileAttached`] and timer-decision actions begin on the next update.

Legacy `update { return false; }` maps directly to [`whileAttached`]. The state
snapshot has already refreshed, but the remaining timer decisions are skipped
for that update:

```splitscript
# state "game.exe" {}
# let helperLoaded = true
whileAttached {
    if !helperLoaded {
        return false
    }

    // Per-update bookkeeping.
}
```

Falling through, a bare [`return`], or `return true` continues to [`start`],
[`isLoading`], [`gameTime`], [`reset`], and [`split`] as applicable. This control result
does not reject or roll back the refreshed snapshot.

ASL `refreshRate` is a frequency. Migrate a stable attached cadence to the
declarative lifecycle policy:

```splitscript
# state "game.exe" {}
tickRate {
    attached: 60,
}
```

SplitScript defaults to 120 Hz while attached and 1 Hz while detached. It
applies the attached rate before [`onAttach`] begins, which is important when
module or signature discovery suspends across updates, and restores the
detached rate before [`onDetach`]. Add `detached: value` only when the 1 Hz
default is unsuitable. Use [`setTickRate`] only when the rate must change
dynamically within one attachment; the next lifecycle transition reapplies the
declaration.

## Process-exit game-time cleanup

ASL commonly pauses game time in `exit`. Map that cleanup directly to
[`onDetach`]:

```splitscript
# state "game.exe" {}
onDetach {
    timer.pauseGameTime()
}
```

The compiler invokes this block once after clearing the closed handle, provider
state, selected layout, and pending process-lifetime continuations. Neither
`process` nor state snapshots are available in [`onDetach`]: a process may close
before attachment initialization or the
first state poll completes.

Use [`isLoading`] for ordinary load removal. [`timer.pauseGameTime()`] and
[`timer.resumeGameTime()`] are explicit lifecycle tools, not a replacement for
that declarative action.
