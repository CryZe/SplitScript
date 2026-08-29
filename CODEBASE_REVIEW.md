# SplitScript codebase review

Reviewed: 2026-08-29

## Executive summary

The codebase has unusually strong architectural documentation, a broad test suite, and several good single-source-of-truth mechanisms around the standard library, intrinsic metadata, runtime planning, and generated editor documentation. No P0 security, data-loss, or compiler-miscompilation issue was found in this review.

The main risks are now at the seams between those well-designed subsystems:

1. A legal-identifier mismatch makes completion incorrect for names containing `$`.
2. The VS Code build/watch command can switch documents while awaiting a save and compile a different, potentially non-SplitScript, file.
3. Editor requests repeatedly re-lex, clone, and sometimes re-check the full source. Full-sync diagnostics also invalidate every cached stage on every edit. This is the largest long-term responsiveness risk.
4. Several backend and type-checking coordinators have grown into very large functions operating through very broad context objects. The documented “over 1,000 lines” ownership signal is no longer representative of the repository.
5. The verification and extension host layers contain measurable duplicated work and duplicated control flow, which will make CI slower and behavior easier to drift.

The best next step is not a broad rewrite. Fix the two correctness defects, establish editor latency/heap benchmarks, then refactor the hot paths and large coordinators behind tests in small slices.

## Scope and method

This review covered the working-tree snapshot observed during the review of:

- the Rust compiler, parser, semantic model, code generator, service boundary, LSP, and documentation system;
- the syntax and standard-library crates;
- the native compiler and `xtask` verification tooling;
- the desktop and browser VS Code extension hosts;
- Rust, JavaScript, and TypeScript test infrastructure;
- manifests, lockfiles, workflow configuration, and architecture documentation.

Generated build output, `node_modules`, and packaged artifacts were excluded. The repository contained pre-existing uncommitted changes and was also being edited by another agent while this review ran; those changes and processes were preserved. The findings below are based on stable implementation patterns observed across that moving snapshot. Approximate reviewed size was 108,000 lines under `src`, 19,000 under `crates`, 32,000 under `tests`, and 1,800 TypeScript source lines in the VS Code extension.

Priority meanings:

- **P0**: release-blocking security, data loss, or systemic correctness issue;
- **P1**: address next because it is a concrete correctness issue or a likely user-visible scaling bottleneck;
- **P2**: schedule as maintainability/performance work before the affected area grows further;
- **P3**: useful hardening or cleanup with lower immediate impact.

## Findings at a glance

| ID | Priority | Area | Finding |
| --- | --- | --- | --- |
| F01 | P1 | Correctness | Completion disagrees with the language grammar about `$` in identifiers |
| F02 | P1 | Correctness | VS Code build/watch can change target document across an awaited save |
| F03 | P1 | Editor performance | Every edit replaces the full document, clears every query stage, and immediately runs diagnostics |
| F04 | P1 | Memory/performance | Compiler database stages retain and recreate deep copies of the source and syntax model |
| F05 | P2 | Architecture | Core backend/type-checking coordinators and their context objects have become too broad |
| F06 | P2 | Verification performance | Runtime verification recompiles identical artifacts 31 times |
| F07 | P2 | Duplication/testability | Node/browser extension hosts duplicate worker and language-client state machines |
| F08 | P2 | Editor performance | Completion re-lexes the source repeatedly and cursor lookups linearly scan ordered data |
| F09 | P2 | Resource use | Documentation page cache permanently stores arbitrary failed URI lookups |
| F10 | P2 | Test maintainability | Runtime harnesses repeat the same WASM host setup across dozens of files |
| F11 | P3 | Test diagnostics | Documentation stability is guarded by an opaque aggregate fingerprint |
| F12 | P3 | Release hygiene | Toolchain/action pinning, license files, and workspace metadata need tightening |

## Detailed findings

### F01 — Completion disagrees with the language grammar about `$` in identifiers

