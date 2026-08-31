# SplitScript language and documentation usability review

Reviewed 2026-08-29. This review evaluates the experience of learning and using
SplitScript, not the quality of the compiler implementation. It covers the
repository landing page, the language and standard-library documentation, the
migration material, examples, compiler-owned reference pages, and the VS Code
extension's Marketplace README.

## Executive summary

SplitScript's documentation contains unusually strong technical material. The
ASL guide records real semantic differences instead of suggesting unsafe
search-and-replace ports, many examples are compiler-checked, and the generated
reference is searchable from both the CLI and VS Code. The language itself also
makes several difficult autosplitter concerns explicit: fixed-width memory
types, transactional snapshots, process-lifetime cancellation, fallible reads,
and typed settings.

The main usability problem is not lack of information. It is that the
information is presented at the wrong level, in the wrong order, or more than
once:

- There is no short, obvious route from “I installed SplitScript” to a small
  working autosplitter.
- `docs/LANGUAGE.md` is presented as user documentation but regularly explains
  HIR, Wasm lowering, GC representation, continuation frames, and DWARF.
- Hand-written overviews and compiler-owned reference data have drifted into
  direct contradictions.
- Migration data is rendered as very large tables and lists that mix ASL, C#,
  JavaScript, Rust, supported features, rewrite advice, and unavailable host
  features.
- The VS Code extension README is also its installed-extension/Marketplace
  description, but nearly half of it is about building, packaging, and testing
  the extension itself.
- Documentation source files rely on compiler-only links and hidden example
  lines, even though the main README sends GitHub readers directly to those
  source files.

The highest-value change is to establish a short learner path and separate the
three documentation modes: tutorials teach a task, guides explain concepts,
and generated reference pages describe exact syntax and APIs. Compiler and
extension implementation details should remain available, but in explicitly
developer-facing documents.

## Priority recommendations

### P0 — Fix the first-use path

Create a compiler-checked “Getting started” guide that starts with one minimal,
clean script and takes the reader through:

1. installing or obtaining the compiler/extension;
2. creating a `.split` file;
3. compiling in debug mode;
4. understanding where the `.wasm` file is written;
5. running or loading that output in the supported host, or clearly stating if
   that workflow is not available yet;
6. building a release module; and
7. opening documentation and interpreting the first diagnostic.

If SplitScript is currently source-only or cannot yet be used in an ordinary
LiveSplit installation, say that directly. “After installation” in
`README.md:124` is not sufficient when the document never defines an
installation method.

The first example should be closer to this:

```splitscript
settings {
    /// Enables splits when the level number changes.
    "Split on level change" => splitOnLevelChange: true,
}

state "game.exe" {
    level: u32 at 0x123456;
}

split {
    return settings.splitOnLevelChange && current.level != old.level
}
```

The guide should then explain the four things the reader sees: attachment,
polled state, old/current snapshots, and a timer decision. Advanced module
scans, Unity layouts, signatures, and async discovery should come later.

### P0 — Rewrite the VS Code extension README for extension users

`editors/vscode/README.md` is the text a user sees on the extension page. Its
current `Development` section begins at line 60 and occupies 47 of the file's
106 lines. It explains `npm install`, VSIX packaging, headless web-host tests,
browser development, virtual test workspaces, and Extension Development Host
launch configurations. Those are useful to contributors and irrelevant to
someone who installed the extension to write SplitScript.

Move that material to `editors/vscode/DEVELOPMENT.md` or the repository's
contributing guide. The Marketplace README should contain:

- one outcome-focused description;
- a three- to five-step getting-started flow;
- a small copyable script;
- the user-visible commands and what each command produces;
- where debug and release `.wasm` files go;
- how to open and search the bundled documentation;
- requirements, current limitations, and troubleshooting; and
- one link to contributor instructions.

The bundled implementation is a user benefit, but it only needs one sentence:
“The extension includes the compiler and language tooling; no separate
`splitc` installation is required.” Details about workers, Wasm adapters,
`ExtensionContext.extensionUri`, and `workspace.fs` belong in development
documentation.

The manifest description in `editors/vscode/package.json:4` is accurate but
undersells the extension. A clearer version would be: “Write, check, format,
and compile statically typed LiveSplit autosplitters.”

### P0 — Turn the language document into user documentation

`docs/LANGUAGE.md` is 2,226 lines with only 21 second-level sections. Its three
largest sections are 354, 289, and 272 lines. It begins with global lifetime
inference, punctuation policy, polling rates, and a large state/pointer-path
section before teaching ordinary variables or presenting a small complete
program. That is reference/design ordering, not learning ordering.

