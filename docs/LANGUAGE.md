# SplitScript language design

## Design center

SplitScript combines familiar expression syntax with ASL's domain concepts
rather than trying to be source-compatible with JavaScript, C#, or old ASL.

| General scripting familiarity | Autosplitter-specific syntax |
| --- | --- |
| `let`, braces, property access | Direct `start`, `split`, `reset`, `isLoading`, `gameTime` blocks |
| `==`, `!=`, `&&`, `||`, `if` | `state`, `settings`, `current`, `old` |
| Inference by default, annotations when useful | Declarative pointer paths and automatic polling |
| Familiar `await` expressions | Process-lifetime cancellation and tick-based retry |

It is statically typed. There is no JavaScript-style numeric supertype and no
implicit widening between integer widths. This is important when values come
from process memory.

Inference flows through global uses and assignments as well as initializers.
An unannotated mutable global initialized to `None` becomes `T?` when a later
ordinary assignment supplies a `T`; a standalone `None` global remains the
zero-sized `None` type. An explicit annotation remains available when it makes
the intended stored type clearer.

Closed comma-separated forms use punctuation rather than line breaks to
separate items. This includes arguments, array and record literals, match arms,
record fields, enum variants, state layouts, settings, choice options, and file
filters. A trailing comma is always optional. The formatter adds one when it
lays a list out across multiple lines and omits it for a compact one-line list.
State fields are instead separated by semicolons because their unclosed pointer
paths already use commas between offsets; the final semicolon is optional and
the formatter adds it in multiline state blocks. Ordinary statements remain
newline-terminated, with semicolons available when multiple statements share a
line.

## State and pointer paths

```text
state "game.exe" {
    level: u16 at "game.exe", 0x1234, -0x20;
    score: u32 at 0x7ff612341000;
}
```

To attach to the first available edition of a game, provide an ordered process
list. Each name is attempted once per tick until one attaches.

```text
state ["game.exe", "game-demo.exe"] {}
```

With a module name, the first signed `i64` offset is added to the module base.
Every remaining signed offset follows a 64-bit pointer, adds the offset, and
continues. Without a module name, the first value is a full-width unsigned
absolute address and only subsequent offsets are signed. Address addition wraps
modulo the 64-bit address space. The final read uses the declared width and
signedness. A failed path rejects that field's candidate value. An offset whose
magnitude does not fit `i64` is rejected instead of silently changing its sign.

State field annotations are optional and participate in whole-program
inference. Expression-backed fields normally obtain their type directly from
the right-hand side. A pointer-path field has no typed right-hand side, so its
type must come from a `current`/`old` use or an explicit annotation. The
compiler reports an ambiguity if neither provides enough information.

After attachment, initialization waits for one poll in which every required
field succeeds. That snapshot initializes both `old` and `current`, and
lifecycle actions begin on the following poll. Consequently action code never
observes synthetic zero-filled state. Later, each successful field advances;
a failed field retains its last accepted value while successful sibling fields
still advance. The resulting snapshots are WebAssembly GC structs, so action
code uses typed references rather than a linear-memory state layout.

Some watchers use failed memory access as meaningful absence rather than a
transient error. Write that choice explicitly with an optional pointer field:

```text
state "game.exe" {
    requiredMenu: String at 0x1000 as utf8(32);
    optionalMenu: String? at 0x2000 as utf8(32);
}
```

The required field keeps the initialization/retention behavior above. The
optional field accepts a failed module lookup, pointer traversal, final memory
read, or decoder as `None`; a successful read is `Some(T)`. It therefore never
blocks initialization and can visibly transition between a value and `None`
in `old` and `current`. The `T?` annotation is mandatory because it selects
failure semantics in addition to constraining the inferred value type.

A pointer-path field can use an ordinary trailing `if` expression to accept the
raw value or produce an error. Expression-backed fields
already have an ordinary right-hand side and should put the `if` there instead:

```text
state "game.exe" {
    scene: i32 at 0x1000 if value == 7 || value == 8 {
        Err("transient loading scene")
    } else {
        value
    };
    entities: i32 at 0x2000;
}
```

Inside this field-local expression, the read-only `value` binding has the
field's inferred type. A plain value is accepted and `Err(message)` rejects the
candidate. Before initialization, any rejected required field leaves state
uninitialized. Afterwards, rejection retains that field's last value without
discarding a new `entities` value from the same poll. Snapshot `current` and
`old` values stay read-only and are available only after initialization.

Games with multiple supported memory layouts can name each layout inside one
state declaration. Fields present in every layout with a compatible type form
the common snapshot interface; field order may differ. A missing field or a
same-named field with a conflicting type remains specific to its layout.

```text
state "game.exe" {
    layout Steam {
        level: u32 at 0x1000;
    },
    layout GOG {
        level: u32 at 0x2000;
    },
}

onAttach {
    let module = await process.mainModule()
    if module.size == 0x1000 {
        return StateLayout.Steam
    }
    if module.size == 0x2000 {
        return StateLayout.GOG
    }
    await process.closed()
}
```

Named layouts generate the enum `StateLayout` and the read-only value `layout`.
For such a state declaration, `onAttach` returns the selected enum variant;
state polling does not begin until it does. Detection remains ordinary typed
SplitScript, so it can use module metadata, signatures, reads, `await`, and
`retry` rather than being limited to a special module-size grammar. Awaiting
`process.closed()` is the explicit unsupported-build path: it keeps the current
process attachment inert until process-lifetime cancellation occurs. Later
lifecycle blocks can compare or exhaustively match `layout`.

Layout-specific fields become available when a direct `match layout` arm proves
which memory layout is active. The refinement applies to both `old` and
`current`, because the selected layout is stable for the attachment:

```text
state "Ronin.exe" {
    layout V8 {
        loading: i32 at 0x100;
        bike: i16 at 0x104;
    },
    layout V9 {
        loading: i32 at 0x200;
        bike: u16 at 0x204;
    },
}

isLoading {
    return current.loading == 1
}

split {
    return match layout {
        StateLayout.V8 => old.bike != 21_368 && current.bike == 21_368,
        StateLayout.V9 => old.bike != 52_688 && current.bike == 52_688,
    }
}
```

The incompatible `bike` declarations occupy distinct typed fields in the Wasm
GC snapshot. They are not optional and the compiler does not synthesize a
default to hide the physical difference. Accessing `current.bike` without a
layout refinement is an error.

## Variables and inference

```text
fn consumeU64(value: u64) {}

let tickCount = 0

whileAttached {
    consumeU64(tickCount) // the call infers tickCount as u64
}

split {
    let changed = current.level != old.level; // inferred bool
    return changed;
}
```

There is one declaration form: `let`. The compiler assigns fresh type variables
to unannotated values and unifies constraints from assignments, operators,
arrays, function bodies, return values, and call sites. Information therefore
flows in either direction: both `value == 0` and `0 == value` infer the literal
from `value`. This applies equally to top-level persistent variables: their
types may come from their initializer or from any later use. An annotation is
only needed to constrain an otherwise ambiguous value or document an important
boundary.