**Priority:** P1 correctness

The lexer explicitly permits `$` at the start of and inside an identifier (`crates/splitscript-syntax/src/lexer.rs:190` and `:482`). Rename validation agrees (`src/database/rename.rs:319-320`). Completion has a separate byte classifier that permits only ASCII alphanumeric bytes and `_` (`src/completion.rs:2391-2408`). Both identifier replacement and member-path discovery use that classifier.

For a legal declaration such as `$value`, invoking completion after `$va` computes a prefix/replacement beginning at `v`. `CompletionBuilder` then filters labels with `starts_with(prefix)` (`src/completion.rs:471-497`), so the legal `$value` candidate does not match. Member paths containing `$` can be segmented incorrectly for the same reason.

**Recommendation**

- Move identifier classification into the syntax crate and make lexer, rename, completion, and any refactoring features consume the same API.
- Prefer recovered token spans over rescanning source bytes when a token is already available.
- Add regression tests for root completion, member completion, rename, and Unicode-adjacent cursor offsets using `$value`, `obj.$field`, and an identifier containing `$` internally.

### F02 — VS Code build/watch can change target document across an awaited save

**Priority:** P1 correctness

`CompilerTaskController.savedActiveScript` first captures and validates the active SplitScript document, then awaits `document.save()` and reads `activeTextEditor` again (`editors/vscode/src/compilerTasks.ts:353-368`). The second document is checked only for `undefined`/untitled status; its language ID and identity are not checked.

If focus changes during the save, build/watch can compile a different document from the one the command validated. If the new editor is a saved non-SplitScript file, it still passes the second check and can receive the adjacent compiler output path. This is a classic async time-of-check/time-of-use race.

There are tests for the pure task ownership state, but no controller-level tests for `savedActiveScript`, output replacement, stale publish suppression, or disposal.

**Recommendation**

- Preserve the intended document identity across the save. Handle an untitled Save As transition explicitly rather than treating whatever editor is active afterward as the target.
- At minimum, revalidate language ID, URI scheme, and the relationship to the originally captured document after the await.
- Extract the controller’s decision logic behind a small VS Code adapter so focus changes, failed saves, stale revisions, and temporary-file cleanup can be unit tested.
- Also wrap both temporary-file write and rename in cleanup logic. Currently a `writeFile` failure occurs before the cleanup `try` block (`editors/vscode/src/compilerTasks.ts:398-415`).

### F03 — Every edit replaces the full document, clears every query stage, and immediately runs diagnostics

**Priority:** P1 editor performance/scalability

The LSP advertises full document synchronization (`src/lsp.rs:78`) and `didChange` immediately publishes diagnostics (`src/lsp.rs:238-252`). `CompilerDatabase::set_source` replaces the complete string and resets `QueryCache` to its default (`src/database/queries.rs:114-126`). The language server runs in a worker, which protects the extension UI thread, but requests to that worker are still serialized: completion, hover, and navigation can wait behind a full diagnostic pass for a superseded keystroke.

This is not necessarily visible on today’s examples, but its cost grows with source size and edit frequency. The architecture has revision checks around compilation, yet the interactive analysis path lacks comparable debouncing, coalescing, or cancellation.

**Recommendation**

1. Add timing and allocation telemetry/benchmarks for `didChange -> diagnostics`, root completion, and member completion on representative small, medium, and large files.
2. Debounce diagnostics and coalesce queued edits by URI/version in the extension worker. Never publish a result for a superseded version.
3. Add cancellation checkpoints to the recovering analysis path, not just artifact compilation.
4. Only after measuring, consider incremental text synchronization and stage-level invalidation. The first three changes should provide value without committing to an incremental compiler rewrite.

Acceptance targets should be explicit—for example, p95 completion latency and maximum retained bytes after repeated edits—so this does not become an open-ended optimization project.

### F04 — Compiler database stages retain and recreate deep copies of the source and syntax model