Split it into a short language overview plus focused guides. A useful reading
order would be:

1. a complete minimal autosplitter and its runtime mental model;
2. lifecycle and snapshots;
3. variables, types, functions, and control flow;
4. state fields and memory reads;
5. settings and timer decisions;
6. `T?`, `T!`, `else`, `?`, `retry`, and `await`;
7. structs, enums, arrays, sets, ranges, closures, and iterators;
8. layouts, Unity schemas, emulators, and other advanced topics; and
9. exact syntax and API reference.

At minimum, add a table of contents and divide the longest sections into
task-named subsections. “Read a value every tick,” “Discover an address once,”
and “Support two game versions” are more usable entry points than one 272-line
“State and pointer paths” section.

### P0 — Redesign migration navigation instead of rendering the catalog as one table

`docs/MIGRATION_CAPABILITIES.md` is the clearest example of the table problem:

- 86 data rows are placed in one four-column Markdown table;
- the longest source line is 591 characters;
- 41 rows say “Use a typed pattern,” 39 say “Supported directly,” and 6 say
  “Planned”;
- internal catalog IDs such as `asl.state.attachment` lead every row;
- long prose, canonical targets, and recipe links all share one cell; and
- despite the title “ASL migration capability index,” the first half includes
  many C#, JavaScript, and Rust concepts.

The bundled reference adds another alphabetical 86-item migration index. It
also injects a “Quick migration map” and a second “Common ASL concepts” table
ahead of the already 1,972-line ASL guide. The underlying catalog is valuable,
but three dense renderings do not create three useful navigation paths.

Keep the catalog as structured data and change its presentation:

- First choose a source: ASL, C#, JavaScript, or Rust.
- Within ASL, group by task: attachment and state, process and memory,
  lifecycle and timer, settings, collections and text, Unity and emulators,
  and unsupported host behavior.
- Show only a short concept name and one-line direction in the landing index.
  Hide catalog IDs in search metadata or URLs.
- Replace “Supported directly” and “Use a typed pattern” with reader-centered
  labels such as “direct replacement,” “requires a semantic rewrite,” and “not
  currently supported.” Define those labels once.
- Move unavailable features to a clearly named limitations section instead of
  mixing them through the supported workflow.
- Give each detailed concept page a consistent shape: source pattern, canonical
  SplitScript example, semantic difference to review, supported hosts, and
  related API links.
- Use a table only for a genuinely compact comparison. Paragraph-length cells
  should become headings, definition lists, or cards in the extension view.

The ASL guide itself has strong content, including its new compiler-clean port
review checklist. It still needs a start-to-finish “How to port a script”
workflow and grouped navigation, not another exhaustive catalog in front of it.

### P0 — Establish one canonical source for reference facts

The manual documents and generated reference currently disagree:

- `docs/LANGUAGE.md:414-416` says user functions are monomorphic and that
  generalized polymorphism is future work. `README.md:292-296` describes
  inferred generic schemes, `README.md:303` describes demand-monomorphized
  generic bodies, and the compiler-owned standard-library text also discusses
  user-authored generic functions.
- `README.md:364-365` says general-purpose suspending user functions remain
  future work. `docs/LANGUAGE.md:1840-1867` documents source-defined async
  helpers and first-class future values as implemented.
- `docs/STANDARD_LIBRARY.md:523-526` calls UTF-16 decoding, numeric formatting,
  and string construction “the next additions,” although those features are
  documented as present elsewhere in the same repository.
- `docs/STANDARD_LIBRARY.md:435-436` and `:473` refer to future browsable or
  generated documentation even though `splitc docs` and the VS Code reference
  already exist.
- `docs/LANGUAGE.md:2225` calls arrays, strings, structs, closures, and generic
  collections future GC values after the document has repeatedly described
  their current GC implementations.
- The generated `String` page includes `isBlank`, while the method table in
  `docs/LANGUAGE.md` does not.

The compiler-owned catalog should be canonical for exact signatures,
availability, effects, public members, and supported status. Hand-written
documentation should explain concepts and link to generated facts rather than
copying inventories. Generate checked static reference pages for repository and
web readers so the CLI, extension, and checked-in documentation cannot drift.

## Language usability findings

### The core domain model is good but introduced too late

The most important explanation in `docs/LANGUAGE.md` is the update order near
line 2204. A learner needs that model before pointer-path edge cases. Present it
near the beginning as a small lifecycle diagram:

