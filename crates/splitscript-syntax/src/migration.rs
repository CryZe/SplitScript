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
    OptionalValue,
    FunctionDeclaration,
    Type,
    StaticTypeReceiver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignSpelling {
    pub source: SourceLanguage,
    pub context: ForeignSpellingContext,
    pub spelling: &'static str,
    pub replacement: &'static str,
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

pub const DIAGNOSTICS: &[MigrationDiagnostic] = &[
    MigrationDiagnostic {
        id: ASL_STRING_N_FIELD_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.string-n"),
        message: "ASL `stringN` fields need an explicit SplitScript memory decoder",
        primary_label: "this ASL pseudo-type combines a byte bound with automatic string decoding",
        notes: &[
            "`as utf8(N)` is appropriate only when the target bytes are UTF-8; ASL `stringN` auto-detects UTF-16 from the second byte and replacement-decodes malformed input",
            "verify the game's in-memory encoding before accepting the suggested rewrite",
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
];

const ASL: &[SourceLanguage] = &[SourceLanguage::Asl];
const CSHARP: &[SourceLanguage] = &[SourceLanguage::CSharp];
const JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::JavaScript];
const CSHARP_JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::CSharp, SourceLanguage::JavaScript];

const LET_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "const",
        replacement: "let",
        message: "SplitScript uses `let` instead of `const` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `const` with `let`",
    },
    ForeignSpelling {
        source: SourceLanguage::CSharp,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "var",
        replacement: "let",
        message: "SplitScript uses `let` instead of `var` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `var` with `let`",
    },
];

const NONE_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::JavaScript,
    context: ForeignSpellingContext::OptionalValue,
    spelling: "null",
    replacement: "None",
    message: "SplitScript uses `None` instead of `null` for absent optional values",
    primary_label: "replace this JavaScript-style value",
    fix_title: "replace `null` with `None`",
}];

const FN_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::Rust,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "func",
        replacement: "fn",
        message: "SplitScript uses `fn` instead of `func` for functions",
        primary_label: "replace this familiar function keyword",
        fix_title: "replace `func` with `fn`",
    },
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "function",
        replacement: "fn",
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
            replacement: $canonical,
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
];

macro_rules! numeric_spelling {
    ($foreign:literal, $canonical:literal) => {
        ForeignSpelling {
            source: SourceLanguage::CSharp,
            context: ForeignSpellingContext::Type,
            spelling: $foreign,
            replacement: $canonical,
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

pub const CONCEPTS: &[MigrationConcept] = &[
    MigrationConcept {
        id: MigrationConceptId::new("declaration.let"),
        name: "Variable declarations",
        sources: CSHARP_JAVASCRIPT,
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
        summary: "Use an explicitly decoded state path such as `as utf8(50)`; choose the encoding from evidence.",
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
        id: MigrationConceptId::new("asl.state.memory-watcher"),
        name: "MemoryWatcher",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Declare polled memory in `state`; `old` and `current` expose transactional snapshots.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.on-start"),
        name: "timer.OnStart",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Observe the `timer.state()` transition in `whileAttached` and reset run-scoped script state there.",
        targets: &[
            MigrationTarget::Language("whileAttached"),
            MigrationTarget::StandardLibraryItem("timer.state"),
        ],
        cookbook_anchor: Some("run-scoped-one-shot-splits"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.exit"),
        name: "exit game-time cleanup",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use guarded `onDetached` cleanup and `timer.pauseGameTime()`.",
        targets: &[
            MigrationTarget::Language("onDetached"),
            MigrationTarget::StandardLibraryItem("timer.pauseGameTime"),
        ],
        cookbook_anchor: Some("process-exit-game-time-cleanup"),
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
        id: MigrationConceptId::new("asl.state.mutable-current"),
        name: "Assignments to current",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Snapshots stay immutable; a typed retain-last-valid normalization pattern is planned.",
        targets: &[MigrationTarget::Language("current")],
        cookbook_anchor: None,
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
                assert!(!spelling.replacement.is_empty());
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