Record member access participates in the same whole-program inference. The
compiler can defer resolving a field path until a later call site supplies the
parameter's nominal record type, so helpers do not need redundant annotations:

```text
fn levelTimeText(parts) {
    return `{parts.minutes as u32}:{parts.seconds as u32}`
}

levelTimeText(current.levelTimeParts)
```

Several accessed fields can jointly identify a unique record even without a
call-site constraint. If the remaining field set matches multiple nominal
records, the compiler reports those candidates instead of guessing.

Annotations and integer suffixes are constraints, not a routine requirement.
Suffixes such as `1u8`, `10i64`, and `0xffu32` remain available when a literal is
genuinely unconstrained or when an exact type should be documented. An
unresolved inference component defaults when it contains an unsuffixed literal
or has a specific numeric-kind constraint: integer literals and `Integer`
values default to `i32`, while floating-point literals and `Float` values
default to `f64`. An integer-looking literal required to satisfy `Float` also
defaults to `f64`. Broader capabilities such as `Numeric`, `Signed`,
`MemoryReadable`, or `Display` do not choose a representation on their own; an
otherwise ambiguous value needs an annotation. Memory reads are stricter: a
component constrained by `MemoryReadable` never uses a numeric default, even if
it also contains a literal or an `Integer`/`Float` constraint. The concrete
memory representation must come from an annotation, explicit generic argument,
or another exact type. Mutable `let` bindings are monomorphic. User functions
are also currently monomorphic per declaration; generalized polymorphic
functions will require Wasm signature specialization.

Decimal floating-point literals may use an exponent, such as `1e-45` or
`6.022e+23`. They are rounded once to their inferred `f32` or `f64` target and
must remain finite and nonzero when the written significand is nonzero.
Representable subnormal values are valid: `let value: f32 = 1e-45` produces the
smallest positive `f32` (bit pattern `0x00000001`), while
`let value: f64 = 5e-324` produces the smallest positive `f64`. A literal that
underflows its target to zero or overflows it to infinity is a type error rather
than silently changing the comparison value.

Hovering a decimal floating-point literal shows both its inferred width and
the exact rounded IEEE-754 bits. Use `f32.fromBits(bits)` or
`f64.fromBits(bits)` when the bit pattern itself is the source data, and
`.toBits()` for the inverse reinterpretation. These operations preserve signed
zero and NaN payloads and do not perform a numeric conversion.

```splitscript
let negativeZero = f32.fromBits(0x8000_0000u32)
let representation = negativeZero.toBits()
```

Assignments support `=`, the arithmetic compound forms `+=`, `-=`, `*=`, `/=`,
and `%=`, plus `|=`, `&=`, `^=`, `<<=`, and `>>=` for integers. A compound
assignment uses exactly the same operand typing and runtime operation as its
ordinary binary operator while resolving the destination only once.

Unread parameters, local variables, loop elements, match payloads, and
`await`/`retry` bindings produce non-fatal warnings. The analysis follows
resolved value identities, so shadowed names and method receivers are handled
correctly. A plain assignment is only a write and does not make a binding used;
a compound assignment also reads the previous value. Prefix an intentionally
unused name with `_`. The warning's quick fix chooses a non-conflicting
underscore-prefixed name and updates writes to that same binding.

The compiler also warns about private globals, functions, records, and enums
that cannot be reached from lifecycle behavior or the host-visible state and
settings interface. Reachability is transitive and follows resolved identities:
a helper called only by another dead helper is still unused, while types in a
reachable function signature and types nested inside a reachable record or enum
remain live. Debug statements participate in this analysis in both build
profiles so editor warnings do not change when publishing a release build.
Prefix an intentionally reserved declaration with `_` to suppress its warning.

Reachable records and enums receive member-level checks without cascading from
an entirely dead type. Accessing a record member reads that field; merely
constructing or deserializing the record does not. Constructing or matching an
enum variant observes that variant, and variants exposed by a choice setting
are host-visible. Structural `==` and `!=` observe every field or variant in the
recursively compared shape. Unobserved fields and variants produce non-fatal
warnings with their exact declaration-name spans and support the same `_`
suppression convention.

Warning codes are stable tooling identifiers: `SS1001` denotes a discarded
must-use value, `SS1002` an unread local binding, `SS1003` an unreachable
declaration, and `SS1004` an unused record field or enum variant. The wording
may improve without requiring editor integrations to classify messages by
text.

Compiler hosts can configure every warning code as `allow`, `warn`, or `deny`.
Allowing suppresses that diagnostic, while denying makes the configured build
fail but keeps the original `SS100x` code and source information. This policy
does not change whether the source parses or type-checks, so editor semantic
features remain available for denied warnings. With `splitc`, repeat
`--allow`, `--warn`, or `--deny` followed by a code; use `warnings` to select
all warning codes. Later arguments override earlier selectors.

The language server offers preferred quick fixes that apply the `_`
suppression convention. For declarations and members, the action is a complete
validated rename: references in dead helper code and record-literal labels are
updated as well, name collisions gain additional underscores, and the edited
program must still preserve every resolved declaration identity.

Supported value types are:

- `bool`
- `char`, exactly one Unicode scalar value
- `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`
- `address`, a nominal 64-bit target-process address
- `f32`, `f64`
- `String`, an immutable UTF-8 WebAssembly GC array
- `[T]`, a mutable array whose length is not encoded in its type
- `[T; N]`, a mutable array with exactly `N` elements
- `T?`, an optional value containing either `Some(T)` or `None`
- `T!`, a result containing either `T` or a standard string error
- the built-in GC reference types `Duration`, `FileVersion`, and `Module`
- `Signature`, the compile-time-only type produced by `sig"..."`

Character literals use single quotes and contain exactly one Unicode scalar
value. They support the ordinary escaped characters plus `\u{...}` scalar
escapes. A `char` is distinct from an integer: it supports equality and
`Display`, and `value as u32` exposes its scalar value, but arbitrary integers
cannot be cast into characters. It is also not directly memory-readable,
because an address alone does not specify a character encoding.

```splitscript
let separator = '/'
let smile = '\u{1f642}'
print(`Character: {smile}`)
```

The usual arithmetic, comparison, logical, bitwise, and shift operators are
supported. Because values are statically typed, `==` and `!=` are unambiguous;
there are no coercing versus strict comparison variants.

Operators use Rust's relative precedence. From tightest to loosest, the
currently supported operators are unary operators, `as`, `*`/`/`/`%`, `+`/`-`,
`<<`/`>>`, `&`, `^`, `|`, comparisons, `&&`, and `||`. In particular, bitwise
operators bind more tightly than comparisons, so `level & 1 == 0` means
`(level & 1) == 0`. Comparisons share one precedence level and cannot be
chained without parentheses.