```text
load settings
    -> observe timer events
    -> attach to a process
    -> run onAttach
    -> commit the first old/current snapshot
    -> run onStateReady
    -> on later ticks: whileAttached -> timer decisions
    -> on process close: onDetach
```

Then explain that failed required reads delay the first snapshot and retain the
last accepted value later. This single model makes `old`, `current`, `retry`,
and the lifecycle blocks much easier to understand.

### Lifecycle availability is powerful but too implicit

There are many top-level blocks with materially different rules: `setup`,
`onAttach`, `onStateReady`, `whileAttached`, `start`, `split`, `reset`,
`isLoading`, `gameTime`, `onDetach`, `onStart`, and `onReset`. The existing
return-default table is useful, but availability rules are spread across long
prose sections.

Add one user-facing lifecycle reference that gives every block the same four
facts:

- when and how often it runs;
- which of `process`, provider context, `layout`, `old`, `current`, `settings`,
  and attempt/attachment globals are available;
- whether it may `await` or `retry`; and
- its return type and fallthrough behavior.

Top-level globals whose lifetime is inferred from assignment in `onAttach` or
`onStart` are concise but surprising. Keep the inference, but make the editor
hover/inlay and diagnostics say “attachment-scoped,” “attempt-scoped,” or
“module-scoped.” Consider an optional explicit lifetime spelling only if user
testing shows that documentation and tooling are insufficient.

### State fields need a choice guide

The `state` declaration supports several different ideas through compact
syntax: static `at` paths, sibling-dependent paths, expression-backed fields,
optional fields that change failure behavior, trailing candidate filters,
named layouts, attachment-wide layout dimensions, and Unity schemas. Each
feature is defensible, but the combined surface is hard to select correctly.

Add a “Which state form should I use?” guide:

- fixed address or pointer chain read every tick -> `at`;
- address discovered once during attachment -> bare global initialized in
  `onAttach`, then an expression-backed state field;
- several values that must update atomically -> one struct or fixed array;
- absence is meaningful -> explicit `T?` field;
- failed reads should retain the last accepted value -> required field;
- a transient value should be rejected -> trailing field `if`;
- whole memory shape differs by build -> named state layouts;
- independent build facts affect native and managed fields -> layout
  dimensions; and
- managed Unity objects -> `state Unity` plus schemas.

Each branch should link to one complete example. The current reference explains
all of these in one page, but does not help the user make the initial choice.

### Error and async syntax needs a decision aid

`T?`, `T!`, postfix `?`, fallback `else`, `throw`, `retry`, and `await` form a
coherent system, but they are all unfamiliar in combination. `else` also serves
as both a normal branch keyword and a low-precedence unwrap fallback. The
compiler's precedence warning is a good safeguard; the documentation should
match it with a task-oriented explanation:

- value may be absent -> `T?` and `match`/`else`;
- operation can fail now -> `T!` and handle with `else`, `match`, or `?`;
- repeat a bounded synchronous operation next tick -> `retry`;
- wait on one already-asynchronous operation -> `await`; and
- unsupported build should remain attached but inert -> `await
  process.closed()`.

Show the same small memory-read example in each form. A side-by-side semantic
comparison is more helpful here than separate exhaustive descriptions.

### String units are precise but easy to misuse

SplitScript deliberately distinguishes UTF-8 byte offsets, Unicode scalar
widths, ASCII-only transformations, Unicode whitespace, UTF-16LE read bounds,
and Unity UTF-16 code-unit bounds. This precision is good for memory work, but
the rules are scattered. Add one “String units” guide that states the unit next
to each operation and calls out the common C#/JavaScript UTF-16 mismatch. Keep
the existing detailed porting recipes as follow-up material.

### Syntax presentation is inconsistent

`docs/LANGUAGE.md` uses `text` fences for most SplitScript examples and
`splitscript` for others. The opening README example also uses `text`. Use the
language fence consistently so static readers get highlighting and tooling can
identify examples. Show formatter-canonical punctuation consistently; the
language's newline/comma/semicolon policy is learnable, but mixed examples make
it look arbitrary.

## Documentation-surface findings

### Main repository README

The README is allowed to serve compiler contributors as well as language users.
The problem is routing, not the existence of developer material.

