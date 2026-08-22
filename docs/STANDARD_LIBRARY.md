# SplitScript standard library

The standard library is being developed from real autosplitter requirements,
with the Lunistice port serving as the first full-scale compatibility target.
Game-specific names and offsets belong in the autosplitter; process access,
signature scanning, engine metadata, watchers, strings, collections, timing,
and cancellation belong here.

## Native process support

An ordinary `state "game.exe" { ... }` selects the catalog's native state
provider. The executable names still come from the source declaration, but
attachment, the implicit value, direct state reads, documentation, and editor
behavior use the same provider model as emulator-backed states.

The provider exposes a read-only `process: Process` value. `Process` is a
nominal scalar handle, not a namespace: it can be passed to functions, returned,
or stored in inferred locals and globals. Its methods include `module`, `read`,
`follow`, `scan`, `readRelative32`, `readUtf8`, `readUtf16Le`, and
`readManagedString`. Method
lowering consumes the written receiver, so this is valid ordinary typed code:

```splitscript
state "game.exe" {}

fn readScore(attached: Process, address: address) -> u32! {
    return attached.read(address)
}

whileAttached {
    let attached = process
    let score = readScore(attached, 0x1234) else 0
}
```

The native provider uses an identity attachment: the host's attached-process
handle already is the `Process` representation. Transformed providers such as
GBA instead name an ordinary standard-library function that asynchronously
constructs a different nominal value. This distinction is catalog metadata,
not a provider-name switch in the parser, checker, or runtime lifecycle.

## GBA emulator support

`state GBA { ... }` selects the standard-library GBA state provider. The
provider attaches to a supported emulator, discovers its EWRAM and IWRAM
mapping, and exposes a read-only `gba: GBAEmulator` value. Its generic
`gba.read(address) -> T!` method accepts original GBA hardware addresses and
infers the memory representation from its expected result type. Reads outside
`0x02000000..0x02040000` and `0x03000000..0x03008000`, including reads that
cross either region boundary, return an error.

State fields with a fixed address should use the concise `at` syntax. The GBA
provider maps it through the same address translation and typed-read operation
as `gba.read`; the explicit method remains available for addresses computed by
the script.

```splitscript
state GBA {
    inventory: [u8; 6] at 0x02002B32
    scene: u8 at 0x03000BF4
}
```

`[T; N]` carries its exact element count in the type. When `T` is
`MemoryReadable`, the provider reads the complete fixed array in one host call
and constructs the GC array only after that call succeeds. It otherwise uses
ordinary array indexing, so `current.inventory[0]` reads the first byte
from the already captured snapshot.

Discovery covers VisualBoyAdvance/VBA-M, mGBA's contiguous mapping, NO$GBA,
standalone Mednafen, the supported RetroArch cores, and mGBA-based BizHawk.
Pointer-backed layouts refresh the current RAM base during reads so starting
or reloading a ROM does not leave the script with a stale mapping.

The emulator policy, signatures, and mapping selection live together in
`GBAEmulator.discover`, an ordinary source-defined standard-library function.
Memory-range lookup and module signature selection are bounded suspension
primitives: each poll inspects at most one range or one signature window before
returning control to the host. Only hardware-address translation and the final
host memory read remain compiler-provided representation primitives.

The provider owns the emulator executable list and attachment lifecycle.
Autosplitters do not call an attachment function or retain an optional handle.
Only `gba` is available as the process-access root in a GBA script; ordinary
native-process scripts use `process` instead. This keeps the two memory models
distinct and lets completion and diagnostics present only the applicable API.

## PlayStation 2 emulator support

`state PS2 { ... }` selects the PlayStation 2 provider and introduces a
read-only `ps2: PS2Emulator` value. It supports PCSX2 and 64-bit RetroArch with
the `pcsx2_libretro.dll` core. PCSX2 discovery prefers the exported `EEmem`
symbol and retains signature fallbacks for older 32-bit and 64-bit builds;
RetroArch resolves `retro_get_memory_data` from the loaded core.