Every integer and floating-point type provides type-directed `min`, `max`, and
`clamp` methods. Arguments are inferred as the receiver's exact type and each
receiver and argument is evaluated once. `clamp(lower, upper)` is equivalent to
`value.max(lower).min(upper)` for correctly ordered bounds.

```text
let cappedStage = stage.min(7)
let nonNegative = score.max(0)
let normalized = amount.clamp(0.0, 1.0)
```

`if` conditions are expressions and do not require delimiter parentheses.
Parentheses remain available for ordinary expression grouping. `else if`
chains do not need a nested brace block.

```text
if level == 14 {
    act = "X"
} else if level & 1 == 0 {
    act = "1"
} else {
    act = "2"
}
```

`if` can also produce a value. An expression-valued `if` requires an `else`,
and both branches are inferred bidirectionally from each other and from the
surrounding expected type. Only the selected branch is evaluated.

```text
let label: String = if isDemo {
    "Demo"
} else if isDlc {
    "DLC"
} else {
    "Base Game"
}
```

`while` repeats a statement block as long as its `bool` condition remains true.
The condition is evaluated before every iteration, and declarations in the body
are scoped to that body.

```text
let index = 0
let total = 0
while index < 5 {
    index += 1
    total += index
}
```

`for name in array` visits each element of a general `[T]` or exact-length
`[T; N]` array. The array expression is evaluated once, the element type is
inferred in both directions, and the read-only element binding is scoped to the
loop body.

```text
for item in inventory {
    if item == ignoredItem {
        continue
    }
    inspect(item)
}
```

`for` supports the same `break`, `continue`, and fallback control flow as
`while`. A `for` body in `onAttach` may also use `await` or `retry`; the array,
next index, and current element are retained across suspension without
evaluating the array expression again.

`break` exits the nearest enclosing loop. `continue` skips the rest of the
current iteration and evaluates that loop's condition again. Both are
statements, and nested `if` blocks do not change which loop they target.

```text
while index < values.length {
    index += 1
    if index == ignoredIndex {
        continue
    }
    if total >= limit {
        break
    }
}
```

Like `else return`, loop control can be used directly as a diverging fallback
for an Option or Result. This works even when the fallback is nested inside an
expression-valued `if`, match arm, or short-circuit expression:

```text
let entry = process.read(address) else continue
let selected = if useFallback {
    optionalValue else break
} else {
    defaultValue
}
```

Inside `onAttach`, `await` and `retry` may suspend within a loop. The async
lowering gives each suspending loop explicit header and exit states, so a
resumed continuation can `break`, `continue`, or complete an iteration without
replaying work from earlier iterations. Nested suspending loops target their
nearest loop and preserve values live across each suspension.

## Debug-only statements

Prefix a statement with `debug` to retain it in debug builds and erase it from
release builds:

```text
debug print(`level {current.level}`)
debug if current.level == 7 {
    print("testing the final split")
}
debug attempts += 1
debug let inspectedLevel = current.level
```

Globals and functions can also be debug-only:

```text
debug let traceLimit = 10

debug fn traceLevel(level: i32) {
    debug let capped = level.min(traceLimit)
    print(`level {capped}`)
}

whileAttached {
    debug traceLevel(current.level)
}
```

A debug function may call ordinary or other debug functions. Debug statements
can declare locals with `debug let`, including values produced by `await` or
`retry` in `onAttach`; later debug statements in the same scope can use them.
An ordinary statement or ordinary function may not use a debug-only function,
global, or local because that reference would remain after its declaration is
erased. The compiler reports this at the reference site; wrapping the use in
`debug` establishes the required context. Release Wasm IR contains neither the
function body nor storage and initialization for a debug-only global.

Debug-only code is still parsed, resolved, and type-checked in release builds,
so a stale diagnostic path cannot silently rot. Erasure happens during semantic
Wasm lowering, before reachability, string collection, import selection, and
helper discovery. Consequently, a release module does not retain debug-only
messages or logging imports.

The statement form accepts bindings, expression statements, assignments, `if`,
`while`, and `await` or `retry` statements. It rejects `return`, `throw`,
`break`, and `continue` until profile-dependent termination rules are
specified.

Expression branches currently contain one expression rather than a sequence of
statements with a trailing value. A state-field assignment is a failure
boundary, so `?` can propagate a read error directly out of the selected branch:

```text
levelOrScene = if isDlcDemo {
    LevelOrScene.Scene(process.readManagedString(
        process.read(gameManager.offset(levelOrSceneOffset))?,
        16
    )?)
} else {
    LevelOrScene.Level(
        process.read(gameManager.offset(levelOrSceneOffset))?
    )
}
```

## Optional and result values

`T?` represents an optional value and `T!` represents an operation that can
fail with the language's standard string error. `None` constructs an empty
option and `Err(message)` constructs a failed result. A plain `T` is lifted
automatically when `T?` or `T!` is expected. `Some(value)` and `Ok(value)` are
available when the wrapper state should be explicit, but are never required.
Because their payload supplies `T`, `Some(value)` and `Ok(value)` can infer a
wrapper type without an annotation. `None` and `Err(message)` still need
expected-type context because they contain no value from which to infer the
missing `T`.

Both wrappers support structural `==` and `!=` when `T` itself supports
equality. Two empty Options are equal; an empty and present Option are not; two
present Options compare their values. Results first compare whether they are
successes or errors. Successes compare their values, errors compare their
error strings by content, and a success never equals an error. This composes
through records and enums that contain wrapper fields or payloads.

Standard-library operations that return a value without mutating their receiver
are must-use by default. Writing such a call as a bare expression statement
produces a warning because discarding its only useful outcome is normally a
mistake. Mutating operations such as `Set.insert` remain intentionally
discardable even when they return status information. Individual declarations
can provide a more specific explanation; immutable string transforms do so to
make clear that they return a new string.

`Option` and `Result` additionally carry a must-use obligation on the value
type itself, because absence or failure must not be silently ignored. Using a
must-use value in an assignment, argument, return, match, `else`, or `?`
consumes it. The warning is non-fatal: debug watch and release builds still emit
their Wasm artifact.

Both wrappers can be matched exhaustively with explicit state patterns:

```text
fn describeOptional(value: i32?) -> String {
    return match value {
        None => "none",
        Some(present) if present > 10 => "large",
        Some(present) => present as String
    }
}

fn describeResult(value: i32!) -> String {
    return match value {
        Err(error) => `error: {error}`,
        Ok(success) => success as String
    }
}
```

The binding inside `Some` has type `T`; the binding inside `Ok` has type `T`;
and the binding inside `Err(error)` has type `String`. `_` matches either state without
binding a value. Exhaustiveness checks require both states unless `_` is
present. A guard narrows when an arm applies but does not count as covering its
state, so the guarded `Some` arm above still needs the following unguarded
arm. Payload extraction and guard evaluation happen only after the pattern's
wrapper state matches.

Postfix `?` unwraps a `T!` or propagates its error to the nearest failure
boundary. State-field assignments are implicit boundaries. A function returning
`T!` is also a boundary, so differently typed failures can pass through without
manually rebuilding `Err`:

```text
fn readMode(address: address) -> i32! {
    let object = process.read(address)?
    return process.read(object)?
}
```

Actions such as `whileAttached` are not result boundaries, so an unhandled `?` there is
a compile-time error. The propagation operation is represented explicitly in
typed HIR with its target result type; it is not treated as a zero/default read.

An explicit `throw error` statement transfers a `String` error through the same
boundary mechanism. It is currently available in functions returning `T!`:

```text
fn requirePositive(value: i32) -> i32! {
    if value < 0 {
        throw "expected a positive value"
    }
    return value
}
```

`throw` and the error arm of `?` lower to the same failure-transfer operation.
Explicit nested `catch` boundaries are planned; until then, an action without a
result boundary rejects `throw` just as it rejects `?`.

```text
let selected: String? = None
let discovered: address! = module.address
let failed: address! = Err("module was not found")
```

Use the low-precedence `else` operation to unwrap either type. A value fallback
has the same type as the wrapped value:

```text
let displayName = selected else "Unknown"
let address = discovered else 0 as address
```

The fallback can instead return from the current function or action. This is an
explicit alternative to `?` and retains an ordinary `T!` return value rather
than using a hidden failure channel:

```text
fn requireAddress(value: address!) -> address! {
    let address = value else return Err("required address is unavailable")
    return address
}
```

`else` is looser than `||` and all other expression operators and associates
to the right. Consequently, `optional else result else fallback` means
`optional else (result else fallback)`. A bare `None` or `Err(...)` needs an
annotation, argument type, return type, or other expected-type context to
determine its contained `T`.

## Casts

Conversions use the Rust- and TypeScript-style `as` operator:

```text
let frame = rawFrame as u32
let seconds = frame as f64 / 60.0
let label = frame as String
```

`as` binds more tightly than arithmetic and comparisons and can be chained,
as in `levelTime as u32 as String`. Numeric casts support every integer width,
`address`, `f32`, and `f64`. Narrowing integer casts retain the low bits;
float-to-integer casts saturate at the destination bounds and convert NaN to
zero, matching Rust's cast behavior. Casting an integer or address to `String`
formats its decimal value. Other reference and domain types are not castable.

## Functions

`///` documentation comments can precede functions and methods, global
variables, state fields, records and their fields, and enums and their
variants. The language server includes this documentation in hover information
alongside inferred types and function effects. Consecutive non-empty lines form
one paragraph; an empty `///` line starts a new Markdown paragraph.

```text
/// Returns whether the player reached the final stage.
fn isFinalLevel(level) {
    return stage(level) == 7
}

record Position {
    /// Horizontal position in world units.
    x: f32,
    /// Vertical position in world units.
    y: f32,
}
```

Documentation comments on settings remain their runtime GUI tooltips as
described in the settings section. Ordinary `//` comments are never published
as documentation.

Function parameter and return annotations are optional. Their constraints are
solved together with every function body and call site, including forward
calls. A function with no value-returning `return` is inferred as returning
nothing. Explicit annotations can still be added at API boundaries.

```text
fn isFinalLevel(level) {
    return stage(level) == 7
}

fn stage(level) {
    return (level / 2) + 1
}
```

Functions are independent of a particular action snapshot. Values from
`current` or `old` are passed explicitly, keeping helpers reusable and making
their dependencies visible. Suspending functions will be added with the async
standard-library layer; currently `await` remains specific to `onAttach`.

## Records

Records are immutable, named value shapes represented as WebAssembly GC
structs. Declarations can refer to records declared later in the file. Record
literals are checked for unknown, duplicate, missing, and incorrectly typed
fields; their source order does not matter.

```text
record Digits {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

fn isFresh(value: Digits) -> bool {
    return value.minutes == 0.0 && value.seconds == 0.0
}

let digits = Digits {
    seconds: 0.0,
    hundredths: 0.0,
    minutes: 0.0,
}
```

Records may contain other records and GC strings, pass through functions, and
remain live in an `onAttach` continuation across `await`. Immutability keeps
shared metadata bindings predictable; a new value is constructed when a
snapshot needs to change.

A record whose fields are all fixed-width primitive memory values or other
readable records also implements the compiler-known `MemoryReadable`
capability. Its process-memory layout follows declaration order with natural
alignment: every field starts at the next multiple of its own alignment, and
the final size is rounded up to the largest field alignment. For example,
`{ tag: u8, count: u32, flags: u16 }` has offsets 0, 4, and 8, alignment 4,
and size 12. Reads are currently little-endian. Explicit offsets, packing, and
endianness controls are intentionally deferred until a real target needs them.

## Pattern matching

Enums model values that can have one of several shapes. Each variant may carry
one typed payload; a record can be used when a variant needs multiple values.
`match` destructures payloads and is checked for duplicate, foreign, unknown,
and missing variants.

```text
enum LevelOrScene {
    Level(i32),
    Scene(String),
}

fn isFirst(value: LevelOrScene) -> bool {
    return match value {
        LevelOrScene.Level(level) => level == 0,
        LevelOrScene.Scene(scene) => scene == "Shrine01",
    }
}
```

Integer and boolean values can be matched with literals. Arms may have an `if`
guard, and `_` is the catch-all pattern. Patterns participate in inference, so
the parameter types below are inferred as an integer and `bool` from their uses.

```text
fn characterName(character, dlcDemo) {
    return match character {
        3 if dlcDemo => "Accel",
        3 => "Erika",
        6 if dlcDemo => "Cres",
        _ => "Unknown",
    }
}
```

An unguarded arm counts toward exhaustiveness; a guarded arm may still reject
its pattern. Enum matches must cover every variant, boolean matches must cover
`true` and `false`, and integer matches require an unguarded `_` arm. A wildcard
can also make an enum or boolean match exhaustive.

Enums are immutable GC values and can be nested in records, passed through
functions, and retained across `await`. This directly models the original
Lunistice autosplitter's base-game level number versus DLC-demo scene name.

Enums support `==` and `!=`. Equality is structural: variants must match, and
payload variants then compare their active payloads. Records are structural as
well, comparing fields in declaration order. The capability is derived
recursively, so an enum carrying numbers, strings, other equatable enums, or
equatable records needs no declaration or implementation boilerplate. A
comparison is rejected at compile time with the field or variant path when a
contained type, such as an array, does not yet support equality.

## Methods

A function can belong to a type by qualifying its name. The receiver is
available as the implicit, statically typed `self` parameter.

```text
fn LevelOrScene.isFirst() -> bool {
    return match self {
        LevelOrScene.Level(level) => level == 0,
        LevelOrScene.Scene(scene) => scene == "Shrine01"
    }
}

if game.location.isFirst() {
    print("First level")
}
```

Methods can have additional typed parameters and may be invoked through nested
record paths. They use the same functions and Wasm call instructions as global
helpers; method syntax only provides type-directed organization and an implicit
receiver.