The opening immediately describes compiler passes and the public compiler
facade (`README.md:8-13`), then presents an advanced script, followed by five
production-port spotlights before the first heading. “Build and use” is almost
200 lines and mixes contributor checks, catalog architecture, CLI usage, LSP
protocol details, VS Code extension development, browser-extension internals,
debug metadata, and user commands. “What works now” is a very long inventory
that mixes language features, implementation representation, tooling, runtime
behavior, and test coverage.

Recommended README structure:

1. what SplitScript lets an autosplitter author do;
2. honest project status and host compatibility;
3. “Choose your path”: use the extension, use the CLI, port ASL, or contribute
   to the compiler;
4. a minimal source/compile example;
5. links to getting started, language concepts, generated reference, migration,
   examples, and contributor architecture;
6. a compact user-visible feature summary and limitations; and
7. contributor build/test instructions.

Move the individual production-port paragraphs to an `examples/README.md` or a
compatibility/evidence page. They are valuable proof for maintainers, but they
delay every new reader's first task. Reduce the LSP and debug-metadata inventory
to a short capabilities summary with a link to tooling architecture.

The opening example should compile without warnings. Its `metadata` binding is
unused, which teaches an avoidable warning in the first code a reader sees.

### Language reference

Keep source-visible behavior and move implementation explanations elsewhere.
Examples of content that does not help an ordinary author choose or write
SplitScript include:

- state snapshots being WebAssembly GC structs (`docs/LANGUAGE.md:153`);
- Wasm `i32` backend representation for narrow arithmetic (`:548`);
- async loop header/exit lowering (`:706`);
- Wasm IR reachability and DWARF compilation-unit details (`:746-775`);
- the typed HIR representation of `?` (`:871`);
- GC storage details for structs, arrays, ranges, and continuation frames; and
- the complete “Why GC and linear memory both appear” section (`:2217`).

Move those to `docs/COMPILER.md`, `docs/ABI.md`, or a dedicated debugging
implementation guide. Retain user-observable consequences: release removes
debug-only code, debug builds can be stepped through, strings are immutable,
arrays are mutable, and a GC-enabled host is required.

### Standard-library document

`docs/STANDARD_LIBRARY.md` has the same audience split inside one file. Its
provider, Unity, watcher, string, and timing explanations are relevant to
authors. The 275-line “Compiler and tooling model” section beginning at line
226 is compiler architecture: catalog IDs, HIR, Wasm IR, `CompilerContext`,
`TypeId`, `ExprId`, `ValueId`, union/find inference, and backend dispatch. It
belongs in `docs/COMPILER.md`.

After moving that section, make this document a task-oriented library overview
that links to generated exact API pages. Avoid manually repeating full member
inventories. The compiler-owned reference is already better suited to exact
signatures and public-member lists.

### Generated CLI and editor reference

This is a strong foundation. `splitc docs` has a useful index, exact symbol
pages, search, signatures, parameter documentation, effects, and examples. The
VS Code commands make it available where users write code.

Its landing page still lacks a getting-started guide, and some generated copy
exposes implementation rather than behavior. For example, the `String` page
says it stores text “in WebAssembly GC memory” and “owns its GC storage.” The
top-level type index includes mechanically rendered names such as
`FilterIterator<I where Iterator, S>` and
`T where Integer..<T where Integer`, which are difficult to parse before the
reader has learned the corresponding source syntax.

Key lifecycle pages are sometimes too terse. The `onAttach` page is one short
paragraph and one two-line example despite being central to cancellation,
layout selection, discovery, and pre-snapshot behavior. Add related links and
one conceptual sentence to key pages without turning each into another full
guide.

The rendered `Process.read` page says both “available everywhere” and “requires
an attached process.” That reflects separate compiler availability/effect
metadata, but reads as a contradiction to a user. Render the actionable rule:
“May only be called in attachment-owned contexts.”

### Static Markdown rendering

The hand-written guides are also source inputs to a compiler-specific renderer.
That renderer resolves links such as `[`at`](syntax@at)` and strips
rustdoc-style `# ` setup lines from examples. The repository files do not get
that transformation:

- `docs/ASL_PORTING.md` contains 178 hidden-scaffolding source lines;
- the C#, JavaScript, and Rust guides contain 12, 18, and 15 respectively; and
- the language, standard-library, and ASL documents contain compiler-only link
  destinations such as `syntax@at`, `method@UnityScenes.active`, and
  `provider@Unity`.

The README nevertheless links directly to these files. On GitHub, the hidden
lines remain visible and semantic links are plain or broken relative links.
Either publish renderer-produced Markdown snapshots for static readers or make
the repository links point to a web/extension documentation entry point. The
source representation should not masquerade as the finished guide.