Both `ps2.read<T>(address)` and state-field `at` declarations use original PS2
addresses. Reads must lie entirely within `0x00100000..=0x01ffffff`.
Provider-relative pointer paths dereference 32-bit guest pointers through the
same translation backend before applying each signed offset:

```splitscript
state PS2 {
    health: u16 at 0x00123456;
    inventory: u32 at 0x00110000, 0x20, 0x8;
}
```

Emulator translation is selected by the direct-read intrinsic's central
provider contract. Ordinary method calls and generated state polling consume
that same contract, so adding an emulator no longer requires a provider-name or
GBA-specific branch in either code-generation path. Only the target address
translation remains a Rust runtime helper; attachment and discovery policy stay
in `stdlib/standard.split` as ordinary checked SplitScript source.

## Compiler and tooling model

The library surface is described by a backend-independent catalog. Each entry
has a stable ID, canonical name, callable kind, generic type scheme,
constraints, documentation, parameter documentation, examples, related items,
deprecation information, and an implementation key.
Type checking resolves source calls to these IDs. WebAssembly generation and
async lowering only handle the resolved implementation key; neither resolves
source names. Operational metadata has exactly one authority per implementation:
the closed intrinsic registry for intrinsic leaves and compiler analysis for
source-defined bodies.

Every callable has exactly one implementation: either a trusted intrinsic
declaration ending in `;` or an ordinary SplitScript body. Intrinsics remain
necessary for host ABI access, physical representations, runtime helpers, and
special lowering primitives. Higher-level operations are composed in library
source and checked and lowered through the same resolver, inference, typed HIR,
effect analysis, reachability, Wasm IR, and encoder as user functions. Merely
being a library body grants no privilege; it can reach host behavior only by
calling a validated intrinsic declaration.

The implemented synchronous body tier uses ordinary inferred function
templates and demand-driven monomorphization. A catalog call supplies its exact
concrete receiver, parameter, and result signature, so constructed layouts and
source-owned types use the same `FunctionInstance` machinery as user-defined
generic functions. Reachability emits only called concrete instances.
`Numeric.clamp` composes the primitive `min` and `max` operations, while
`[T].isEmpty()` composes `length()` across arbitrary array element types.
`[T].contains(value)` and `[T].indexOf(value)` are source-defined search loops;
their declarations add `Equatable` only to the inherited `T` used by those
methods, leaving operations such as `length()` available for every element
type. None of these helpers has dedicated intrinsic dispatch or backend
lowering.

Bodies may perform any operation reachable through their permitted intrinsic
leaves. All `Duration` constructors are source-defined: their arithmetic and
physical GC-record construction live in `standard.split`, so the type has no
intrinsic operations. This includes zero, exact integer and fractional unit
constructors, frames, and parts. Capability-directed implementation cases let
one public `T: Numeric` constructor retain separate `Integer` and `Float`
source bodies; selection occurs only after generic specialization and is not a
general user-visible overload system. A catalog-owned body may construct its
owning GC struct, including runtime-private fields, without making that
constructor syntax available to user code or unrelated library functions.
`address.offset` widens its argument and delegates to primitive full-width
address addition. `timer.isRunning` and `Module.readRelative32` demonstrate
effectful composition over timer and process intrinsics entirely in library
source.

Each independently owned catalog graph runs a standalone compilation of its
source bodies once. Ordinary typed call-graph analysis derives a canonical
effect set, availability, attachment requirement, suspension, and cancellation
metadata without consulting a user program. The immutable result is cached on
the graph and supplies checking and editor queries. Source entries contain no
fake `pure` fields that a consumer could accidentally treat as authoritative.
Normal user compilations still type-check the injected bodies and verify their
inferred metadata against the standalone result. Actual suspending library
bodies remain the next body tier.

