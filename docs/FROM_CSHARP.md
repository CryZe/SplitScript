# SplitScript for C# authors

SplitScript keeps C#'s static predictability, familiar braces, fixed-width
numbers, and [`as`] casts, but it is a small autosplitter language rather than a
general .NET environment. Its lifecycle, process access, error values, and
settings are language features, so translating only the spelling is not
enough.

Keep these common spelling changes in mind:

- C# `int` becomes SplitScript [`i32`].
- C# `string` becomes SplitScript [`String`].
- C# `TimeSpan` becomes SplitScript [`Duration`].
- C# `record` declarations become SplitScript [`struct`] declarations.

## Declarations and types

Use [`let`] for every local and global variable and [`fn`] for functions. Bindings
are mutable unless the surrounding value is read-only. Types are normally
inferred in both directions; annotate process-memory boundaries when the
target layout does not already determine the type.

Use an irrefutable [`binding pattern`] when a known struct or fixed shape
should introduce several names at once. Binding patterns work in initialized
[`let`] declarations, function and closure parameters, and runtime [`for`]
bindings. Unlike a C# deconstruction assignment, the pattern declares new
names and must match every value of the incoming type; use [`is`] or [`match`]
when the test can fail. An annotation after the pattern applies to the complete
incoming value.

The struct name may be omitted when the surrounding value or annotation is
already a concrete struct: `let { x, y } = position` and
`fn length({ x, y }: Position)` remain nominal rather than matching any value
that happens to have those fields.

C# names map to explicit widths: `byte` becomes [`u8`], `int` becomes [`i32`],
`long` becomes [`i64`], and their unsigned counterparts retain the same width.
Use [`f32`] or [`f64`], [`String`], and [`Duration`] instead of `float`, `double`,
`string`, and `TimeSpan`.

```splitscript
state "game.exe" {
    health: u32 at 0x1000
}

fn healthText(health) {
    return `Health: {health}`
}

whileAttached {
    print(healthText(current.health))
}
```

Backtick strings interpolate with `{expression}` without C#'s leading `$`.
Use arrays as [`[T]`], fixed memory arrays as [`[T; N]`], [`struct`] declarations
for named product types, and enums with exhaustive [`match`]. SplitScript uses
the familiar [`struct`] spelling for immutable value shapes too; C# `record` is
not an alias, but the compiler offers a direct migration fix.

There is no class override or interface declaration for textual display.
Structs and enums receive a readable multiline [`Display`] representation by
default. Define `fn Type.toString() -> String` only to override it; the compiler
can infer the result. Use lower-camel `toString`, not C#'s `ToString`.

```splitscript
# state "game.exe" {}
struct Position {
    x: i32,
    y: i32,
}
fn Position.toString() { return `({self.x}, {self.y})` }
# onAttach {
print(Position { x: 3, y: 5 })
# }
```

## Value-producing blocks

A block used in an expression can declare local values and then yield its final
expression. This avoids a temporary helper or immediately invoked delegate when
an [`if`] branch, [`match`] arm, fallback, or argument needs multiple steps:

```splitscript
# state "game.exe" {}
fn levelLabel(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}
# setup { print(levelLabel(true)) }
```

The final expression yields from the nested block; it does not return from the
function. Function, method, and lifecycle bodies continue to use explicit
[`return`]. A block without a final expression yields [`None`]. A trailing
semicolon on the final value is accepted for familiarity, but the compiler
warns because it does not discard that value, and the formatter removes it.

## Unconditional and value-producing loops

C# `while (true)` maps to [`loop`] when repetition is intentionally
unconditional. Unlike C#, a [`loop`] can itself produce a value through
`break value`; all break values infer one result type. A loop without a break
has type [`Never`].

```splitscript
# state "game.exe" {}
fn choose(flag: bool) -> i32 {
    return loop {
        if flag { break 7 }
        break -1
    }
}
# setup { print(choose(true)) }
```

Keep [`while`] for a real condition. Only [`loop`] accepts a value-carrying
[`break`]; bare break and [`continue`] work in all runtime loops. Function
results still require the explicit [`return`] shown above.

## Optional and fallible values

[`T?`] is an optional value and [`T!`] is a value or an error. [`None`] is both the
unit value and the absent option case. Plain `T` values promote into either
wrapper, so construction does not need [`Some`] or [`Ok`]; those names exist for
pattern matching. Recover a result with [`else`], propagate it from a [`T!`]
function with postfix [`?`], or inspect it with [`match`].

```splitscript
# state "game.exe" {}
fn parseLives(text: String) -> u32! {
    return text.parse()
}

fn livesOrZero(text: String) -> u32 {
    return parseLives(text) else 0u32
}

# onAttach {
print(livesOrZero("3"))
# }
```

These are typed values, not CLR exceptions. [`throw`] constructs and transfers
the same error value; there is no general `try`/`catch` construct.

## Retrying synchronous fallible work

Use [`retry`] when attachment should re-run a synchronous read transaction on
later ticks until every required value is available. A value block is an
ordinary expression, so several reads can share one local failure boundary:

```splitscript
# state "game.exe" {}
onAttach {
    let health = retry {
        let player = process.read<address>(0x1000)?
        process.read<i32>(player)?
    }
    print(health)
}
```

This is not a C# exception retry loop. A [`T!`] error, [`?`], or [`throw`] ends
the current attempt, and the complete operand begins again on the next attached
update. [`return`] still returns from the enclosing function; [`break`] and
[`continue`] keep their lexical loop targets. The attempt must be synchronous
and bounded, so [`await`] and nested [`retry`] are rejected inside it. Await
asynchronous discovery before entering the retry block.

## Autosplitter lifecycle

Declare one [`state`] block. It owns attachment and polls its fields into the
transactional [`old`] and [`current`] snapshots. Use [`onAttach`] for discovery that
may suspend, [`onStateReady`] for work after the first complete snapshot,
[`whileAttached`] for per-tick bookkeeping, and [`onDetach`] for process-lifetime
cleanup. [`start`], [`split`], and [`reset`] return booleans; [`isLoading`] and
[`gameTime`] may return [`None`] when there is no new observation.

```splitscript
state "game.exe" {
    completed: bool at 0x2000
}

split {
    return !old.completed && current.completed
}
```

There is no explicit [`async`] modifier on lifecycle blocks. Awaiting is allowed
where the lifecycle permits suspension, and process closure cancels pending
attachment-owned work.

## Process reads and settings

Memory access is fallible. State pointer paths propagate read failure at the
field boundary; direct `process.read<T>(address)` calls return [`T!`]. Prefer a
state field for values polled every tick and direct reads for attachment-time
discovery.

Settings are declared statically and become typed members. Documentation
comments become tooltips, and the [`settings`] and [`oldSettings`] views reflect
the current and previous tick.

```splitscript
settings {
    /// Enables the completion split.
    "Split on completion" => completionSplit: true
}

state "game.exe" {
    completed: bool at 0x2000
}

split {
    return settings.completionSplit && !old.completed && current.completed
}
```

## Next step

Open **Getting started** from the documentation index and build its first
autosplitter workflow. When a particular C# API does not translate directly,
use **Search Documentation** with the original spelling; migration results link
to the canonical SplitScript symbol or focused porting recipe.
