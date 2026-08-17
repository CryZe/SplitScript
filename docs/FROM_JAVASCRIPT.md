# SplitScript for JavaScript authors

SplitScript uses JavaScript-like expressions, braces, and backtick strings,
but it is statically typed and deliberately has fewer overlapping concepts.
The compiler infers most types from both definitions and uses, while process
memory remains explicit enough to preserve exact layouts.

## One declaration style, static types

Use `let` for mutable locals and globals and `fn` for functions. There is no
`const`/`let` split, no `var`, and no implicit coercion. Typed `==` and `!=`
replace `===` and `!==`; logical and bitwise operators remain distinct.

```splitscript
# state "game.exe" {}
fn scoreText(player: String, score: u32) -> String {
    return `{player}: {score}`
}

# onAttach {
let score = 1200u32
print(scoreText("Runner", score))
# }
```

Interpolation is `{expression}`, not `${expression}`. A literal dollar sign
before an interpolation stays literal, so `${score}` produces a dollar sign
followed by the formatted score.

## Numbers describe memory

Numbers are not all one floating-point type. SplitScript has signed and
unsigned 8-, 16-, 32-, and 64-bit integers plus `f32` and `f64`. Unsuffixed
integer and floating-point literals normally default to `i32` and `f64`, but
memory reads require an unambiguous layout.

```splitscript
state "game.exe" {
    flags: u8 at 0x1000;
    score: i64 at 0x1008
}

split {
    return old.flags & 1u8 == 0u8 && current.flags & 1u8 != 0u8
}
```

Use `as` for deliberate numeric casts. Narrow integer arithmetic wraps to the
declared width, so addresses, masks, and counters keep their intended shape.

## None, options, and errors

`None` replaces `null` for absence. `T?` means an optional `T`; `T!` means a
`T` or an error. Plain values assign directly to their present/success cases.
Match with `Some(value)`/`None` or `Ok(value)`/`Err(message)` when both states
matter.

```splitscript
# state "game.exe" {}
fn optionalScore(value: u32?) -> String {
    return match value {
        Some(score) => `{score}`,
        None => "unknown",
    }
}

# onAttach {
print(optionalScore(None))
# }
```

Recover a `T!` with `else fallback`, propagate it from another `T!` function
with postfix `?`, or use `match`. Errors are explicit values rather than thrown
JavaScript exceptions.

## Arrays, records, and control flow

`[T]` is a growable array and `[T; N]` is an exact fixed-length array. Records
replace object literals when a stable named shape matters. `if` and `match`
are expressions, and `for value in values` plus `while condition` provide
loops without callback allocation.

```splitscript
# state "game.exe" {}
fn containsLevel(levels: [u32], target: u32) -> bool {
    for level in levels {
        if level == target {
            return true
        }
    }
    return false
}

# onAttach {
print(containsLevel([1u32, 3u32, 7u32], 3u32))
# }
```

## Attachment-aware async work

One `state` declaration owns process attachment. `onAttach` may use `await`
without an `async` keyword; waiting operations yield back to the runtime and
are cancelled when that process closes. Poll memory in `state`, then use
`old` and `current` in the timer actions.

```splitscript
state "game.exe" {}

onAttach {
    let image = await process.module("GameAssembly.dll")
    print(`GameAssembly at {image.address}`)
}
```

Use `settings` declarations rather than dynamic JavaScript objects. They
produce typed members, stable host keys, and documentation tooltips.