## Arrays

Arrays use mutable WebAssembly GC storage. `[T]` is the general array type: each
value keeps its creation-time length, but that length is not part of the type.
`[T; N]` additionally records an exact compile-time element count. A sized
array can be passed anywhere `[T]` is expected, while a general `[T]` cannot be
narrowed to a particular length without proof. Each element type is
monomorphized into a concrete GC array representation.

Non-empty literals infer their element type. An expected `[T; N]` also checks
the literal's exact element count, while empty literals need an annotation or
another expected type.

```text
let bytes: [u8] = [0x48, 0x00, 0x01]
let header: [u8; 3] = [0x48, 0x00, 0x01]
let inferred = [1, 2, 3] // [i32]
let empty: [u16] = []

bytes.set(1, 0x8b)
let opcode = bytes[1]
let count = bytes.length()
let hasTerminator = bytes.contains(0)
let marker = bytes.indexOf(0x8b) // u32?
```

`length()` returns `u32`, `array[index]` returns `T`, and `set(index, value)`
mutates the selected element. The index is a `u32`, and Wasm performs bounds
checks. `contains(value)` tests every element from the beginning, while
`indexOf(value)` returns the first matching `u32` index or `None`. These search
methods are available when the element type supports `Equatable`; they are
ordinary source-defined library loops rather than dedicated compiler
operations. Arrays can contain records, enums, strings, and other arrays, and
can themselves be stored in records or continuation frames.

Arrays can be traversed directly without manually managing an index:

```text
for byte in header {
    print(byte)
}
```

A non-empty `[T; N]` is `MemoryReadable` when `T` has a fixed readable layout.
`process.read` and `state ... at` then fetch the complete `N * stride(T)` byte
range once and only publish the newly constructed array if that read succeeds.
This makes indexed flags and inventories transactional rather than a collection
of independently failing state fields. Reads are currently limited to 4096
elements and 65536 bytes to bound generated code and host-memory traffic.

Nested arrays can combine both forms, for example `[[u8; 4]; 2]`. Wrapper
postfixes apply to the complete array type, so `[T; N]?` is an optional sized
array and `[T!; N]` is a sized array of fallible values.

## Sets

`Set<T>` is a growable collection of unique values. `T` must implement
`Equatable`; this constraint is declared by the standard library and applies
both to inferred construction and explicit `Set<T>` annotations. Construct a
set with an explicit element type because an empty set has no values from which
to infer it:

```text
let visited = Set.new<String>()

whileAttached {
    if visited.insert(current.roomName) {
        print(`First visit to {current.roomName}`)
    }
}
```

A global set is constructed once for the loaded script instance and retains its
contents across ticks and attachments until it is explicitly changed or the
instance is unloaded. Constructing one inside a polling block instead creates a
fresh empty set each time that expression runs.

`insert(value)` returns whether the value was new. `remove(value)` returns
whether a value was present, `contains(value)` performs a linear equality
search, `length()` returns `u32`, `isEmpty()` reports whether the length is zero,
and `clear()` removes all values. The set object remains stable across mutation;
its backing storage allocates at construction, when insertion grows capacity,
and when clearing releases stored references.

Sets can be traversed directly:

```text
for room in visited {
    print(room)
}
```

Set iteration order is not a language guarantee. Traversal currently observes
the live collection, so mutating that same set from its loop body can change
which values remain to be visited. Avoid doing so; the language will adopt a
stricter diagnostic or snapshot rule before more mutable collection types are
introduced.

## Settings

```text
enum CaptureMode {
    WindowTitle,
    ExecutableName,
    FullPath,
}

settings {
    /// Options used during normal operation.
    "General" {
        /// Can be changed while the splitter is running.
        "Enable Auto Splitting"
            => enableAutoSplitting key "auto-splitting": true,

        /// Chooses how the target application is identified.
        "Capture Source"
            => captureMode: choice {
                "Window Title" => CaptureMode.WindowTitle,
                "Executable Name" => CaptureMode.ExecutableName default,
                "Full Path" => CaptureMode.FullPath,
            },

        /// Files used by the autosplitter.
        "Files" {
            /// Accepts image and JSON layout files.
            "Layout File" => layoutFile: file {
                "Images" => "*.png *.jpg",
                _ => "*.json",
                mime => "image/*",
            },
        },
    },
}
```

Quoted blocks create settings titles. Nesting determines their heading level.
Consecutive `///` documentation comments immediately before a title or setting
become its tooltip. Lines in the same paragraph are joined with spaces; empty
`///` lines preserve paragraph breaks. Ordinary `//` comments remain regular
comments and do not become GUI text.

A boolean setting infers its type from `true` or `false`. A `choice` is backed
by a payloadless enum, so matching it is exhaustive and type checked. A `file`
setting is a `String` and can declare named glob filters, an unnamed fallback
filter, and MIME filters.

The optional `key "host-key"` clause gives a setting the exact string key used
in the host settings map. Without it, the source identifier is also the host
key. Script code always uses the readable declaration name, such as
`settings.enableAutoSplitting`; the external key is metadata for persistence
and data-driven lookup. Keys must be nonempty and unique across the complete
settings block.

Large finite boolean families use a compile-time `for` declaration instead of
hand-written members or mutable runtime registration:

```text
settings {
    "Levels" {
        /// Splits when the player reaches this level.
        for level in 2..=36 {
            `Level {level}` key `{level}`: true,
        },
    },
}
```

The bounds are inclusive non-negative `u32` constants, and one family may
produce at most 4096 settings. Label and key templates may interpolate only
the family binding. The compiler expands the range into ordinary boolean
settings, so generated entries use the same registration, stable-key,
snapshot, default, tooltip, and validation paths as explicit declarations.
They intentionally do not invent statically named members; use
`settings.enabled(key)` or `oldSettings.enabled(key)` for lookup. A `///`
comment before `for` becomes every generated setting's tooltip. Quoted groups
remain visual headings and do not implicitly gate child values.

`settings.enabled(key)` performs allocation-free data-driven lookup over the
declared boolean settings, using those same host-map strings. `oldSettings`
provides the corresponding method for the preceding snapshot. Unknown keys—or
keys belonging to a choice or file setting—return `false`; the API therefore
does not erase heterogeneous setting values into a dynamic type.

Controls are registered during `_start`. At the beginning of every exported
tick, including detached ticks, the compiler loads the current host settings
map. User code sees the freshly decoded values through `settings`; the values
from the preceding tick are available through `oldSettings` for change
detection. Missing host entries use their declared defaults (or an empty string
for a file setting). Misspelled settings and invalid choice variants are
compile-time errors. The quoted text before `=>` is the setting's visible
label; `///` documentation comments are the only way to define its tooltip.

## Actions

`setup` runs once for each loaded WebAssembly instance at the beginning of its
first host update, after global values and the settings UI have been initialized
and the current settings have been loaded. It is intended for
process-independent startup work such as printing a startup message:

