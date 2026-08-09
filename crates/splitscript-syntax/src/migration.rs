//! Structured knowledge for migrating autosplitters from familiar languages.
//!
//! This catalog deliberately does not redeclare SplitScript's language or
//! standard-library APIs. Instead, it maps foreign concepts and spellings to
//! canonical catalog names that the compiler validates against the active
//! language and standard-library graphs.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationConceptId(&'static str);

impl MigrationConceptId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationDiagnosticId(&'static str);

impl MigrationDiagnosticId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceLanguage {
    Asl,
    CSharp,
    JavaScript,
    Rust,
}

impl SourceLanguage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Asl => "ASL",
            Self::CSharp => "C#",
            Self::JavaScript => "JavaScript",
            Self::Rust => "Rust",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationSupport {
    Direct,
    TypedPattern,
    Planned,
    SandboxNonGoal,
}

impl MigrationSupport {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "Supported directly",
            Self::TypedPattern => "Use a typed pattern",
            Self::Planned => "Planned",
            Self::SandboxNonGoal => "Intentional sandbox non-goal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationTarget {
    Language(&'static str),
    StandardLibraryType(&'static str),
    StandardLibraryItem(&'static str),
    StateProvider(&'static str),
}

impl MigrationTarget {
    pub const fn display(self) -> &'static str {
        match self {
            Self::Language(name)
            | Self::StandardLibraryType(name)
            | Self::StandardLibraryItem(name)
            | Self::StateProvider(name) => name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForeignSpellingContext {
    VariableDeclaration,
    VariableModifier,
    OptionalValue,
    FunctionDeclaration,
    Type,
    StaticTypeReceiver,
    Method,
    ValuePath,
    AttachedProcessValuePath,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignSpellingReplacement {
    Text(&'static str),
    Remove,
}

impl ForeignSpellingReplacement {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Text(value) => value,
            Self::Remove => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignSpelling {
    pub source: SourceLanguage,
    pub context: ForeignSpellingContext,
    pub spelling: &'static str,
    pub replacement: ForeignSpellingReplacement,
    pub message: &'static str,
    pub primary_label: &'static str,
    pub fix_title: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationConcept {
    pub id: MigrationConceptId,
    pub name: &'static str,
    pub sources: &'static [SourceLanguage],
    pub support: MigrationSupport,
    pub summary: &'static str,
    pub targets: &'static [MigrationTarget],
    pub cookbook_anchor: Option<&'static str>,
    pub spellings: &'static [ForeignSpelling],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationDiagnostic {
    pub id: MigrationDiagnosticId,
    pub concept: MigrationConceptId,
    pub message: &'static str,
    pub primary_label: &'static str,
    pub notes: &'static [&'static str],
}

pub const ASL_STRING_N_FIELD_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.string-n-field");
pub const DUPLICATE_STATE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.duplicate-version-layout");
pub const ASL_STARTUP_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.startup-block");
pub const ASL_INIT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.init-block");
pub const ASL_UPDATE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.update-block");
pub const ASL_EXIT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.exit-block");
pub const ASL_SHUTDOWN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.shutdown-block");
pub const ASL_TIMER_EVENT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.timer-event-block");
pub const ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.current-split-index-path");
pub const ASL_MONOTONIC_TIME_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.time.monotonic-path");
pub const ASL_TIMER_REAL_TIME_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.current-real-time-path");
pub const ASL_MUTABLE_CURRENT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.mutable-current-assignment");
pub const ASL_LIST_TYPE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.collection.list-type");
pub const ASL_SETTINGS_ADD_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.settings.add-call");
pub const CSHARP_STRING_EQUALS_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.equals-call");
pub const CSHARP_STRING_SUBSTRING_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.substring-call");
pub const CSHARP_NUMERIC_PARSE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.numeric.static-parse-call");
pub const CSHARP_TIMESPAN_PARSE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.timespan.parse-call");
pub const CSHARP_TIMESPAN_TICKS_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.timespan.from-ticks-call");

pub const DIAGNOSTICS: &[MigrationDiagnostic] = &[
    MigrationDiagnostic {
        id: ASL_STRING_N_FIELD_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.string-n"),
        message: "ASL `stringN` fields need an explicit SplitScript memory decoder",
        primary_label: "this ASL pseudo-type combines a byte bound with automatic string decoding",
        notes: &[
            "ASL `stringN` reads N bytes, then auto-detects UTF-16LE from the second byte; SplitScript requires the encoding to be chosen from evidence",
            "`utf8` bounds are bytes, while `utf16le` bounds are two-byte code units; both suggested rewrites are intentionally maybe-incorrect",
        ],
    },
    MigrationDiagnostic {
        id: DUPLICATE_STATE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.version-label"),
        message: "SplitScript uses one `state` declaration with named layouts for game versions",
        primary_label: "merge this state declaration into named `layout` blocks",
        notes: &[
            "compatible fields form a common interface; missing or conflicting fields are accessed after `match layout` refines the selected `StateLayout` variant",
            "`onAttach` returns the selected `StateLayout` variant before polling begins",
            "merging is not automatic because versioned fields and pointer paths may differ semantically",
        ],
    },
    MigrationDiagnostic {
        id: ASL_STARTUP_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.startup"),
        message: "ASL `startup` is not a SplitScript lifecycle block",
        primary_label: "split startup work according to what it initializes",
        notes: &[
            "declare controls in `settings` and constants with global `let` declarations",
            "put remaining process-independent startup statements in `setup`; it runs once after settings are initialized",
            "do not move process discovery into `setup`; use `onAttach` for each selected process",
            "legacy timer event subscriptions are a separate migration concern and do not become `start`, `split`, or `reset` decision blocks",
        ],
    },
    MigrationDiagnostic {
        id: ASL_INIT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.init"),
        message: "ASL `init` has no blind one-to-one lifecycle rename",
        primary_label: "choose the destination from the state this block needs",
        notes: &[
            "use `onAttach` for suspending process discovery and layout selection before SplitScript starts polling",
            "legacy ASL runs `init` after an initial state refresh; code that truly needs that first snapshot needs an explicit guarded first `whileAttached` tick for now",
        ],
    },
    MigrationDiagnostic {
        id: ASL_UPDATE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.update"),
        message: "ASL `update` is named `whileAttached` for ordinary per-tick work",
        primary_label: "review the block's boolean control result before moving it",
        notes: &[
            "`whileAttached` runs after a successful state refresh and before timer-decision blocks",
            "ASL `return false` skips all remaining decisions for that tick; SplitScript does not yet have an exact equivalent",
        ],
    },
    MigrationDiagnostic {
        id: ASL_EXIT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.exit"),
        message: "ASL `exit` is not exactly the same as `onDetached`",
        primary_label: "guard process-exit-only cleanup when using `onDetached`",
        notes: &[
            "ASL `exit` runs after an attached process exits",
            "SplitScript `onDetached` also runs once before the first attachment, so process-exit-only work needs an attached-once guard",
        ],
    },
    MigrationDiagnostic {
        id: ASL_SHUTDOWN_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.shutdown"),
        message: "ASL `shutdown` has no SplitScript equivalent yet",
        primary_label: "script teardown requires a host lifecycle callback",
        notes: &[
            "do not use `onDetached`: it runs for process transitions rather than only when the script is disabled, reloaded, or dropped",
            "the runtime evolution plan records the required teardown export and ordering contract",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_EVENT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.events"),
        message: "ASL timer event handlers are not SplitScript decision blocks",
        primary_label: "an external timer event can occur independently of this script's decision",
        notes: &[
            "simple run-start state can be reconstructed from `timer.state()` in `whileAttached`",
            "exact `onStart`, `onSplit`, and `onReset` delivery needs the planned ordered host event contract",
        ],
    },
    MigrationDiagnostic {
        id: ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.current-split-index"),
        message: "ASL `timer.CurrentSplitIndex` is optional in SplitScript",
        primary_label: "call `timer.currentSplitIndex()` and handle the no-attempt case",
        notes: &[
            "`timer.currentSplitIndex()` returns `u64?`; `None` represents every negative host value when no attempt is in progress",
            "a decision block commonly uses `let index = timer.currentSplitIndex() else return false`, while other contexts can use `match`",
            "the index also advances for skipped segments and equals the segment count after the final split",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MONOTONIC_TIME_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.time.monotonic-delay"),
        message: "use SplitScript's monotonic clock for elapsed-time checks",
        primary_label: "capture `Instant.now()` at the event and compare an exact `Duration`",
        notes: &[
            "`Instant` is appropriate for debouncing, cooldowns, and delayed actions because it never moves backwards during one runtime instance",
            "it has no calendar value; logging timestamps and other wall-clock uses are intentionally outside this API",
            "use `startedAt.hasElapsed(Duration.fromMilliseconds(...))` for the common polling pattern",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_REAL_TIME_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.current-real-time"),
        message: "LiveSplit run real time is not the same as SplitScript's monotonic clock",
        primary_label: "determine whether this code needs an independent delay or actual timer metadata",
        notes: &[
            "for a delay anchored to a game event, store `Instant.now()` at that event and compare a `Duration`",
            "the exact LiveSplit `CurrentTime.RealTime` phase is not exposed by the current host contract",
            "do not silently replace run-relative time used for offsets or game-time calculations with elapsed process-independent time",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MUTABLE_CURRENT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.mutable-current"),
        message: "SplitScript state snapshots are immutable",
        primary_label: "move this transformation to the state declaration or script-owned state",
        notes: &[
            "to retain an old field when a transient candidate is read, add a trailing `if` to that state field and return `Err(message)` for the rejected candidate",
            "after initialization, rejecting one field retains its last accepted value while successful sibling fields continue to advance; rejecting the initial candidate prevents publishing a fabricated snapshot",
            "computed values that are not process snapshots belong in a state-field expression or an ordinary global `let`, depending on whether they should refresh with memory or remain script-owned",
            "there is no automatic rewrite because an assignment to `current` can represent filtering, derivation, or mutable run state in legacy ASL",
        ],
    },
    MigrationDiagnostic {
        id: ASL_LIST_TYPE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.collection.list"),
        message: "`List<T>` has no single SplitScript replacement",
        primary_label: "choose the collection from the behavior the script requires",
        notes: &[
            "use `[T]` for a fixed ordered table; `.contains(value)` searches it and `.indexOf(value)` returns `u32?`, with `None` instead of C#'s `-1` sentinel",
            "use `Set<T>` for growable unique membership such as visited maps; construct it with `Set.new<T>()` and use `insert`, `contains`, `remove`, and `clear`",
            "a growable ordered collection that preserves duplicates is not currently provided; do not silently replace one with a set",
            "there is no automatic rewrite because choosing an array or set changes ordering, duplication, mutation, and absence semantics",
        ],
    },
    MigrationDiagnostic {
        id: ASL_SETTINGS_ADD_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.settings.registration"),
        message: "ASL `settings.Add` calls become declarations in SplitScript",
        primary_label: "settings are declared statically rather than registered at runtime",
        notes: &[
            "declare one boolean setting as `\"Label\" => name key \"host-key\": true,` inside `settings`",
            "for a bounded numbered family, use `for value in start..=end { `Label {value}` key `{value}`: true, }` instead of expanding every member by hand",
            "read a statically named setting through `settings.name`; use `settings.enabled(key)` only when a data-driven string selects among declared boolean settings",
            "there is no automatic rewrite because `settings.Add` overloads encode labels, parents, defaults, and runtime control flow differently",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_EQUALS_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.equality"),
        message: "C# `String.Equals` becomes an equality expression in SplitScript",
        primary_label: "compare string values with `==` or `!=`",
        notes: &[
            "`left == right` compares immutable strings by their exact UTF-8 text, not by object identity",
            "use `left.equalsIgnoreAsciiCase(right)` only when the game identifier intentionally ignores ASCII letter case",
            "there is no automatic rewrite because C# also has static and comparison-mode overloads whose semantics must be reviewed",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_SUBSTRING_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.substring"),
        message: "C# `String.Substring` needs an explicit UTF-8 boundary review",
        primary_label: "SplitScript `slice` uses UTF-8 byte offsets and an exclusive end",
        notes: &[
            "for proven ASCII text, `value.Substring(start, length)` becomes `value.slice(start, start + length)` with ordinary Result handling",
            "for proven ASCII text, `value.Substring(start)` becomes `value.slice(start, value.byteLength())`",
            "C# indexes UTF-16 code units; non-ASCII offsets cannot be copied into `slice` because SplitScript indexes UTF-8 bytes and rejects positions inside a character",
            "there is no automatic rewrite because the compiler cannot prove the source text is ASCII or recover C# overload semantics from the method name alone",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_NUMERIC_PARSE_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.numeric-parse"),
        message: "C# static numeric parsing becomes `String.parse<T>()` in SplitScript",
        primary_label: "move the target type to the receiving boundary",
        notes: &[
            "rewrite `Int32.Parse(text)` as `let value: i32 = text.parse()?`, or use `else` when malformed input needs a fallback",
            "rewrite `TryParse` output parameters with ordinary Result control flow such as `match` or `else`; SplitScript does not mutate an out argument",
            "SplitScript parsing consumes strict ASCII decimal text in full; whitespace, separators, trailing text, and integer overflow are errors",
            "there is no automatic rewrite because the call alone does not identify the receiving declaration, fallback behavior, or `TryParse` control flow",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_TIMESPAN_PARSE_DIAGNOSTIC,
        concept: MigrationConceptId::new("duration.parse"),
        message: "C# `TimeSpan.Parse` needs an explicit duration migration",
        primary_label: "review whether this text is data or a serialized timer value",
        notes: &[
            "for a fixed literal, construct the exact value with `Duration.fromWholeSeconds`, `Duration.fromWholeMilliseconds`, or `Duration.fromParts` rather than preserving a runtime parser",
            "when the input came from `timer.CurrentTime` or another duration converted to text, keep the value typed instead of serializing and reparsing it",
            "C# parsing is culture-sensitive and accepts a broad grammar; SplitScript does not provide a compatibility parser whose meaning could vary by host locale",
            "there is no automatic rewrite because the call does not reveal the input format or whether the surrounding timer API itself needs redesign",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_TIMESPAN_TICKS_DIAGNOSTIC,
        concept: MigrationConceptId::new("duration.csharp-ticks"),
        message: "C# ticks must be converted to a language-level duration unit",
        primary_label: "one C# tick is exactly 100 nanoseconds",
        notes: &[
            "for a proven in-range tick count, use `Duration.fromNanoseconds(ticks * 100)` with an explicit `i64` value",
            "prefer the source's native seconds, milliseconds, frames, or nanoseconds when available instead of preserving a C# representation detail",
            "multiplying an arbitrary signed 64-bit tick count by 100 can overflow even though C# `TimeSpan` accepts that tick count",
            "there is no automatic rewrite because SplitScript's exact nanosecond constructor cannot represent the full C# tick range through a single `i64` nanosecond count",
        ],
    },
];

pub fn legacy_string_method_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    match name {
        "Equals" => Some(CSHARP_STRING_EQUALS_DIAGNOSTIC),
        "Substring" => Some(CSHARP_STRING_SUBSTRING_DIAGNOSTIC),
        _ => None,
    }
}

pub fn legacy_static_call_diagnostic(path: &[String]) -> Option<MigrationDiagnosticId> {
    let [owner, method] = path else {
        return None;
    };
    if owner == "Duration" && method == "Parse" {
        return Some(CSHARP_TIMESPAN_PARSE_DIAGNOSTIC);
    }
    if owner == "Duration" && method == "FromTicks" {
        return Some(CSHARP_TIMESPAN_TICKS_DIAGNOSTIC);
    }
    if method != "Parse" && method != "TryParse" {
        return None;
    }
    matches!(
        owner.as_str(),
        "SByte"
            | "Byte"
            | "Int16"
            | "UInt16"
            | "Int32"
            | "UInt32"
            | "Int64"
            | "UInt64"
            | "Single"
            | "Double"
            | "sbyte"
            | "byte"
            | "short"
            | "ushort"
            | "int"
            | "uint"
            | "long"
            | "ulong"
            | "float"
            | "double"
    )
    .then_some(CSHARP_NUMERIC_PARSE_DIAGNOSTIC)
}

pub fn legacy_lifecycle_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    Some(match name {
        "startup" => ASL_STARTUP_DIAGNOSTIC,
        "init" => ASL_INIT_DIAGNOSTIC,
        "update" => ASL_UPDATE_DIAGNOSTIC,
        "exit" => ASL_EXIT_DIAGNOSTIC,
        "shutdown" => ASL_SHUTDOWN_DIAGNOSTIC,
        "onStart" | "onSplit" | "onReset" => ASL_TIMER_EVENT_DIAGNOSTIC,
        _ => return None,
    })
}

pub fn legacy_value_path_diagnostic(path: &str) -> Option<MigrationDiagnosticId> {
    if path == "timer.CurrentSplitIndex" {
        return Some(ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC);
    }
    if path == "DateTime.Now"
        || path.starts_with("DateTime.Now.")
        || path == "System.DateTime.Now"
        || path.starts_with("System.DateTime.Now.")
    {
        return Some(ASL_MONOTONIC_TIME_DIAGNOSTIC);
    }
    if path == "timer.CurrentTime.RealTime" || path.starts_with("timer.CurrentTime.RealTime.") {
        return Some(ASL_TIMER_REAL_TIME_DIAGNOSTIC);
    }
    None
}

pub fn legacy_type_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    match name {
        "List" => Some(ASL_LIST_TYPE_DIAGNOSTIC),
        _ => None,
    }
}

const ASL: &[SourceLanguage] = &[SourceLanguage::Asl];
const CSHARP: &[SourceLanguage] = &[SourceLanguage::CSharp];
const JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::JavaScript];
const CSHARP_JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::CSharp, SourceLanguage::JavaScript];
const ASL_CSHARP: &[SourceLanguage] = &[SourceLanguage::Asl, SourceLanguage::CSharp];

const LET_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "const",
        replacement: ForeignSpellingReplacement::Text("let"),
        message: "SplitScript uses `let` instead of `const` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `const` with `let`",
    },
    ForeignSpelling {
        source: SourceLanguage::CSharp,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "var",
        replacement: ForeignSpellingReplacement::Text("let"),
        message: "SplitScript uses `let` instead of `var` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `var` with `let`",
    },
    ForeignSpelling {
        source: SourceLanguage::Rust,
        context: ForeignSpellingContext::VariableModifier,
        spelling: "mut",
        replacement: ForeignSpellingReplacement::Remove,
        message: "SplitScript `let` bindings are already mutable",
        primary_label: "remove this Rust-only binding modifier",
        fix_title: "remove `mut`",
    },
];

const NONE_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::JavaScript,
    context: ForeignSpellingContext::OptionalValue,
    spelling: "null",
    replacement: ForeignSpellingReplacement::Text("None"),
    message: "SplitScript uses `None` instead of `null` for absent optional values",
    primary_label: "replace this JavaScript-style value",
    fix_title: "replace `null` with `None`",
}];

const FN_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::Rust,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "func",
        replacement: ForeignSpellingReplacement::Text("fn"),
        message: "SplitScript uses `fn` instead of `func` for functions",
        primary_label: "replace this familiar function keyword",
        fix_title: "replace `func` with `fn`",
    },
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "function",
        replacement: ForeignSpellingReplacement::Text("fn"),
        message: "SplitScript uses `fn` instead of `function` for functions",
        primary_label: "replace this familiar function keyword",
        fix_title: "replace `function` with `fn`",
    },
];

macro_rules! type_spelling {
    ($source:expr, $context:expr, $foreign:literal, $canonical:literal, $message:literal, $label:literal) => {
        ForeignSpelling {
            source: $source,
            context: $context,
            spelling: $foreign,
            replacement: ForeignSpellingReplacement::Text($canonical),
            message: $message,
            primary_label: $label,
            fix_title: concat!("replace `", $foreign, "` with `", $canonical, "`"),
        }
    };
}

const STRING_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "string",
        "String",
        "SplitScript uses `String` instead of `string` for the string type",
        "type names are case-sensitive"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "string",
        "String",
        "SplitScript uses `String` instead of `string` for the string type",
        "type names are case-sensitive"
    ),
];

const STRING_ASCII_LOWER_SPELLINGS: &[ForeignSpelling] = &[type_spelling!(
    SourceLanguage::CSharp,
    ForeignSpellingContext::Method,
    "ToLower",
    "toAsciiLowerCase",
    "SplitScript makes this conversion's ASCII-only semantics explicit with `toAsciiLowerCase`",
    "replace this culture-sensitive C# method name"
)];

const STRICT_EQUALITY_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::JavaScript,
        ForeignSpellingContext::Operator,
        "===",
        "==",
        "SplitScript uses typed `==` instead of JavaScript's `===`",
        "replace this JavaScript equality operator"
    ),
    type_spelling!(
        SourceLanguage::JavaScript,
        ForeignSpellingContext::Operator,
        "!==",
        "!=",
        "SplitScript uses typed `!=` instead of JavaScript's `!==`",
        "replace this JavaScript inequality operator"
    ),
];

const DURATION_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "TimeSpan",
        "Duration",
        "SplitScript uses `Duration` instead of `TimeSpan` for timer durations",
        "replace this C# type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "TimeSpan",
        "Duration",
        "SplitScript uses `Duration` instead of `TimeSpan` for timer durations",
        "replace this C# type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::ValuePath,
        "Duration.Zero",
        "Duration.zero()",
        "SplitScript constructs a zero duration with `Duration.zero()`",
        "replace this C# static property"
    ),
];

macro_rules! numeric_spelling {
    ($foreign:literal, $canonical:literal) => {
        ForeignSpelling {
            source: SourceLanguage::CSharp,
            context: ForeignSpellingContext::Type,
            spelling: $foreign,
            replacement: ForeignSpellingReplacement::Text($canonical),
            message: concat!(
                "SplitScript uses `",
                $canonical,
                "` instead of `",
                $foreign,
                "` for this numeric type"
            ),
            primary_label: "replace this C# numeric type name",
            fix_title: concat!("replace `", $foreign, "` with `", $canonical, "`"),
        }
    };
}

const NUMERIC_SPELLINGS: &[ForeignSpelling] = &[
    numeric_spelling!("sbyte", "i8"),
    numeric_spelling!("byte", "u8"),
    numeric_spelling!("short", "i16"),
    numeric_spelling!("ushort", "u16"),
    numeric_spelling!("int", "i32"),
    numeric_spelling!("uint", "u32"),
    numeric_spelling!("long", "i64"),
    numeric_spelling!("ulong", "u64"),
    numeric_spelling!("float", "f32"),
    numeric_spelling!("double", "f64"),
];

const ASL_PROCESS_IDENTITY_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::Asl,
    context: ForeignSpellingContext::AttachedProcessValuePath,
    spelling: "game.ProcessName",
    replacement: ForeignSpellingReplacement::Text("process.name()"),
    message: "ASL `game.ProcessName` is `process.name()` in SplitScript",
    primary_label: "read the exact process candidate that matched during attachment",
    fix_title: "replace `game.ProcessName` with `process.name()`",
}];