**Priority:** P1 memory/performance

The query cache stores recovered, parsed, lowered, checked, recovering-checked, and semantic snapshot products concurrently (`src/database/cache.rs`). Creating those products repeatedly clones owned structures:

- recovered parse to strict parse clones `SourceDocument`, syntax, and diagnostics (`src/database/queries.rs:146-160`);
- recovering lowering clones syntax twice and clones the document (`src/database/queries.rs:222-265`);
- strict lowering clones the complete `ParsedProgram` (`src/database/queries.rs:277`);
- checking clones the complete `LoweredProgram` (`src/database/queries.rs:289` and `:304`);
- semantic highlighting clones the semantic model (`src/database/queries.rs:180-184`);
- completion clones the source and recovered syntax at entry (`src/completion.rs:83-87`), while signature help has similar probe-oriented ownership.

These types own strings, vectors, ASTs, HIR, and resolution maps, so this is structurally more than cheap reference-count cloning. A single editor revision can retain multiple representations containing duplicate source/syntax data. Member completion may then create a separate database for a probe, compounding peak allocation.

**Recommendation**

- Make immutable stage inputs shared: `Arc<SourceDocument>`, `Arc<Program>`, and, where useful, `Arc<ProgramResolutions>` or a shared parsed-stage object.
- Let later stage results reference immutable prior-stage data instead of embedding owned copies. Keep only genuinely transformed/derived data stage-local.
- Have APIs consume `Arc<T>` or borrow `&T` where ownership is not required. Do not fix this with more derived `Clone` calls.
- Measure retained heap per document revision before and after the change, and add a benchmark that exercises parse, semantic tokens, diagnostics, hover, and completion on the same database.

This is a good precursor to incremental analysis because it clarifies which data is immutable and shareable without changing semantics.

### F05 — Core backend/type-checking coordinators and their context objects have become too broad

**Priority:** P2 architecture/maintainability

`docs/COMPILER.md:1342-1356` says files above roughly 1,000 lines receive ownership review and lists seven modules above that signal. The current tree has at least 39 production Rust source files above 1,000 lines. Line count is only a signal, but the documented exception list has ceased to describe reality.

The strongest examples are not merely large files; they are functions with many unrelated reasons to change:

- `compile_expr_unconverted` is approximately 1,950 lines (`src/codegen/expression.rs:2862`) and handles structural expression lowering, control flow, calls, and many intrinsic families.
- `compile_suspension_poll` is approximately 660 lines (`src/codegen/async_state.rs:1767`) and dispatches across async/runtime operation families.
- `lower_async_statements` is approximately 530 lines (`src/wasm_ir.rs:3778`).
- `Checker::call` is approximately 485 lines (`src/typeck/call_resolution.rs:331`), while path-call resolution adds another large coordinator at `:1891`.

`ExprContext` has more than 40 fields (`src/codegen/expression.rs:97-149`) spanning catalogs, plans, storage, semantics, IR, GC, debugging, locals, globals, settings, and control-flow state. `EmissionContext` explicitly notes that narrower inputs are preferable (`src/codegen/context.rs:30-31`), but the dominant expression path still has access to almost everything.

The cost is high review surface and accidental coupling: a new intrinsic or expression form can touch a giant dispatch function and a context shared by otherwise independent emitters. It also makes focused tests harder because constructing a narrow behavior requires a complete compilation world.

**Recommendation**

- Split by semantic ownership, not by arbitrary line targets. Useful seams are structural expressions, call dispatch, numeric intrinsics, string/collection intrinsics, process/memory intrinsics, and suspension operations.
- Introduce narrow domain emitters that receive a small immutable program/runtime view plus explicit per-function mutable state. Avoid turning the current context into nested “bags of everything.”
- Move one tested match-arm family at a time; keep the top-level coordinator as a short dispatcher.
- Separate call candidate collection, overload selection, generic solving, and diagnostic construction in type checking. They should have independently testable inputs and outputs.
- Replace the stale module list in `COMPILER.md` with a generated metric or a policy that records reviewed ownership boundaries rather than enumerating files manually.

