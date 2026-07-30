# SplitScript standard library

The standard library is being developed from real autosplitter requirements,
with the Lunistice port serving as the first full-scale compatibility target.
Game-specific names and offsets belong in the autosplitter; process access,
signature scanning, engine metadata, watchers, strings, collections, timing,
and cancellation belong here.

## Compiler and tooling model

The library surface is described by a backend-independent catalog. Each entry
has a stable ID, canonical name, callable kind, generic type scheme,
constraints, effects, availability, documentation, parameter documentation,
examples, related items, deprecation information, and an implementation key.
Type checking resolves source calls to these IDs. WebAssembly generation and
async lowering only handle the resolved implementation key; neither resolves
source names.

Effects distinguish ordinary process reads from operations that require an
attached process, suspend or retry, and cancel when that process closes.
`RequiresAttachedProcess` and `CancelsOnProcessClose` are catalog facts shared
with type checking and async lowering. `StdlibItem::operation_semantics`
normalizes them into `SuspensionKind`, `CancellationKind`, availability, and a
process requirement; `render_operation_semantics` provides their common human
presentation. Catalog validation rejects incompatible declarations such as a
non-awaitable cancellable item. Documentation and editor tooling should consume
these queries directly when their machine-readable interfaces are added.

Language-only constructs live in a sibling `LanguageCatalog`, rather than as
fake standard-library functions. It gives keywords such as `await`, `retry`,
and `as`; lifecycle actions; source-spellable built-in and constructed types;
wrapper/literal syntax; snapshot roots; compiler-provided fields and
`TimerState`; and the settings DSL stable IDs, compact source forms,
documentation, and checked examples. Both catalogs
share the generic documentation and example model in `catalog.rs`, so generated
reference pages and LSP hover/completion can consume one metadata shape while
preserving the semantic difference between syntax and callable APIs.

Resolved user functions and methods inherit the attached-process requirement
through a fixed-point call-graph analysis. This includes recursive call graphs;
no source annotation is required. The inferred result is available through the
checked compiler product and prevents a process-dependent helper from hiding an
invalid `onDetached` operation.

Catalog signatures no longer depend on parser AST types: built-in catalog
types use `BuiltinType`, and a checked call exposes its inferred generic
arguments as interned `TypeId` values. `TypeStore` lets documentation and editor
clients inspect `TypeKind` directly, including constructed arrays, without
depending on inference-only AST variants. Inferred declaration types are also
semantic facts: `SemanticModel::value_type` covers globals, parameters, locals,
await bindings, state fields, settings, and match payload bindings, while
`SemanticModel::function_result` covers function results. Optional source
annotations remain optional in checked syntax instead of being overwritten by
inference. Record fields, enum payloads, and array elements likewise publish
their resolved `TypeId` layouts through `RecordFieldId`, `EnumVariantId`, and
the dedicated `ArrayTypeId`; WebAssembly GC layout construction reads those
semantic queries rather than the AST annotations. `TypeKind::Array` retains
both its `ArrayTypeId` layout identity and element `TypeId`, so code generation
never needs to reconstruct this information from syntax. `TypeStore` has no
parallel legacy type representation: Wasm storage/value selection lowers
`TypeId` / `TypeKind` directly into backend-local physical categories.

Source annotations, cast targets, and integer suffixes use the separate,
inference-free `ast::TypeRef`; parser-owned array references and checker-owned
inferred array layouts are distinct as well. Expressions have no checker-owned
type slot: pending types are recorded directly by `ExprId` and resolved when
the semantic model is finalized. The dedicated inference context owns type
variables, union/find unification, requirement composition, integer-literal
bounds, numeric defaulting, and inferred array layouts. The checker translates
solver failures into source diagnostics; syntax and editor APIs never expose
these temporary types.

Free functions, typed paths, and type-directed methods are exposed as
declarative `CallCandidate` values. `process.read(address)` leaves its named
generic parameter open for bidirectional inference, while an explicit typed
path such as `process.read.u16(address)` seeds `T = u16`. Method candidates
carry their receiver type scheme and capability constraints. The checker uses
one catalog-call path to instantiate that scheme and submit receiver,
argument, expected-result, and capability constraints to the same inference
context used by ordinary expressions. Candidate discovery itself commits no
new constraints, and ambiguous applicable candidates produce a diagnostic
before a candidate is committed. The catalog validates duplicate callable
shapes so accidental indistinguishable overloads are caught by tests.