`CompilerContext` owns the selected catalog through a cloneable
`StandardLibrary` handle backed by an immutable `Arc` graph. Compiler passes
borrow that graph; parsed, checked, tooling, and backend products clone the
owner only when they must retain it. The bundled graph is cached, while tests
also construct and inject an independently owned validated graph through the
same context path. This makes catalog identity and lifetime explicit without
requiring each pass to reconstruct global state. Its declaration producer is
the build-only privileged SplitScript loader: `stdlib/standard.split` is parsed
and source-validated once during the Cargo build, then emitted as typed catalog
data. Callable body blocks are retained only for source-defined
implementations. The compiler injects one hidden inferred function template per
catalog source body after the user file and parses it with the ordinary program
parser. Resolved catalog call signatures drive concrete instances through
ordinary demand specialization. Public syntax, HIR, semantic
iterators, symbols, and editor features retain the user-only view, while the
backend sees the complete unit. Generated function names use a reserved prefix
that user programs cannot declare.

Effects distinguish ordinary process reads from operations that require an
attached process, suspend or retry, and cancel when that process closes.
`RequiresAttachedProcess` and `CancelsOnProcessClose` are catalog facts shared
with type checking and async lowering. `StandardLibrary::operation_semantics`
normalizes them into `SuspensionKind`, `CancellationKind`, availability, and a
process requirement; `render_operation_semantics` provides their common human
presentation. Catalog validation rejects incompatible declarations such as a
non-awaitable cancellable item. Documentation and editor tooling consume these
queries directly rather than reading implementation-specific storage.

Language-only constructs live in a sibling `LanguageCatalog`, rather than as
fake standard-library functions. It gives keywords such as `await`, `retry`,
and `as`; lifecycle actions; source-spellable built-in and constructed types;
wrapper/literal syntax; snapshot roots; compiler-provided fields and
the settings DSL stable IDs, compact source forms, documentation, and checked
examples. Standard-library namespaces, nominal types (including
`TimerState`), fields, variants, and callables remain exclusively in the
standard-library catalog. Both catalogs
share the generic documentation and example model in `catalog.rs`, so generated
reference pages and LSP hover/completion can consume one metadata shape while
preserving the semantic difference between syntax and callable APIs.

Resolved user functions and methods inherit the attached-process requirement
through a fixed-point call-graph analysis. This includes recursive call graphs;
no source annotation is required. The inferred result is available through the
checked compiler product and prevents a process-dependent helper from hiding an
invalid `onDetach` operation.

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
dedicated constructed-type identities; WebAssembly GC layout construction reads those
semantic queries rather than the AST annotations. `TypeKind::Array` retains
its `ArrayTypeId` layout identity, element `TypeId`, and optional exact length,
so code generation never needs to reconstruct this information from syntax.
`TypeKind::Set` similarly retains the source generic-application identity,
element type, and compiler-selected backing-array identity.
`TypeStore` has no
parallel legacy type representation: Wasm storage/value selection lowers
`TypeId` / `TypeKind` directly into backend-local physical categories.

The semantic `TypeStore` is created before inference. Core primitives,
standard-library types, and source record/enum types enter inference as their
canonical `TypeId` and retain that exact identity through checked publication,
rather than passing through parallel enums and post-inference conversion
tables. Only inference variables and temporarily unresolved `[T]`, `T?`, and
`T!` constructor terms remain solver-local. Namespace, nominal-type, field,
variant, and callable IDs are generated from the same declaration rows as their
names, ownership, documentation, and representation metadata.

