# SplitScript for Rust authors

SplitScript shares Rust's expression-oriented control flow, fixed-width
numbers, `as` casts, exhaustive `match`, postfix `?`, and strong inference.
It removes ownership syntax and exposes autosplitter lifecycle and process
attachment directly, targeting WebAssembly GC rather than native code.

## Bindings and inferred capabilities

Use `let` without `mut`; ordinary bindings are mutable, while values such as
`old` snapshots and iteration elements are read-only by their role. Function
parameters and returns may be inferred from all uses. Generic behavior is
reported as capability bounds such as `Numeric`, `Display`, or
`MemoryReadable`, which play a trait-like role but are currently declared by
the standard library rather than user programs.

```splitscript
# state "game.exe" {}
fn greater(left, right) {
    return left > right
}

# onAttach {
print(greater(7u32, 3u32))
# }
```

Arrays use `[T]` and `[T; N]`, records are GC product types, and enums support
payload variants. There are no lifetimes, moves, borrows, or explicit memory
management in source.

## None is the unit type

`None` is both the language's zero-sized unit value and the absent side of
`T?`. Functions that only perform effects infer `None`; `void` and Rust's `()`
are not source syntax. Plain `T` values promote into `T?` and `T!`, while
`Some` and `Ok` are used only to distinguish patterns.

```splitscript
# state "game.exe" {}
fn parseCount(text: String) -> u32! {
    return text.parse()
}

fn showCount(text: String) -> None {
    let count = parseCount(text) else 0u32
    print(count)
}

# onAttach {
showCount("4")
# }
```

Postfix `?` propagates to the nearest state-field boundary or `T!` function.
Use `else fallback` for local recovery and `match` for explicit
`Ok(value)`/`Err(message)` handling.

## Async values and cancellation

An asynchronous value has type `async T`. Named functions write that as
`-> async T`; lifecycle blocks infer suspension from `await`. Unlike an
executor-agnostic Rust future, attachment-owned work is automatically
cancelled when its process closes.

```splitscript
state "game.exe" {}

fn findImage() -> async Module {
    return await process.module("GameAssembly.dll")
}

onAttach {
    let image = await findImage()
    print(image.address)
}
```

Use `retry expression` for an operation that should be attempted again on
later ticks until it succeeds; use ordinary `await` when one asynchronous
operation already owns its retry policy.

## Autosplitter domains

`state` owns attachment and memory polling. Its accepted values become the
transactional `old` and `current` snapshots. `onAttach`, `onStateReady`,
`whileAttached`, and `onDetach` describe process-lifetime phases. `start`,
`split`, `reset`, `isLoading`, and `gameTime` communicate timer decisions.

```splitscript
state "game.exe" {
    level: u32 at 0x1000
}

split {
    return old.level != current.level
}
```

Process reads are fallible and require a concrete `MemoryReadable` layout.
Prefer state pointer paths for values polled every tick and direct reads or
signature scans for attachment-time discovery. Settings are a typed
declaration DSL, not a map assembled at runtime.