```text
setup {
    print("Autosplitter loaded")
}
```

`setup` cannot access `process`, `gba`, `current`, `old`, or suspend. The body
is emitted in `_start`; the LiveSplit runtime deliberately defers that export
until the first controlled update so arbitrary startup code can be interrupted.
A debug watch reload creates a new module instance, so its `setup` block runs
again on that instance's first update.
This is deliberately distinct from `onAttach`, which runs once for every
selected process and may suspend while performing discovery.

`onDetached` runs once when the runtime enters the detached state: once initially,
then once immediately each time an attached process closes. Attachment retries
do not rerun it. This makes detached-state policy explicit in the script:

```text
onDetached {
    setTickRate(1.0)
}
```

`setTickRate(hz)` uses updates per second. The LiveSplit runner reads the
resulting interval after the current `update` returns, so a call affects the
wait before the following update rather than the invocation already in
progress. The selected rate persists until another call; process closure does
not restore either the host's 120 Hz initial rate or a script-defined baseline
automatically. Set the baseline in `onDetached`, which already runs once before
the first attachment, and let `onAttach` override it only while attached.

```text
start {
    return current.level == 1;
}

split {
    return current.level != old.level;
}

reset {
    return current.level == 0;
}

isLoading {
    return current.loading;
}

gameTime {
    return Duration.fromFrames(current.frames, 60);
}
```

Every action may fall through or use a bare `return`; the runtime supplies its
domain default:

| Action | Fallthrough result | Runtime meaning |
| --- | --- | --- |
| `start`, `split`, `reset` | `false` | Do not perform the timer action |
| `isLoading` | `None` | Leave the current game-time pause state unchanged |
| `gameTime` | `None` | Do not set a new game time |

`None` is an explicit return value only for `isLoading` and `gameTime`, where it
represents a real third state. It is deliberately rejected in `start`, `split`,
and `reset`; those blocks are simply boolean and default to `false`.
`gameTime` otherwise returns a `Duration`. Source-defined constructors include
`Duration.zero()`, exact whole-unit and nanosecond constructors,
`Duration.fromFrames(i64, i64)`, and `Duration.fromParts(i64, i32)`.
`Duration.fromMilliseconds`, `fromSeconds`, `fromMinutes`, `fromHours`, and
`fromDays` accept either floating-point type. A day is always exactly 86,400
seconds; these are elapsed durations, not calendar values. `setup`,
`whileAttached`, and `onDetached` return nothing.
`whileAttached` runs before timer actions on every attached tick.

## Discovered state and watchers

An `at` field retains the compact static pointer-path syntax. A state field may
instead use a typed expression, allowing `onAttach` to discover Unity roots and
field offsets once and the state DSL to read them every tick:

```text
let manager = 0
let pointsOffset = 0

state "game.exe" {
    points: i32 = process.read(manager.offset(pointsOffset));
}
```

Every state expression produces a `T!`: a plain value is lifted automatically,
while a process read already returns a result. The first snapshot requires all
required fields to succeed together and initializes `old` and `current` to the
same value. On later polls, an error keeps that field's accepted value while
successful fields advance. Put values that must advance atomically into one
record- or array-valued state field; the whole aggregate is then one acceptance
unit.

When one field is genuinely optional, make its value type optional and convert
that read's error with the source-defined `Result.toOption()`:

```text
state "game.exe" {
    level: u32 = process.read(levelAddress);
    bonus: u32? = process.read<u32>(bonusAddress).toOption();
}
```

A failed `level` read retains its last accepted value after initialization. A
failed `bonus` read is instead accepted as `current.bonus == None`; a later
successful read becomes a present `u32` without an explicit `Some` constructor.
The error text is deliberately discarded, so `toOption()` should not be used
merely to silence an unexpected failure.

Discovery globals currently require an initializer because they exist for the
whole script lifetime. State polling is gated until `onAttach` completes, so a
typed sentinel such as `let levelAddress: address = 0x0` is safe when every
completing attach path assigns it. Unsupported builds should remain suspended
with `await process.closed()` rather than complete with an uninitialized
address. The maintained ABZÛ example demonstrates this contract.

Pointer width belongs to the resolved memory path, not to the state-field
declaration. Use `PointerSize.Bit32` for a 32-bit target even when SplitScript
and the host run on 64-bit systems:

```text
state "game.exe" {
    loading: bool = process.read<bool>(
        executableBase.memoryPath([0x00480af0], 0, PointerSize.Bit32).resolve()?
    )?;
}
```

This is why there is no separate `at32` spelling: static `at` paths use the
attached process's native pointer width, while discovered or cross-width paths
state their `PointerSize` exactly where traversal occurs. The maintained
Borderlands PE32 layout has a host-executed fixture for this form.

## Structured async initialization

`onAttach` is inherently suspending; it does not need an `async` modifier.

```text
onAttach {
    let expectedVersion: u32 = 1
    let gameAssembly = await process.module("GameAssembly.dll")
    let marker = await gameAssembly.scan(sig"48 8B ?? B? 00")
    if expectedVersion == 1 && marker != 0 {
        print("Initialization finished")
    }
}
```

Suspending operations are polled once per runtime tick until they complete.
State reads and timer actions remain gated while initialization is pending. If
the process closes at any suspension point, the generated process-lifetime
scope cancels the initializer, resets it, and starts fresh with the next
attached process. This is the language-level counterpart to ASR's
`until_closes`, without requiring scripts to manually write the outer
attach/cancellation loops.

`retry expression` is separate from `await`: it accepts any ordinary `T!`
expression, evaluates it once per update, and yields the contained `T` when it
succeeds. An error keeps the suspension pending. Because `retry` is control
flow rather than a standard-library function, it works equally well through a
user helper whose Result type and value type are inferred:

```text
fn readMarker() {
    return process.read<i32>(0x3000)
}

onAttach {
    let marker = retry readMarker()
    print(`marker {marker}`)
}
```

`await nextTick()` is the basic scheduling primitive. It always suspends once
and resumes on the following attached-process update. It is useful when a game
needs one frame to publish metadata after another awaited discovery:

```text
let gameAssembly = await process.module("GameAssembly.dll")
await nextTick()
let marker = await gameAssembly.scan(sig"48 8B ?? ??")
```

Like every `onAttach` suspension, a pending next-tick continuation is discarded
if the process closes.

Source-defined helpers use `async T` as a real future type. The annotation is
required only when the return type is written explicitly; otherwise both the
future and its completion type are inferred:

```text
fn afterTick(value) {
    await nextTick()
    return value
}

onAttach {
    let pending = afterTick(42)
    print("future created")
    let value = await pending
    print(value)
}
```

Calling `afterTick` evaluates and captures its arguments once and allocates a
typed WebAssembly GC continuation frame; it does not begin polling the body.
Intrinsically asynchronous operations behave the same way, so
`let pending = process.module("game.dll")` creates a future that may be passed
or stored before `await pending`. An `async T` value can be held in a local,
record, enum, option, result, or array and passed as a parameter. Awaiting it
dispatches to its concrete typed poll function. Once complete, the frame
retains `T`, so another await returns the same value without rerunning the
operation. Merely creating a future is synchronous. Futures are owned by the
attached-process lifetime and therefore cannot be stored in globals.

