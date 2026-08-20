# SplitScript for C# authors

SplitScript keeps C#'s static predictability, familiar braces, fixed-width
numbers, and [`as`] casts, but it is a small autosplitter language rather than a
general .NET environment. Its lifecycle, process access, error values, and
settings are language features, so translating only the spelling is not
enough.

## Declarations and types

Use [`let`] for every local and global variable and [`fn`] for functions. Bindings
are mutable unless the surrounding value is read-only. Types are normally
inferred in both directions; annotate process-memory boundaries when the
target layout does not already determine the type.

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
Use arrays as [`[T]`], fixed memory arrays as [`[T; N]`], records for named product
types, and enums with exhaustive [`match`].

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

These are typed values, not exceptions. A later [`throw`]/`catch` model is not
part of the language today.

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