### F06 — Runtime verification recompiles identical artifacts 31 times

**Priority:** P2 verification performance

`RUNTIME_FIXTURES` currently contains 93 scenarios but only 62 unique `(source, output, profile)` compilation tuples. The loop at `src/bin/xtask.rs:819-828` compiles once per scenario, so 31 compiler invocations recreate an artifact already produced in the same run. Validation later sorts and deduplicates output paths (`src/bin/xtask.rs:862-878`), showing that artifact uniqueness is already recognized at the next stage. One current artifact is compiled nine identical times solely to feed different runtime arguments.

The 600+ line literal fixture table also repeats source/output/profile/harness values when only scenario arguments differ.

**Recommendation**

- Model one compiled artifact with a list of runtime scenarios, or derive a unique compile map before invoking `compile_once`.
- Validate each unique module once, then run every harness/argument scenario against it.
- Add a manifest validation test that rejects conflicting definitions of the same output and reports accidental duplicate scenarios.

This removes one third of the runtime-table compilation jobs without reducing coverage.

### F07 — Node/browser extension hosts duplicate worker and language-client state machines

**Priority:** P2 duplication/testability

The following pairs are near-parallel implementations with host-specific transport differences embedded in otherwise shared lifecycle logic:

- `editors/vscode/src/embeddedCompilerNodeWorker.ts` and `embeddedCompilerBrowserWorker.ts`;
- `editors/vscode/src/embeddedCompilerWorkerClient.ts` and `embeddedCompilerBrowserWorkerClient.ts`;
- `editors/vscode/src/languageClient.ts` and `languageClientBrowser.ts`.

They duplicate initialization, active-request tracking, cancellation, error propagation, start/restart, send, and stop behavior. The shared `EmbeddedCompilerWorkerConnection` is tested, but the duplicated worker controllers and language-client lifecycle are not. A future fix to cancellation or restart semantics has multiple places to drift.

**Recommendation**

- Extract a host-neutral compiler worker state machine parameterized by `postMessage`, error reporting, and port subscription adapters.
- Extract language-server lifecycle into one controller parameterized by worker creation and Node/browser transport construction.
- Keep the Node and browser entry points thin and explicit; they should contain environment setup, not protocol policy.
- Add contract tests that run the same lifecycle cases through both adapters.

### F08 — Completion re-lexes the source repeatedly and cursor lookups linearly scan ordered data

**Priority:** P2 editor performance

The recovered `SourceDocument` already owns the lexeme stream and exposes tokens (`crates/splitscript-syntax/src/source.rs:36-45`). Completion nevertheless calls `lex_lossless(source)` in multiple sequential context probes:

- `src/completion.rs:202`, `:285`, `:2082`, and `:2319`;
- `src/completion/settings.rs:18`;
- `src/completion/top_level.rs:114`;
- `src/completion/types.rs:22`;
- `src/completion/state.rs:35`.

A single completion request can therefore tokenize the whole document several times. Member completion is more expensive: `infer_receiver` builds a fresh `CompilerDatabase`, analyzes the whole source, and may build/analyze a repaired probe source a second time (`src/completion.rs:2247-2311`). It returns a cloned complete syntax `Program` even though callers need a small set of receiver facts.

Separately, `SourceDocument::token_at` linearly scans all lexemes (`crates/splitscript-syntax/src/source.rs:53-67`), and `DefinitionIndex::reference_at` linearly scans references that were explicitly sorted (`src/database.rs:238-244` and `:269-273`). These are frequent cursor-position operations and should exploit ordering.

**Recommendation**