### Background-language guides

The C#, JavaScript, and Rust guides are the best-sized conceptual documents in
the repository. They focus on differences that authors are likely to get wrong
and use short examples. Keep them concise.

Improve them by adding a small “start here next” section, making every static
example render cleanly, and linking each concept to both a tutorial and exact
reference page. A few explicit “source spelling -> SplitScript spelling” pairs
would make the migration purpose clearer without turning the guides into large
tables.

### Examples

There is no curated examples index and no obviously beginner-level example.
`examples/hello_lunistice.split` sounds introductory but is 77 lines and uses a
Unity schema, enums, structs, array mutation, module discovery, pointer size,
signature scans, pointer following, retries, relative reads, interpolation,
and custom variables. It is a feature/conformance probe, not “hello world.”

Add an `examples/README.md` that labels each file by purpose, difficulty,
provider, and concepts. Add small compiler-checked examples for:

- a first native-process split;
- one setting and one tooltip;
- load removal and game time;
- `T?` versus `T!`, `else`, `?`, `retry`, and `await`;
- one attach-time module/signature discovery;
- two named game layouts;
- one Unity schema;
- one emulator provider; and
- a complete but still readable production autosplitter.

Every example should state whether it is runnable, needs simulated host data,
or exists only as a compile/conformance fixture. Avoid names such as “testing,”
“weird,” and “showcase” in the user-facing index unless their exact purpose is
explained.

## Recommended documentation architecture

Use each surface for one job:

- `README.md`: mixed-audience project landing page and router.
- `docs/GETTING_STARTED.md`: first successful user workflow.
- Concept guides: lifecycle, state/memory, errors/async, settings/timer,
  strings/encoding, Unity, and emulators.
- Compiler-generated reference: exact syntax, signatures, members, effects,
  examples, and migration concept pages.
- `docs/ASL_PORTING.md` and `FROM_*.md`: source-background migration guides,
  with a grouped landing page rather than one exhaustive table.
- `examples/README.md`: curated examples map.
- `docs/COMPILER.md`, `docs/ABI.md`, ADRs, baselines, conformance records, and
  port audits: explicitly developer/maintainer-facing.
- `editors/vscode/README.md`: extension-user/Marketplace page.
- `editors/vscode/DEVELOPMENT.md`: extension contributor workflow.

This preserves the repository's valuable compiler and conformance information
without requiring a language author to read through it.

## Documentation quality safeguards

The existing compiler-checked examples are worth preserving and extending.
Add checks for the presentation layer as well:

- compile every tutorial's complete visible example;
- generate static reference and guide output in CI and fail if snapshots are
  stale;
- verify local links and anchors in the actual published Markdown, not only the
  compiler-specific source format;
- ensure hidden scaffolding never appears in published examples;
- require every public language construct and library symbol to have a summary,
  at least one useful example where appropriate, and related links;
- derive signatures, effects, and support status from the compiler catalog;
- keep roadmap statements out of durable user guides unless they are generated
  from one current status source; and
- review the VS Code packaged README as a user artifact, independently of the
  repository developer workflow.

## Suggested implementation order

### Phase 1: high impact, low structural risk

- Split the extension README into user and development documents.
- Add a minimal getting-started guide and true beginner example.
- Restructure the root README as an audience router.
- Correct the known contradictions and stale “future” statements.
- Add an examples index.
- Publish statically rendered versions of bundled guides so links and hidden
  lines work outside the extension.

### Phase 2: information architecture

- Split the language document into overview, concepts, and generated reference.
- Move the standard-library compiler/tooling section to compiler architecture.
- Replace the exhaustive migration table with source- and task-grouped pages.
- Add the lifecycle and state-form decision guides.

### Phase 3: ongoing consistency

- Generate all exact API and status facts from the compiler-owned catalogs.
- Add published-output link and example checks.
- Run short newcomer and ASL-porter usability tests against the new paths.

## Success criteria

The documentation is substantially more usable when:

- a new extension user can create and compile a small script without reading
  repository build instructions;
- a reader can explain the attachment/snapshot/lifecycle sequence after the
  first guide;
- the correct state-field and failure-handling form can be selected from a
  short task-based guide;
- an ASL porter can find one source concept without scanning an 86-row table;
- the repository, CLI, and VS Code reference agree on supported language
  behavior;
- every README-linked Markdown page renders working links and clean examples on
  GitHub; and
- compiler internals remain available to contributors without appearing in the
  primary language-author path.