Catalog-declared capabilities are executable contracts rather than labels.
Capabilities can build on other capabilities in the privileged source. For
example, `Numeric<T: Equatable>` makes equality part of the numeric
contract, while `Integer<T: Numeric + Display>` makes every integer numeric,
equatable through that numeric relationship, and displayable. The
compiler follows this hierarchy transitively for type checking and method
completion, while inferred constraints and rendered signatures retain only the
strongest non-redundant capabilities. Concrete integer types consequently
declare `Integer` once instead of separately repeating those memberships.
Nominal standard-library types can connect a parameterless `String`-returning
source method to `Display` with `@display`. The generated type declaration owns
that implementation identity, catalog validation checks its receiver and
signature, and reachability treats conversions as calls to the ordinary hidden
library body. `FileVersion.toString()` is the first implementation: casts,
interpolation, `print`, and `setVariable` all dispatch to it, while codegen has
no `FileVersion` formatting case.
`MemoryReadable` GC records derive their naturally aligned field layout from
catalog field declarations through the same semantic-`TypeId` layout engine as
source records. `Equatable` catalog records similarly receive generated
structural equality helpers whose dependencies close over nested declared
fields. Catalog validation rejects readable or equatable declarations whose
representation and fields cannot satisfy those contracts. A test-only ordinary
record, with no intrinsic implementation, passes through name resolution,
checking, memory layout, equality, hover, completion, and Wasm GC generation;
this guards the promise that future ordinary types do not need compiler-wide
type matches.

Source annotations, cast targets, and integer suffixes use the separate,
inference-free `ast::TypeRef`. It contains no catalog or resolved nominal IDs;
lowering publishes those as `types::ResolvedTypeRef`. Parser-owned constructed
type expressions and checker-owned resolved layouts are distinct as well.
Expressions have no checker-owned
type slot: pending types are recorded directly by `ExprId` and resolved when
the semantic model is finalized. The dedicated inference context owns type
variables, union/find unification, requirement composition, integer-literal
bounds, numeric defaulting, and inferred array layouts. The checker translates
solver failures into source diagnostics; syntax and editor APIs never expose
these temporary types.

Free functions and type-directed methods are exposed as
declarative `CallCandidate` values. `process.read(address)` leaves its named
generic parameter open for bidirectional inference, while an explicit generic
call such as `process.read<u16>(address)` seeds `T = u16`. Method candidates
carry their receiver type scheme and
capability constraints, and retain the selected receiver value through Wasm
lowering. The checker uses
one catalog-call path to instantiate that scheme and submit receiver,
argument, expected-result, and capability constraints to the same inference
context used by ordinary expressions. Candidate discovery itself commits no
new constraints, and ambiguous applicable candidates produce a diagnostic
before a candidate is committed. The catalog validates duplicate callable
shapes so accidental indistinguishable overloads are caught by tests.
Binary and unary operator syntax uses this same path. Privileged declarations
bind methods to operator identities with `@operator`, so `value + other`,
`value == other`, `-value`, and `!value` resolve to the same items exposed as
ordinary documented methods. Short-circuit `&&` and `||` remain language-level
control flow rather than eager method calls.

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
validation programs. A line prefixed with `# ` inside a `splitscript` example
fence participates in parsing, type checking, semantic highlighting, and
definition resolution, but is omitted from rendered documentation. This is the
standard way to supply lifecycle scaffolding and local declarations without
distracting from the focused snippet. The generated reference maps semantic
spans from the complete fixture back onto the visible lines, so links never
depend on identifier text alone.

Catalog entries can also declare a use obligation with
`@mustUse("reason")`. A bare expression statement that discards such a return
value produces a warning while compilation and Wasm generation still succeed.
The marker may be attached to a callable for an operation-specific explanation,
or to a type form such as `T?` and `T!` so the obligation
follows values returned by user functions as well. `String.toAsciiLowerCase`,
`String.replaceAll`, and `String.split` use callable-specific reasons because
strings are immutable: they return transformed values and never change their
receivers. The ASCII conversion's runtime helper reuses the receiver when no
byte changes, while still conservatively advertising its possible allocation
effect. The checked compiler product retains these warnings, and the database,
LSP, compiler service, CLI, and watch workflow all
publish the same structured diagnostics.

## Value layer

`String` is a first-class, statically typed value backed by a WebAssembly GC
array of immutable UTF-8 bytes. String literals can be inferred into locals,
stored in continuation frames across `await`, compared by content with `==`
and `!=`, and passed to APIs such as `print`.