- Build one `CompletionContext` per request containing the source, recovered document/token stream, syntax, cursor, and lazily requested semantic facts.
- Pass token slices/indexes into the context-specific completion helpers instead of re-lexing.
- Query receiver facts from the existing database first. Run a repair probe only if the existing semantic snapshot cannot supply a usable type, and return a compact `ReceiverAnalysis` rather than a full `Program`.
- Use `slice::partition_point`/binary search for token and reference-at-offset lookup, with tests for trivia, zero-width tokens, equal boundaries, and overlapping semantic references.

### F09 — Documentation page cache permanently stores arbitrary failed URI lookups

**Priority:** P2 resource use

`PAGE_CACHE` is a process-global unbounded `HashMap<(bool, String), OnceLock<Option<Page>>>` (`src/documentation/reference.rs:95`). `DocumentationReference::page` inserts a cache entry before checking whether the URI is valid and caches `None` permanently (`src/documentation/reference.rs:456-467`). The LSP documentation page request accepts a URI supplied by the client.

The valid page graph is finite, but failed lookups are not. A buggy extension, repeated malformed link, or local client issuing unique unknown URIs can grow the compiler process for its lifetime. This is primarily a robustness issue rather than a remote security issue in the current deployment model.

**Recommendation**

- Validate/canonicalize against the finite documentation route space before inserting into the cache, or cache only successful renders.
- Alternatively use a bounded cache for non-catalog routes, though a finite validated key space is simpler here.
- Cache the immutable documentation index and normalized search fields as well; `index()`/`search()` currently reconstruct and normalize strings per query (`src/documentation/reference.rs:123` and `:362`).
- Add a test asserting that many unknown URI requests do not grow cache state.

### F10 — Runtime harnesses repeat the same WASM host setup across dozens of files

**Priority:** P2 test maintainability

There are 66 focused `*_runtime.mjs` files. Many independently repeat module loading, `TextDecoder`, memory-string helpers, timer/process import stubs, instantiation, and `_start`/`update` invocation even though `tests/support/splitscript_host.mjs` already centralizes a richer host for maintained ports.

Some repetition is valuable: a focused fixture should not have to boot an oversized host merely to assert one ABI behavior. The current degree of copy/paste, however, means an ABI import change requires many manual edits and can leave subtly different default behavior between tests.

**Recommendation**

- Add a small `instantiateRuntimeFixture` helper that supplies the common minimal imports and permits per-test overrides.
- Keep scenario assertions and unusual host behavior local to each harness.
- Migrate opportunistically when touching a fixture rather than rewriting every test at once.
- Pair this with the grouped artifact/scenario model from F06 so the test manifest expresses compilation and runtime concerns separately.

### F11 — Documentation stability is guarded by an opaque aggregate fingerprint

**Priority:** P3 test diagnostics

`rendered_reference_snapshot_is_stable` asserts a page count and one 64-bit fingerprint over the root page plus flattened index metadata (`src/documentation/validation.rs:368-406`). It is deterministic, but a failure reports only two integers. It does not identify which URI, title, signature, summary, or root content changed, and a maintainer can update the constant without seeing the semantic diff. The fingerprint also does not include every rendered page body.

**Recommendation**

- Store a human-readable checked-in snapshot of stable page metadata, or compute per-page fingerprints and print only changed URIs/fields on failure.
- Provide an explicit update command that regenerates the snapshot for review.
- Retain graph/link validation as the semantic guard; use the snapshot for reviewable change detection rather than treating an aggregate hash as explanation.

### F12 — Toolchain/action pinning, license files, and workspace metadata need tightening

**Priority:** P3 release/reproducibility

- CI uses `dtolnay/rust-toolchain@stable` (`.github/workflows/check.yml:28`) and there is no checked-in `rust-toolchain.toml`. Because clippy warnings are denied, a new stable release can break an unchanged commit.
- GitHub Actions are referenced by moving major tags (`actions/checkout@v7`, `actions/setup-node@v7`) rather than commit SHA. In contrast, the downloaded `wasm-tools` binary is versioned and checksum-verified, which is the stronger pattern.
- Cargo manifests declare `MIT OR Apache-2.0`, but no `LICENSE-MIT` or `LICENSE-APACHE` file is present in the repository.
- Version, edition, and license metadata are repeated across the root and three workspace crate manifests rather than inherited from `[workspace.package]`.