pub const CONCEPTS: &[MigrationConcept] = &[
    MigrationConcept {
        id: MigrationConceptId::new("declaration.let"),
        name: "Variable declarations",
        sources: &[
            SourceLanguage::CSharp,
            SourceLanguage::JavaScript,
            SourceLanguage::Rust,
        ],
        support: MigrationSupport::Direct,
        summary: "Use one inferred `let` declaration; SplitScript has no const/let split.",
        targets: &[MigrationTarget::Language("let")],
        cookbook_anchor: None,
        spellings: LET_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("value.none"),
        name: "Absent optional values",
        sources: JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "`None` is SplitScript's zero-sized unit value and the absent side of an option.",
        targets: &[MigrationTarget::Language("None")],
        cookbook_anchor: None,
        spellings: NONE_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("declaration.function"),
        name: "Function declarations",
        sources: CSHARP_JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "Functions and methods use the `fn` declaration keyword.",
        targets: &[MigrationTarget::Language("fn")],
        cookbook_anchor: None,
        spellings: FN_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.string"),
        name: "String type",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "The immutable UTF-8 string type is named `String`.",
        targets: &[MigrationTarget::StandardLibraryType("String")],
        cookbook_anchor: None,
        spellings: STRING_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.ascii-lowercase"),
        name: "ASCII string lowercasing",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use `toAsciiLowerCase` when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode lowercasing.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "String.toAsciiLowerCase",
        )],
        cookbook_anchor: Some("c-string-operations"),
        spellings: STRING_ASCII_LOWER_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.equality"),
        name: "String equality",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use `==` or `!=` for exact string content equality; use `equalsIgnoreAsciiCase` only when ASCII-insensitive matching is intended.",
        targets: &[
            MigrationTarget::StandardLibraryType("String"),
            MigrationTarget::StandardLibraryItem("String.equalsIgnoreAsciiCase"),
        ],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("operator.strict-equality"),
        name: "Strict equality operators",
        sources: JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "Use typed `==` and `!=`; SplitScript has no coercing equality operators, so JavaScript's extra `=` is unnecessary.",
        targets: &[
            MigrationTarget::Language("=="),
            MigrationTarget::Language("!="),
        ],
        cookbook_anchor: None,
        spellings: STRICT_EQUALITY_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.substring"),
        name: "Substring extraction",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use fallible `slice(start, exclusiveEnd)` only after translating C#'s length argument and verifying that UTF-16 source positions are valid UTF-8 byte offsets.",
        targets: &[MigrationTarget::StandardLibraryItem("String.slice")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.numeric-parse"),
        name: "Numeric string parsing",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Replace static Parse/TryParse calls and output parameters with fallible `text.parse()` and ordinary Result handling.",
        targets: &[MigrationTarget::StandardLibraryItem("String.parse")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.duration"),
        name: "Timer durations",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use `Duration` instead of C#'s `TimeSpan`.",
        targets: &[MigrationTarget::StandardLibraryType("Duration")],
        cookbook_anchor: None,
        spellings: DURATION_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("duration.parse"),
        name: "Text duration parsing",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Replace `TimeSpan.Parse` according to whether the input is fixed data or an already-typed timer value; do not preserve culture-sensitive parsing by default.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Duration.fromWholeSeconds"),
            MigrationTarget::StandardLibraryItem("Duration.fromWholeMilliseconds"),
            MigrationTarget::StandardLibraryItem("Duration.fromParts"),
        ],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("duration.csharp-ticks"),
        name: "C# duration ticks",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Convert 100-nanosecond C# ticks to a source unit explicitly, with range review; SplitScript does not expose C# ticks as a native duration unit.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "Duration.fromNanoseconds",
        )],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.time.monotonic-delay"),
        name: "Monotonic delays and debouncing",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Replace elapsed-time uses of `DateTime.Now` or `Stopwatch` with an `Instant` captured at the source event and an exact `Duration` comparison.",
        targets: &[
            MigrationTarget::StandardLibraryType("Instant"),
            MigrationTarget::StandardLibraryType("Duration"),
        ],
        cookbook_anchor: Some("monotonic-delays-and-debouncing"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.fixed-width-number"),
        name: "Fixed-width numeric types",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Memory-facing numbers use explicit signedness and bit widths.",
        targets: &[
            MigrationTarget::Language("i8"),
            MigrationTarget::Language("u8"),
            MigrationTarget::Language("i16"),
            MigrationTarget::Language("u16"),
            MigrationTarget::Language("i32"),
            MigrationTarget::Language("u32"),
            MigrationTarget::Language("i64"),
            MigrationTarget::Language("u64"),
            MigrationTarget::Language("f32"),
            MigrationTarget::Language("f64"),
        ],
        cookbook_anchor: None,
        spellings: NUMERIC_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.string-n"),
        name: "Bounded native stringN state",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Choose the native encoding explicitly: `utf8` bounds bytes and `utf16le` bounds two-byte code units.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("bounded-native-stringn-state"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.version-label"),
        name: "Version-labelled state blocks",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use named layouts in one state block and return the selected layout from `onAttach`.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("version-labelled-asl-states"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.memory.deep-pointer"),
        name: "DeepPointer",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use typed state paths for polled fields or `process.follow` for discovered paths.",
        targets: &[
            MigrationTarget::Language("state"),
            MigrationTarget::StandardLibraryItem("Process.follow"),
        ],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.memory.background-scan"),
        name: "Background signature scans",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Remove legacy worker threads and await a module, explicit-range, or process-wide scan. Scans inspect a bounded window per tick and process closure cancels pending discovery.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Module.scan"),
            MigrationTarget::StandardLibraryItem("Module.scanAny"),
            MigrationTarget::StandardLibraryItem("Process.scan"),
            MigrationTarget::StandardLibraryItem("Process.scanMemory"),
            MigrationTarget::StandardLibraryItem("Process.scanMemoryAny"),
        ],
        cookbook_anchor: Some("background-signature-scans"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.process.identity"),
        name: "Attached process identity",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use `process.name()` to read the exact process candidate that matched during attachment; use module metadata when the executable name alone does not identify a build.",
        targets: &[MigrationTarget::StandardLibraryItem("Process.name")],
        cookbook_anchor: Some("attached-process-identity"),
        spellings: ASL_PROCESS_IDENTITY_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.memory-watcher"),
        name: "MemoryWatcher",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Declare polled memory in `state`; use a trailing field `if` with `value` and return `Err(message)` when a transient candidate should retain its last accepted value.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("retaining-the-last-accepted-field-value"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.startup"),
        name: "startup lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use settings and global declarations for data, then `setup` for remaining process-independent startup statements.",
        targets: &[MigrationTarget::Language("setup")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.init"),
        name: "init lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use `onAttach` for pre-poll process discovery; legacy post-refresh snapshot work needs a guarded first attached tick.",
        targets: &[MigrationTarget::Language("onAttach")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.update"),
        name: "update lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use `whileAttached` for ordinary post-refresh work; ASL's false control result has no exact equivalent yet.",
        targets: &[MigrationTarget::Language("whileAttached")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.exit"),
        name: "exit lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use guarded `onDetached` cleanup because it also runs before the first attachment.",
        targets: &[MigrationTarget::Language("onDetached")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.shutdown"),
        name: "shutdown lifecycle block",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Exact script teardown needs the planned host shutdown notification; `onDetached` is not equivalent.",
        targets: &[],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.events"),
        name: "timer event handlers",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Simple start transitions can be reconstructed in `whileAttached`; exact ordered start, split, and reset events need host support.",
        targets: &[],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.current-split-index"),
        name: "Current timer split index",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Call `timer.currentSplitIndex()` and handle its optional `u64` result so the host's negative no-attempt sentinel cannot become a route index.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "timer.currentSplitIndex",
        )],
        cookbook_anchor: Some("timer-split-index"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.current-real-time"),
        name: "LiveSplit current real time",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Use `Instant` only for independent elapsed-time checks; exact `timer.CurrentTime.RealTime` metadata requires additional host support.",
        targets: &[],
        cookbook_anchor: Some("monotonic-delays-and-debouncing"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.dynamic-lookup"),
        name: "Dynamic settings lookup",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Declare an exact string key with `key \"...\"`, then use `settings.enabled(key)` or `oldSettings.enabled(key)` for boolean settings. Choice and file settings remain statically typed.",
        targets: &[
            MigrationTarget::Language("settings"),
            MigrationTarget::Language("oldSettings"),
        ],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.registration"),
        name: "Runtime settings registration",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Move `settings.Add` calls into the static `settings` declaration; use a compile-time family for a bounded integer range instead of hand-expanding it.",
        targets: &[MigrationTarget::Language("settings")],
        cookbook_anchor: Some("finite-settings-families"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.finite-family"),
        name: "Finite startup-generated settings",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use a compile-time settings family for bounded integer-keyed booleans; it lowers to ordinary declarations and remains available through `settings.enabled(key)`.",
        targets: &[MigrationTarget::Language("settings family")],
        cookbook_anchor: Some("finite-settings-families"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.collection.list"),
        name: "List<T> collections",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `[T]` with `contains` or `indexOf` for fixed ordered tables, and `Set<T>` for growable unique membership. Ordered growable lists with duplicates remain a distinct planned collection.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Array.contains"),
            MigrationTarget::StandardLibraryItem("Array.indexOf"),
            MigrationTarget::StandardLibraryItem("Set.new"),
            MigrationTarget::StandardLibraryItem("Set.insert"),
            MigrationTarget::StandardLibraryItem("Set.contains"),
            MigrationTarget::StandardLibraryItem("Set.clear"),
        ],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.mutable-current"),
        name: "Assignments to current",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Keep snapshots immutable. Define derived values in the state declaration; when a transient candidate should retain its last accepted value, use a trailing field `if` and return `Err(message)`.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("retaining-the-last-accepted-field-value"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.runtime.refresh-rate"),
        name: "refreshRate",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Call `setTickRate` on the lifecycle transitions where the polling rate changes.",
        targets: &[MigrationTarget::StandardLibraryItem("setTickRate")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.gba"),
        name: "GBA emulator attachment",
        sources: &[SourceLanguage::Rust],
        support: MigrationSupport::Direct,
        summary: "Use `state GBA`; the `gba` root reads normalized emulated addresses.",
        targets: &[MigrationTarget::StateProvider("GBA")],
        cookbook_anchor: None,
        spellings: &[],
    },
];

pub fn concept(id: MigrationConceptId) -> Option<&'static MigrationConcept> {
    CONCEPTS.iter().find(|concept| concept.id == id)
}

pub fn diagnostic(id: MigrationDiagnosticId) -> Option<&'static MigrationDiagnostic> {
    DIAGNOSTICS.iter().find(|diagnostic| diagnostic.id == id)
}

pub fn foreign_spelling(
    spelling: &str,
    context: ForeignSpellingContext,
) -> Option<&'static ForeignSpelling> {
    CONCEPTS
        .iter()
        .flat_map(|concept| concept.spellings)
        .find(|candidate| candidate.spelling == spelling && candidate.context == context)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn concept_and_spelling_identities_are_unique() {
        let mut ids = HashSet::new();
        let mut diagnostic_ids = HashSet::new();
        let mut spellings = HashSet::new();
        for concept in CONCEPTS {
            assert!(
                ids.insert(concept.id),
                "duplicate ID {}",
                concept.id.as_str()
            );
            assert!(!concept.name.trim().is_empty());
            assert!(!concept.sources.is_empty());
            assert!(!concept.summary.trim().is_empty());
            for spelling in concept.spellings {
                assert!(
                    spellings.insert((spelling.context, spelling.spelling)),
                    "duplicate spelling {}",
                    spelling.spelling
                );
                if let ForeignSpellingReplacement::Text(replacement) = spelling.replacement {
                    assert!(!replacement.is_empty());
                }
            }
        }
        for diagnostic in DIAGNOSTICS {
            assert!(
                diagnostic_ids.insert(diagnostic.id),
                "duplicate diagnostic ID {}",
                diagnostic.id.as_str()
            );
            assert!(
                concept(diagnostic.concept).is_some(),
                "diagnostic {} references an unknown concept",
                diagnostic.id.as_str()
            );
            assert!(!diagnostic.message.trim().is_empty());
            assert!(!diagnostic.primary_label.trim().is_empty());
        }
    }
}