`await` is an ordinary prefix expression rather than a declaration form. It
can appear inside member access, arguments, arithmetic, interpolation,
conditional and match arms, guards, fallbacks, and loop conditions. Lowering
spills earlier operands once and leaves branch-local suspensions inside the
selected branch, preserving source evaluation order across ticks.

`onAttach` supports the same variables, assignments, expressions, calls, and
conditional control flow as other action blocks, including suspensions nested
in `if`, `else if`, and `else` branches. Locals that live across a suspension
are stored in a compiler-generated WebAssembly GC continuation frame;
values whose next use is preceded by another assignment stay ordinary Wasm
locals. Each suspension has a dedicated poll state, so conditions and side effects
before a pending operation are not replayed on the next tick. A successful poll
continues through the selected branch and rejoins statements after it.

`process.module(name)` produces a `Module` with `address: address` and
`size: u64`. An awaited result is stored in the GC continuation frame only when
a later suspension segment uses it. A signature literal is distinct from
`String`: its nibbles are checked and converted to needle/mask bytes at compile
time.

```text
let code = await process.module("GameAssembly.dll")
let matchAddress = await code.scan(sig"48 8B ?? ?? B? 00")
```

`?` may replace either nibble, so `??` matches any byte and `B?` matches any
byte whose high nibble is `B`. Module scans read overlapping 4 KiB chunks and
therefore find patterns crossing page boundaries. A missing pattern suspends
and retries on the next tick; process closure cancels the whole initializer.

Windows executable versions have their own checked `FileVersion` value and
`v"major.minor.build.private"` literal. The literal requires exactly four
decimal components, each within the `u16` range. It is therefore safe to use in
typed build selection without parsing host-formatted version strings.

```text
let executable = await process.mainModule()
let version = executable.fileVersion() else v"0.0.0.0"
if version == v"1.5.0.0" {
    return StateLayout.V1500
}
```

The quotes deliberately bound the complete structured literal, as they do for
`sig"..."`, so an omitted or malformed component receives one focused parser
diagnostic rather than being interpreted as unrelated numeric/member syntax.

## Typed process memory

`process.read(address)` infers its exact memory representation from an
annotation, state-field usage, argument, fallback, or other surrounding type
constraint. A synchronous read returns `T!`, so failure must be handled with
`else`, propagated, or passed directly to a state field. Retrying the same call
re-evaluates it on later ticks and yields `T` directly. `retry` responds to the
Result state only; it does not invent a separate sentinel value for failure.

```text
let mode: i32 = retry process.read(object + 0x10)
let elapsed: f32 = process.read(object + 0x18) else 0.0
let next: address = retry process.read(object + 0x20)
```

Named `MemoryReadable` records use the same call. The runtime performs one host
read for the complete naturally aligned record, then constructs its immutable
WebAssembly GC value locally. Nested readable records are decoded recursively,
so a state snapshot cannot observe a torn mixture of individually read fields.

```text
record LevelTimeParts {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

state "game.exe" {
    levelTimeParts: LevelTimeParts = process.read(timer.offset(levelTimeOffset));
}
```

Native strings are read as bounded decoding operations rather than synthetic
types such as `string32`. `process.readUtf8(address, maxBytes)` reads at most
4096 bytes in one host call, stops at the first NUL byte (or at the bound), and
returns `String!`. An inaccessible range, a zero or excessive bound, or invalid
UTF-8 is an ordinary error. `process.readUtf16Le(address, maxUtf16Units)` reads
at most 2048 little-endian UTF-16 code units, also stops at NUL or the bound,
and replaces malformed surrogate sequences with the Unicode replacement
character. This is intentionally different from
`process.readManagedString`, which understands the in-memory layout of a Unity
managed string.

Pointer-backed state fields have compact sugar for the same operation. The
decoder applies after the complete module-relative pointer path has been
resolved, and it infers the field as `String`:

```text
state "game.exe" {
    mapName at "game.dll", 0x1234, 0x20 as utf8(64);
    chapterName at "game.dll", 0x2345, 0x18 as utf16le(64);
}
```

The bound describes a read operation, not the resulting value's type. All
decoded values are ordinary `String` values; there are no `string64`-style
pseudo-types. `utf8` is strict because invalid bytes cannot form a language
string. `utf16le` deliberately uses replacement decoding, matching the native
ASL UTF-16 behavior while naming the byte order explicitly.

When no context determines the representation, add an annotation or use an
explicit type argument such as `process.read<u8>(address)`. Any
`MemoryReadable` type can be written there, including named records and
fixed-length arrays. An ambiguous generic read produces a diagnostic showing
both fixes. `address` is nominal
rather than an alias for `u64`, preventing a module size or counter from being
passed where a target pointer is required.

`Module` values retain the identity used to discover them. In addition to
`address` and `size`, `module.path()` returns the runtime's portable,
host-provided filesystem path as `String!`. The operation is fallible because
the host may not expose a path; it does not provide general filesystem access.

```text
let executable = await process.mainModule()
let executablePath = executable.path() else "Unavailable"
```

For Windows PE modules, `module.fileVersion()` parses the bounded numeric
`VS_FIXEDFILEINFO` resource directly from process memory. It returns an
equatable `FileVersion` record with `major`, `minor`, `build`, and
`privatePart` fields. This keeps version selection typed instead of relying on
legacy version strings with inconsistent separators.

```text
let version = executable.fileVersion() else return
if version.major == 1 && version.minor == 2 {
    print("recognized executable version")
}
```

Generic calls put type arguments directly after the callable name:

```text
let header = process.read<Header>(address) else return
let bytes = process.read<[u8; 16]>(address) else return
```

There is no Rust-style `::` turbofish. The opening `<` must touch the callable
name. This keeps `value < limit` an ordinary comparison while making
`read<u32>(address)` unambiguous; the formatter removes any remaining spaces
around the generic call delimiters.

`process.follow(base, offsets)` accepts `[i64]` and reads a non-null 64-bit
pointer at every successive `current + offset` location. This uses the same
wrapping signed-displacement arithmetic as static `at` paths and
`MemoryPath`. `process.readRelative32(location)`
decodes the common x86-64 RIP-relative form as `location + 4 + i32(location)`.
Both return `address!`. Use `else` or `?` for a one-shot attempt, or `retry` in
`onAttach` to poll until they succeed.

```text
let object = retry process.follow(module.address, [0x10, -0x28])
let target = retry process.readRelative32(instruction + 0x3)
let found = await process.scan(target, 0x200, sig"48 8B ?? ??")
```