Read-only queries cover item enumeration, exact canonical lookup, path and
method candidate lookup, signature rendering, and the documentation stored on
each item. `StandardLibraryDocumentation` turns those facts into one canonical
generic or call-site-substituted reference entry. Completion, hover, and
signature help already consume that entry; the future browsable and
machine-readable documentation renderers will use the same payload.

Every parsed expression also has a stable per-program `ExprId`. Checked
expression types and standard-library call resolutions are queried by this ID,
not by source spans. Syntax expressions are never mutated with inferred types;
WebAssembly lowering reads the semantic `TypeId`. This keeps syntax suitable
for future incremental editor analysis while inference internals continue to
be migrated.

User functions and methods likewise have stable per-program `FunctionId`
values. `ResolvedCall` distinguishes user functions, user methods, and catalog
items, so editor navigation and WebAssembly lowering consume the same checked
call target. Method-call facts also retain the resolved receiver root and its
semantic type, allowing hover/navigation and lowering to agree about the value
on which a method operates. Backend callable dispatch is entirely ID-based;
member resolution is being migrated independently as described below.

Globals, parameters, ordinary locals, awaited bindings, state fields, and
settings now carry `ValueId` values as well. Path expressions publish a
`ResolvedValue` root for go-to-definition, including the temporal distinction
between `current`/`old` and `settings`/`oldSettings`. Backend reads use ID-keyed
Wasm locals, continuation-frame fields, state slots, setting globals, and user
globals. Assignment statements have stable `AssignmentId` values and publish
their `ValueId` targets too, so backend writes use the same ID-keyed storage.
Match payload bindings also have `ValueId` values. Method receivers are lowered
from their semantic `ResolvedValue` roots, so compiler-created receiver paths
and their former local/global/setting name maps are gone. Records, enums,
record fields, enum variants, and compiler-provided fields have distinct typed
IDs. Path expressions publish ordered `ResolvedMember` chains, while record
literals and enum constructors publish the selected field/variant IDs. The
backend therefore does not reinterpret member-path or constructor spelling.
Match arms have `PatternId` values, and choice options have
`SettingChoiceOptionId` values; their resolved enum variants, including choice
defaults, are semantic queries consumed directly by match and settings
lowering. Variant text remains in settings lowering only as the external value
understood by the host settings map.

The same catalog is the API for future generated documentation, LSP completion,
hover, and signature help. Tools can parse and check a program and inspect its
semantic call resolutions without constructing the WebAssembly backend. All
currently implemented functions and type-directed methods—including numeric,
array, process, duration, address, and Unity APIs—are catalog-backed. Compiled
catalog examples keep their user-facing snippets separate from complete
validation programs. Each visible snippet demonstrates one symbol without
lifecycle scaffolding or compiler-smoke setup, while the test suite still
compiles its hidden validation program.

## Value layer

`String` is a first-class, statically typed value backed by a WebAssembly GC
array of immutable UTF-8 bytes. String literals can be inferred into locals,
stored in continuation frames across `await`, compared by content with `==`
and `!=`, and passed to APIs such as `print`.

```text
let message = "Assembly-CSharp is ready"

if (String.length(message) != 0) {
    print(message)
}
```

The runtime ABI still accepts strings through linear memory. A single standard
library boundary adapter grows the exported memory when necessary, copies the
GC string to scratch space, and invokes the host. Language code never receives
or manages that scratch pointer.

Reusable functions, type-directed methods, named immutable GC records, and
payload enums with exhaustive matching are now part of the value layer,
including nesting and persistence across suspension. The next additions are
UTF-16 decoding, numeric formatting, and string construction. Generic mutable
GC arrays are now available as the typed buffer and collection foundation.
Together these facilities directly cover Lunistice's `Digits`, `GameInfo`,
`GameManager`, `LevelOrScene`, character labels, and managed scene names.

## ASR layer

The reusable ASR surface is grouped by responsibility:

- Live user settings with nested headings and tooltips, booleans, enum-backed
  choices, file selectors with glob/MIME filters, and typed current/previous
  tick snapshots.
- Process attachment to an ordered list of executable names, GC `Module`
  values containing base and size, and managed process-lifetime cancellation.
- Compile-time parsed `sig"..."` literals and overlapping page-based module
  scanning, including full-byte and nibble wildcards.
- Typed synchronous and retrying reads for fixed-width primitives and naturally
  laid-out readable records, non-null address discovery, 64-bit pointer
  traversal, RIP-relative decoding, and arbitrary-range scans.
- `nextTick()` as a host-independent one-update suspension integrated with the
  implicit async action state machine and process-lifetime cancellation. The
  language-level `retry expression` form polls arbitrary `T!` expressions;
  race combinators remain planned on the same foundation.
- Unity IL2CPP module/image/class/field/static-instance discovery, versioned
  layouts, generated typed bindings, and managed-string decoding.
- Watchers with current/old pairs and change predicates.
- Timer state and controls, custom runtime variables, tick-rate control, and
  saturating duration conversion.

These APIs are implemented as ordinary typed standard-library facilities where
possible. Compiler lowering is reserved for representation primitives, host
imports, static signature data, and suspension points that cannot yet be
expressed as normal source code.

## Unity IL2CPP

The implemented IL2CPP surface supports 64-bit Unity base, 2019, 2020, and
2022 layouts:

```text
let unity = await Unity.il2cpp(2020)
let image = await unity.image("Assembly-CSharp")
let gameManager = await image.class("GameManager")
let instanceOffset = await gameManager.field("Instance")
let staticTable = await gameManager.staticTable()
let instance = retry process.read.address(staticTable.offset(instanceOffset))
```

`UnityModule` exposes `assemblies`, `typeInfoTable`, `version`, and
`pointerSize`. `UnityImage` and `UnityClass` expose their metadata `address`.
Each discovery operation retries across ticks. Field lookup walks parent
classes and recognizes C# auto-property backing fields, so `field("Instance")`
also matches `<Instance>k__BackingField`.

`fieldAny([names...])` returns a `UnityField { offset, index }`, making
version-dependent layouts explicit without manual races. `staticInstance`
combines alternative field lookup, static-table discovery, and a retrying
non-null pointer read:

```text
let layout = await gameManager.fieldAny(["currentLevel", "_currentScene"])
let instance = await gameManager.staticInstance(["Instance", "_instance"])
```

An `address` supports `offset(u32)` and `add(u64)`, keeping target pointers
nominally distinct from numeric sizes while still making field offsets easy to
apply.

## Watchers, strings, and timing

Expression-backed state fields form transactional watchers. A fresh snapshot
is committed only when every field's `T!` succeeds; actions see the committed
`current` and prior `old` objects. `process.read(address)` infers its
`MemoryReadable` type from the field, annotation, or later usage. This includes
fixed-width primitives and named records containing only readable fields.
Record fields use declaration order and natural alignment; one host read obtains
the complete layout before the compiler recursively constructs its GC value.
Immediate process operations return `T!`: fixed-layout reads, pointer following,
relative-address decoding, and managed-string decoding. They can be handled
synchronously with `else` or `?`; `retry expression` polls any of them across
attached updates and yields `T`. Managed IL2CPP strings are decoded with
`process.read.managedString(pointer, maxUtf16Units)` and propagate failed
memory access as an error. The unit limit bounds decoding, while malformed
surrogate sequences become the Unicode replacement character.

Numeric conversions and integer formatting use `value as Type`. JavaScript-style
template strings such as `` `{stage}-{act}` `` apply the same `as String`
conversion automatically to non-String interpolations. `String.concat` remains
available as the underlying collection helper.
`setVariable`, `timer.state`, and `setTickRate` wrap their ASR host calls.
`timer.state()` returns the exhaustive `TimerState` enum with
`NotRunning`, `Running`, `Paused`, `Ended`, and `Unknown`; raw host integers are
normalized only at the ABI boundary. `Duration.fromSeconds` converts Unity's
floating-point level clock into LiveSplit game time.