**Recommendation**

- Pin Rust with `rust-toolchain.toml` and update it deliberately/automatically.
- Pin third-party Actions to full commit SHAs, with a dependency updater responsible for refreshes.
- Add both declared license texts and ensure packaged release artifacts include the appropriate license metadata/files.
- Use `[workspace.package]` plus `version.workspace = true`, `edition.workspace = true`, and `license.workspace = true` for Rust crates. Keep the VS Code product version separate if releases are intentionally independent, but add a release check if versions are expected to move together.

## Recommended implementation order

### Phase 1 — correctness and guardrails

1. Fix F01 and add cross-feature identifier grammar tests.
2. Fix F02 and introduce controller-level tests around save/focus races and output replacement.
3. Validate documentation cache keys (F09), which is a small, contained hardening change.
4. Add editor latency/allocation benchmarks before changing ownership or analysis behavior.

### Phase 2 — editor responsiveness

1. Introduce a shared request-scoped token/completion context and binary-search cursor indexes (F08).
2. Debounce/coalesce diagnostics and suppress superseded revisions (F03).
3. Replace deep stage copies with shared immutable ownership (F04).
4. Profile again before considering incremental parsing or fine-grained query invalidation.

### Phase 3 — backend ownership

1. Narrow expression emission inputs and extract one intrinsic domain at a time (F05).
2. Decompose call resolution into candidate collection, selection/solving, and diagnostics.
3. Update architecture documentation with enforceable/generated ownership signals.

### Phase 4 — build and host deduplication

1. Compile each runtime artifact once and group scenarios (F06).
2. Add the minimal shared runtime host helper and migrate touched fixtures (F10).
3. Consolidate Node/browser worker and language-server lifecycles behind adapters (F07).
4. Finish release/reproducibility hardening (F11-F12).

## Validation performed

The following checks were run during the review:

- Rust formatting and clippy checks completed successfully in the baseline `cargo xtask check` pass.
- `cargo test --lib`: **341 passed, 0 failed**.
- Syntax and standard-library crate suites: **117 combined tests passed** during the verification run.
- VS Code TypeScript application check: passed by invoking the local TypeScript compiler directly.
- VS Code TypeScript test compilation: passed.
- VS Code unit tests: **14 passed, 0 failed**.

The full end-to-end verification result is intentionally not treated as authoritative because another agent was changing and testing the same working tree concurrently; transient formatting and locked-test-binary failures were observed as that snapshot moved. The machine’s global `npm` launcher also points at a missing global `npm-cli.js`; local TypeScript and Node tests were invoked directly, so this is an environment issue rather than a repository failure. Rust reported Windows “access denied” warnings while finalizing incremental compilation directories; completed tests passed, but incremental artifacts could not be reused.

## What is already working well

- The compiler stages and backend/runtime contract are documented in much more detail than is typical for a project of this size.
- Standard-library source, generated IDs/catalog data, documentation, and intrinsic contracts have strong single-authority tests that actively prevent parallel registries from returning.
- Deterministic collections and explicit runtime/codegen plans make output stability an architectural property rather than an incidental one.
- The test suite covers parser recovery, typing, completion, hover, semantic tokens, code actions, service cancellation, native/embedded boundaries, and runtime behavior at multiple layers.
- Embedded compilation uses revisioned responses and staged cancellation, and output publication already includes stale-task ownership checks.
- The verification workflow checksum-validates downloaded WASM tooling.

These strengths make the recommended refactors feasible: most changes can be performed behind existing contracts and validated incrementally rather than as a compiler rewrite.