```text
let message = "Assembly-CSharp is ready"

if (message.byteLength() != 0) {
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
- Unity Mono assembly/image/class/inherited-field/static-table discovery for
  modern 64-bit Windows V2 and V3 metadata layouts.
- Watchers with current/old pairs and change predicates.
- Timer state and controls, custom runtime variables, tick-rate control, and
  saturating duration conversion.

These APIs are implemented as ordinary typed standard-library facilities where
possible. Compiler lowering is reserved for representation primitives, host
imports, static signature data, and suspension points that cannot yet be
expressed as normal source code.

## Unity IL2CPP

The implemented IL2CPP surface supports 64-bit Unity base, 2019, 2020, and
2022 layouts. `Unity.il2cpp` is implemented in standard-library SplitScript:
its supported-version policy, signatures, scan windows, and instruction
displacements are visible together in one source body. It composes the same
bounded module and range scans available to scripts, so discovery yields
between scan windows rather than hiding an unbounded compiler helper. Only the
target-memory metadata offsets and object-layout facts needed by low-level
operations remain compiler-owned.

```text
let unity = await Unity.il2cpp(2020)
let image = await unity.image("Assembly-CSharp")
let gameManager = await image.class("GameManager")
let instanceOffset = await gameManager.field("Instance")
let staticTable = await gameManager.staticTable()
let instance = retry process.read<address>(staticTable.offset(instanceOffset))
```

`UnityModule` exposes `assemblies`, `typeInfoTable`, `version`, and
`pointerSize`. `UnityImage` and `UnityClass` expose their metadata `address`.
Each discovery operation retries across ticks. Field lookup walks parent
classes and recognizes C# auto-property backing fields, so `field("Instance")`
also matches `<Instance>k__BackingField`.

## Unity Mono

The initial Mono surface supports the 64-bit Windows
`mono-2.0-bdwgc.dll` family with explicit `MonoVersion.V2` and
`MonoVersion.V3` target layouts. V3 corresponds to Unity 2021.2 and newer;
the enum names describe internal metadata layouts rather than Mono marketing
versions. Older V1 layouts, 32-bit layouts, ELF, and Mach-O are not silently
guessed.

```splitscript
let mono = await Unity.mono(MonoVersion.V2)
let image = await mono.image("Assembly-CSharp")
let autosplitter = await image.class("AutoSplitterData")
let gameTime = await autosplitter.staticField("inGameTime")
```

Discovery resolves `mono_assembly_foreach` through the source-defined
`Module.peExport`, finds Mono's assembly-list global from its RIP-relative
load, and yields while assemblies or metadata are not ready. Class names may
be simple or namespace-qualified. Field lookup walks parent classes and also
recognizes C# auto-property backing fields. The implementation and its target
offsets live in `stdlib/standard.split`; Mono does not add compiler intrinsics
or type-checker cases.

Static managed singletons can remain dynamically resolved instead of caching
the object address observed during attachment. `staticFieldPath` describes the
static slot as a `MemoryPath`, and immutable `dereference` appends the instance
field access:

```splitscript
let playerStats = await image.class("PlayerStats")
let script = await playerStats.staticFieldPath("script")
let districtOffset = await playerStats.field("currDistrict")
let district = script.dereference(districtOffset as i64)
```

Resolving `district` during each state poll rereads the static slot, so a
replacement singleton is observed without rerunning `onAttach`. The maintained
[`Himno` port](HIMNO_PORT.md) verifies this behavior against three distinct
managed objects.

`fieldAny([names...])` returns a `UnityField { offset, index }`, making
version-dependent layouts explicit without manual races. `staticInstance`
combines alternative field lookup, static-table discovery, and a retrying
non-null pointer read:

```text
let layout = await gameManager.fieldAny(["currentLevel", "_currentScene"])
let instance = await gameManager.staticInstance(["Instance", "_instance"])
```

Loaded Windows modules expose `peExport(name) -> address!` as ordinary
source-defined standard-library composition. It validates the mapped PE export
directory, name/ordinal/function tables, and rejects forwarded exports rather
than pretending their forwarder string is executable code. This is useful for
runtime metadata discovery such as Mono's `mono_assembly_foreach`; it is not a
cross-platform symbol API, and malformed or absent exports remain ordinary
`T!` errors.

An `address` supports generic `offset<T: Integer>` for displacements and
`add(u64)` for unsigned full-width deltas. Signed arguments retain their sign;
smaller integer widths are extended before addition. Both wrap modulo the 64-bit address space while
keeping target pointers nominally distinct from numeric sizes. Static `at`
paths keep their absolute root unsigned, but module-relative and
post-dereference offsets are signed. `process.follow` accepts `[i64]`, and
`MemoryPath` stores signed dereference and final offsets, so negative native
pointer paths do not require lossy casts.

## Watchers, strings, and timing

Expression-backed state fields form persistent watchers. Initialization waits
for every required field to succeed in one poll and seeds `old == current`.
Later, each successful `T!` advances that field and each error retains its last
accepted value; actions see the resulting `current` and prior `old` objects.
`process.read(address)` infers its
`MemoryReadable` type from the field, annotation, or later usage. This includes
fixed-width primitives and both source- and catalog-declared records containing
only readable fields. Record fields use declaration order and natural
alignment; one host read obtains the complete layout before the compiler
recursively constructs its GC value.
`T!.discardError()` is ordinary source-defined library composition over
wrapper matching. It turns success into a present `T?` and error into `None`,
which lets one intentionally unavailable state field commit as absent while
all remaining required field failures continue to reject the transaction. It
discards the error string and therefore is not a general replacement for
handling or propagating failures.
Immediate process operations return `T!`: fixed-layout reads, pointer following,
relative-address decoding, and string decoding. They can be handled
synchronously with `else` or `?`; `retry expression` polls any of them across
attached updates and yields `T`. Native NUL-terminated UTF-8 uses
`process.readUtf8(address, maxBytes)`. The bound is part of the operation, so
all successful values have the ordinary `String` type rather than generated
`string32`-style types. Pointer state fields can write
`name at address as utf8(maxBytes)`; this is sugar for the same strict,
bounded, fallible decode after the pointer path is resolved. Native
NUL-terminated UTF-16LE similarly uses
`process.readUtf16Le(address, maxUtf16Units)` or
`name at address as utf16le(maxUtf16Units)`. Its bound is measured in code
units, malformed surrogate sequences become the Unicode replacement character,
and successful values are still ordinary `String` values. Managed IL2CPP
strings are decoded with
`process.readManagedString(pointer, maxUtf16Units)` and propagate failed
memory access as an error. The unit limit bounds decoding, while malformed
surrogate sequences become the Unicode replacement character.

Numeric conversions and integer formatting use `value as Type`. The `Display`
capability is the single contract for `as String`, JavaScript-style template
strings such as `` `{stage}-{act}` ``, `print`, and `setVariable`.
Standard-library nominal types may fulfill that contract with an `@display`
source method; primitives retain their compact compiler implementation.
`String.concat` remains available as the underlying collection helper.
`timer.state`, `timer.pauseGameTime`, `timer.resumeGameTime`, and `setTickRate`
wrap their ASR host calls. `isLoading` remains the normal declarative load-
removal API; explicit pause/resume is for lifecycle transitions such as process
exit cleanup. The language-level `tickRate` policy defaults to 120 Hz attached
and 1 Hz detached and reapplies those rates at lifecycle transitions.
`setTickRate(hz)` is the dynamic escape hatch: it is measured in updates per
second, affects the next host wait after the current update, and persists until
another call or attachment-state transition. It accepts every integer and
floating-point type and converts the value only at the host's `f64` ABI boundary.
`timer.state()` returns the exhaustive `TimerState` enum with
`NotRunning`, `Running`, `Paused`, `Ended`, and `Unknown`; raw host integers are
normalized only at the ABI boundary. `Duration.fromSeconds` converts Unity's
floating-point level clock into timer game time.