Use `address.offset(displacement)` when the displacement is signed and
`address.add(delta)` when an unsigned full-width `u64` delta is already
available. Both preserve the nominal `address` type and wrap modulo 2^64.
`address.memoryPath(dereferences, finalOffset, pointerSize)` stores signed
`i64` dereference and final offsets and resolves them with `offset`.

`print` is a regular typed builtin available in every action block and writes
through the runtime debug-message API. Its argument is any `String` expression,
not only a literal. Strings use content equality with `==` and `!=`, and
`value.byteLength()` returns a string's UTF-8 byte length. A message after the final await in
`onAttach` therefore prints once per successful process attachment, while a
message in `whileAttached` prints every attached tick.

The immutable `String` API uses explicit UTF-8 byte semantics where indexing
is involved:

| Operation | Behavior |
| --- | --- |
| `byteLength()` | UTF-8 byte length |
| `isEmpty()` | Whether the required string contains zero UTF-8 bytes |
| `contains(text)` | Case-sensitive substring test |
| `indexOf(text)` | First matching UTF-8 byte offset as `u32?` |
| `startsWith(text)` / `endsWith(text)` | Case-sensitive prefix/suffix tests |
| `equalsIgnoreAsciiCase(text)` | Equality folding only ASCII letters |
| `toAsciiLowerCase()` | Lowercase ASCII letters; preserve every other UTF-8 byte |
| `toAsciiUpperCase()` | Uppercase ASCII letters; preserve every other UTF-8 byte |
| `trimAsciiWhitespace()` | Remove ASCII boundary whitespace; preserve interior and non-ASCII bytes |
| `split(delimiter)` | Fallible exact split preserving leading, repeated, and trailing empty segments |
| `parse<T>()` | Strict fallible ASCII decimal parsing into an inferred numeric type |
| `byteAt(byteIndex)` | Fallible raw UTF-8 byte lookup; continuation bytes remain observable |
| `charAt(byteIndex)` | Fallible `char` lookup at a UTF-8 byte boundary |
| `slice(start, end)` | Fallible half-open UTF-8 byte range; offsets must be code-point boundaries |
| `replaceAll(search, replacement)` | Fallible exact non-overlapping replacement |
| `String.concat(values)` | Concatenate an array of strings |

Case conversion reuses an already-normalized immutable string and allocates
only when at least one ASCII letter changes. The operations intentionally have
ASCII-specific names: full Unicode case conversion can change byte length and
requires a separate, explicitly specified API.

All string offsets are UTF-8 byte offsets. `byteAt` is appropriate for binary
inspection and accepts every in-range byte. `charAt` decodes the complete
Unicode scalar beginning at an offset and returns it as a `char`; it fails when
the offset points into a multibyte sequence. It deliberately does not use
JavaScript's or C#'s UTF-16 code-unit indexing:

```splitscript
let separator = "map_01".charAt(3) else return
let sharpS = "Straße".charAt(4) else return
return separator == '_' && sharpS == 'ß'
```

`text.parse<T>() -> T!` parses the complete string as a numeric value. The
target type is normally inferred from the assignment, return value, or
fallback, and can be written explicitly when needed:

```splitscript
let percentage: f64 = current.percentageText.parse() else 0.0
let lives = current.livesText.parse<u8>()?
```

Parsing accepts an optional ASCII sign and decimal digits. Floating-point
targets additionally accept a decimal point and `e`/`E` exponent, plus
case-insensitive `NaN`, `inf`, and `Infinity`. Floating-point decimals are
correctly rounded directly to the target width with ties to even: finite
overflow produces signed infinity and underflow produces signed zero. Integer
overflow remains an error. Whitespace, digit separators, and trailing text are
rejected, so malformed game-memory input uses ordinary `Result` handling
rather than silently producing a partial value.

The Wasm implementation uses allocation-free Simple Decimal Conversion with a
reused 768-digit scratch buffer. It does not parse through an intermediate
`f64` when the target is `f32`, avoiding double rounding, and it does not call a
locale-sensitive host routine.

`process.readManagedString(address, maxLength)` reads a bounded IL2CPP managed
string, decodes UTF-16 (including surrogate pairs), and returns a GC UTF-8
`String!`. Memory-access failures are ordinary errors that can be handled with
`else` or `?`, or polled with `retry` during attachment. The unit limit bounds
decoding, and malformed surrogate sequences become the Unicode replacement
character.

JavaScript-inspired template strings use backticks and `{expression}`, without
JavaScript's `$` marker. Existing strings are inserted directly. Every other
interpolated value is converted by the same rules as `value as String`; integer
widths, `address`, and standard-library types with source-defined formatting
such as `FileVersion` are supported, while values without the `Display`
capability produce compile-time errors. Template strings may contain multiple interpolations,
nested expressions, and newlines. Literal braces are written as `\{` and `\}`.
`$` is ordinary template text, so `${value}` intentionally emits a literal
dollar sign followed by the interpolation of `value`; the compiler cannot
safely treat that spelling as a JavaScript migration typo.

```text
let level = `{stage}-{act}`
let levelTime = `{minutes as u32}:{twoDigits(seconds as u32)}`
let executableLabel = `version {version}`
```

Transformations such as `toAsciiLowerCase`, `toAsciiUpperCase`,
`trimAsciiWhitespace`, `replaceAll`, and `split` do not mutate their receiver.
They return new values and are marked must-use, so a discarded result receives
a focused warning explaining the immutable behavior.
`print(value)` and `setVariable(key, value)` accept any `Display` value
and apply these same conversions at the runtime boundary, so numeric values and
addresses do not need an explicit `as String` cast. A standard-library type can
tag an ordinary source-defined method with `@display`; that one implementation
then powers all four conversion entry points without a type-specific backend
branch. `FileVersion` uses this mechanism to render
`major.minor.build.private`. `timer.state()` and
`setTickRate(f64)` expose the corresponding ASR facilities without
linear-memory pointers in source code.
`timer.state()` returns `TimerState`, a
compiler-provided enum with `NotRunning`, `Running`, `Paused`, `Ended`, and
`Unknown`. Match it like any other enum; the raw host integer is not visible to
source code.
`timer.pauseGameTime()` and `timer.resumeGameTime()` provide explicit timer
mutation for lifecycle cleanup. Prefer `isLoading` for ordinary load removal.

The generated loop follows this order:

1. Attach to the configured process, or return and retry next tick.
2. Detect a closed process, detach, and return.
3. Rotate and refresh state.
4. Run `whileAttached`.
5. If the timer has not started, evaluate `start`.
6. If it is running or paused, apply `isLoading`, then `gameTime`, then `reset`;
   evaluate `split` only when reset did not trigger.

## Why GC and linear memory both appear

Long-lived language values use WebAssembly GC. Today, the Auto Splitting Runtime
ABI represents host strings and process read buffers as `(pointer, length)` pairs
in exported linear memory. SplitScript therefore keeps a small memory page for
the host boundary and scratch reads. This is an ABI adapter, not the language's
object model.

Future arrays, strings, user records, closures, and generic collections can all
be represented as GC values without changing the host ABI.
