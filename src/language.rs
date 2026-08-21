//! Canonical catalog for language syntax and domain-specific constructs.
//!
//! These items are deliberately separate from [`crate::stdlib`]: `retry`,
//! lifecycle actions, and the settings DSL are syntax, not fake functions.
//! Documentation generators and editor tooling can still consume the same
//! metadata model through [`crate::catalog`].

use std::collections::HashSet;

use crate::{
    ast::ActionKind,
    catalog::{Documentation, Example},
    types::BuiltinType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageItemKind {
    Keyword,
    Declaration,
    Syntax,
    BuiltinType(BuiltinType),
    SnapshotRoot,
    Action(ActionKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageItem {
    pub id: LanguageItemId,
    pub name: &'static str,
    pub kind: LanguageItemKind,
    /// Compact source shape suitable for completion details and reference docs.
    pub form: &'static str,
    pub documentation: Documentation<LanguageItemId>,
}

const DECLARATIONS_SOURCE: &str = r#"state "game.exe" {}

record Position {
    x: f32,
    y: f32,
}

enum Mode {
    Menu,
    Playing,
}

fn modeName(mode: Mode) {
    return match mode {
        Mode.Menu => "Menu",
        Mode.Playing => "Playing"
    }
}

fn executableKind(name: String) {
    return match name {
        "game.exe" => "full game",
        "game-demo.exe" => "demo",
        _ => "unsupported",
    }
}

fn Position.isOrigin() {
    return self.x == 0.0 && self.y == 0.0
}

whileAttached {
    let origin = Position { x: 0.0, y: 0.0 }
    if origin.isOrigin() {
        print(modeName(Mode.Playing))
    }
}"#;

const STATE_SOURCE: &str = r#"state "game.exe" {
    score = process.read<i32>(0x1000);
}

split {
    return current.score > old.score
}"#;

const FALLIBLE_STATE_SOURCE: &str = r#"state "game.exe" {
    score: i32 = process.read(process.follow(0x1000, [0x20])?);
}"#;

const ALTERNATE_PROCESS_STATE_SOURCE: &str = r#"state ["game.exe", "game-demo.exe"] {
    score: i32 at 0x1000;
}

split {
    return current.score > old.score
}"#;

const MULTI_LAYOUT_STATE_SOURCE: &str = r#"state "game.exe" {
    layout Steam {
        level: u32 at 0x1000;
        checkpoint: u8 at 0x1100;
    },

    layout GOG {
        level: u32 at 0x2000;
        checkpoint: u16 at 0x2100;
    },
}

onAttach {
    let module = await process.mainModule()
    if module.size == 10_000 {
        return StateLayout.Steam
    }
    if module.size == 20_000 {
        return StateLayout.GOG
    }
    await process.closed()
}

whileAttached {
    setVariable("Level", current.level)
    setVariable("Checkpoint", match layout {
        StateLayout.Steam => current.checkpoint as u16,
        StateLayout.GOG => current.checkpoint,
    })
}"#;

const GBA_STATE_SOURCE: &str = r#"state GBA {
    room: u8 at 0x03000010;
}"#;

const TICK_RATE_SOURCE: &str = r#"state "game.exe" {}

tickRate {
    attached: 60,
    detached: 2,
}
"#;

const POINTER_STATE_SOURCE: &str = r#"state "game.exe" {
    score: i32 at 0x1000;
}

split {
    return current.score > old.score
}"#;

const NATIVE_STRING_STATE_SOURCE: &str = r#"state "game.exe" {
    mapName at "game.dll", 0x1234, 0x20 as utf8(64);
}

whileAttached {
    print(current.mapName)
}"#;

const NATIVE_UTF16LE_STATE_SOURCE: &str = r#"state "game.exe" {
    chapterName at "game.dll", 0x2345, 0x18 as utf16le(64);
}

whileAttached {
    print(current.chapterName)
}"#;

const STATE_LAYOUT_SOURCE: &str = include_str!("../examples/state_layouts.split");

const SETTINGS_SOURCE: &str = include_str!("../examples/lso_desktop_settings.split");

const SETTINGS_FAMILY_SOURCE: &str = r#"state "game.exe" {}

settings {
    "Levels" {
        /// Controls the corresponding level split.
        for level in 2..=36 {
            `Level {level}` key `{level}`: true,
        },
    },
}

split {
    return settings.enabled(currentLevel as String)
}

let currentLevel = 2
"#;

const DOCUMENTATION_COMMENT_SOURCE: &str = r#"state "game.exe" {}

/// Formats a level number for display.
fn levelLabel(level) {
    return `Level {level}`
}

settings {
    /// Splits when the boss is defeated.
    "Split boss" => splitBoss: true
}
"#;

const CONTROL_FLOW_SOURCE: &str = r#"state "game.exe" {}

enum Mode {
    Menu,
    Playing,
}

fn modeName(mode: Mode) {
    return match mode {
        Mode.Menu => "Menu",
        Mode.Playing => "Playing"
    }
}

whileAttached {
    let mode = if timer.state() == TimerState.Running {
        Mode.Playing
    } else {
        Mode.Menu
    }
    if mode == Mode.Playing {
        print(modeName(mode))
    }
    debug print("development trace")

    let repetitions = 0
    while repetitions < 2 {
        repetitions += 1
        if repetitions == 1 {
            continue
        }
        break
    }

    for label in ["one", "two"] {
        print(label)
    }
}"#;

const LOOP_SOURCE: &str = r#"state "game.exe" {}

fn chooseModule(useVulkan: bool) -> String {
    return loop {
        if useVulkan {
            break "EngineWin64sv.dll"
        }
        break "EngineWin64s.dll"
    }
}

fn waitForever() -> async Never {
    loop {
        await nextTick()
    }
}

setup {
    print(chooseModule(false))
}"#;

const FOR_SOURCE: &str = r#"state "game.exe" {}

settings {
/// Controls each generated level split.
for level in 2..=36 {
    `Level {level}` key `{level}`: true,
}
}

whileAttached {
for label in ["one", "two"] {
    print(label)
}
}"#;

const IF_SOURCE: &str = r#"state "game.exe" {}

fn levelKind(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}

whileAttached {
if timer.state() == TimerState.Running {
    print("Timer is running")
}
}"#;

const VALUE_BLOCK_SOURCE: &str = r#"state "game.exe" {}

fn levelKind(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}

setup {
    print({
        let level = 7
        `Level {level}`
    })
}"#;

const EQUALITY_SOURCE: &str = r#"state "game.exe" {}

fn same(left: i32, right: i32) -> bool {
    return left == right
}

fn different(left: i32, right: i32) -> bool {
    return left != right
}
"#;

const FAILURE_SOURCE: &str = r#"state "game.exe" {}

fn readOrZero() {
    return process.read<i32>(0x1000) else 0
}

fn forwarded() -> i32! {
    return process.read<i32>(0x1000)?
}

fn unavailable() -> i32! {
    throw "not available"
}

fn explicitError() -> i32! {
    return Err("not available")
}

fn engineModule() {
    return retry {
        let module = process.loadedModule("EngineWin64s.dll")
            else process.loadedModule("EngineWin64sv.dll")
            else throw "engine module is not loaded yet"
        module
    }
}

whileAttached {
    print(readOrZero() as String)
}"#;

const ASYNC_SOURCE: &str = r#"state "game.exe" {}

fn loadGameAssembly() -> async Module {
    let module = await process.module("GameAssembly.dll")
    return module
}

fn readMarker() {
    return process.read<i32>(0x3000)
}

onAttach {
    let offsets: [i64; 2] = [0x100, 0x20]
    let module = await process.module("GameAssembly.dll")
    let marker = retry readMarker()
    let health = retry {
        let player = process.follow(module.address, offsets)?
        process.read<i32>(player)?
    }
    print(`ready {module.address}:{marker}:{health}`)
}"#;

// Catalog examples are compiled all the way through Wasm. The declaration is
// deliberately unreachable until typed source-function frame emission lands;
// parsing, checking, hover, and inlay inference still validate its async
// signature and body here.
const ASYNC_RESULT_SOURCE: &str = r#"state "game.exe" {}

fn loadGameAssembly() -> async Module {
    let module = await process.module("GameAssembly.dll")
    return module
}

onAttach {
    let module = await process.module("GameAssembly.dll")
    print(module.address)
}"#;

const TYPES_AND_LITERALS_SOURCE: &str = r#"state "game.exe" {}

fn maybe(value) -> i32? {
    if value > 0 {
        return value
    }
    return None
}

fn result() -> i32! {
    return 42
}

fn ignoreUnsupportedBuild() -> async Never {
    await process.closed()
}

onAttach {
    let module = await process.module("GameAssembly.dll")
    let marker = await module.scan(sig"48 8B ?? B?")
    let supportedVersion = v"1.2.3.4"
    let optionalMarker = Some(marker)
    let successfulValue = Ok(result() else 0)
    let text = (successfulValue else 0) as String
    print(`marker {optionalMarker else 0 as address}, value {text}`)
}"#;

const LIFECYCLE_SOURCE: &str = r#"state "game.exe" {}

setup {}
onDetach {}
onAttach {}
whileAttached {}
start {}
split {}
reset {}
isLoading {}
gameTime {}
"#;

macro_rules! focused_example {
    ($name:ident, $title:literal, $source:literal, $validation:expr) => {
        const $name: &[Example] = &[Example::checked($title, $source, $validation)];
    };
}

focused_example!(
    LET_EXAMPLE,
    "Infer a local type",
    "let retryDelay = 30",
    DECLARATIONS_SOURCE
);
focused_example!(
    FUNCTION_EXAMPLE,
    "Declare a helper",
    "fn isBoss(level) {\n    return level == 7\n}",
    DECLARATIONS_SOURCE
);
focused_example!(
    RECORD_EXAMPLE,
    "Group related values",
    "record Position {\n    x: f32,\n    y: f32,\n}",
    DECLARATIONS_SOURCE
);
focused_example!(
    ENUM_EXAMPLE,
    "Describe distinct states",
    "enum Mode {\n    Menu,\n    Playing,\n}",
    DECLARATIONS_SOURCE
);
const STATE_DECL_EXAMPLES: &[Example] = &[
    Example::checked(
        "Read state from a native process",
        "state \"game.exe\" {\n    score = process.read<i32>(0x1000);\n}",
        STATE_SOURCE,
    ),
    Example::checked(
        "Compose fallible address discovery with a read",
        FALLIBLE_STATE_SOURCE,
        FALLIBLE_STATE_SOURCE,
    ),
    Example::checked(
        "Try alternate executable names",
        "state [\"game.exe\", \"game-demo.exe\"] {\n    score: i32 at 0x1000;\n}",
        ALTERNATE_PROCESS_STATE_SOURCE,
    ),
    Example::checked(
        "Support multiple game builds",
        MULTI_LAYOUT_STATE_SOURCE,
        MULTI_LAYOUT_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a GBA emulator",
        "state GBA {\n    room: u8 at 0x03000010;\n}",
        GBA_STATE_SOURCE,
    ),
];
const STATE_LAYOUT_EXAMPLE: &[Example] = &[Example::checked(
    "Select and refine a supported build",
    STATE_LAYOUT_SOURCE,
    STATE_LAYOUT_SOURCE,
)];
focused_example!(
    STATE_POINTER_EXAMPLE,
    "Read a pointer-backed field",
    "score: i32 at 0x1000",
    POINTER_STATE_SOURCE
);
focused_example!(
    NATIVE_STRING_DECODER_EXAMPLE,
    "Decode a native string field",
    "mapName at 0x1234 as utf8(64)",
    NATIVE_STRING_STATE_SOURCE
);
focused_example!(
    NATIVE_UTF16LE_DECODER_EXAMPLE,
    "Decode a native UTF-16LE field",
    "chapterName at 0x2345 as utf16le(64)",
    NATIVE_UTF16LE_STATE_SOURCE
);
focused_example!(
    SETTINGS_DECL_EXAMPLE,
    "Declare a toggle",
    "settings {\n    \"Split bosses\" => splitBosses: true,\n}",
    SETTINGS_SOURCE
);
focused_example!(
    SETTING_KEY_EXAMPLE,
    "Keep a stable host key",
    "\"Enable Auto Splitting\" => enableAutoSplitting key \"auto-splitting\": true",
    SETTINGS_SOURCE
);
const IF_EXAMPLES: &[Example] = &[
    Example::checked(
        "Run a conditional statement",
        "if timer.state() == TimerState.Running {\n    print(\"Timer is running\")\n}",
        IF_SOURCE,
    ),
    Example::checked(
        "Choose a value",
        "let label = if isBoss { \"Boss\" } else { \"Level\" }",
        IF_SOURCE,
    ),
    Example::checked(
        "Prepare a branch value",
        "let label = if isBoss {\n    let kind = \"Boss\"\n    `{kind} level`\n} else {\n    \"Level\"\n}",
        IF_SOURCE,
    ),
];
const VALUE_BLOCK_EXAMPLES: &[Example] = &[
    Example::checked(
        "Compute a value with local steps",
        "let label = {\n    let level = 7\n    `Level {level}`\n}",
        VALUE_BLOCK_SOURCE,
    ),
    Example::checked(
        "Prepare an argument locally",
        "print({\n    let level = 7\n    `Level {level}`\n})",
        VALUE_BLOCK_SOURCE,
    ),
];
const ELSE_EXAMPLES: &[Example] = &[
    Example::checked(
        "Provide a read fallback",
        "let health = process.read<i32>(healthAddress) else 0",
        FAILURE_SOURCE,
    ),
    Example::checked(
        "Retry either of two module names",
        "let engine = retry {\n    let module = process.loadedModule(\"EngineWin64s.dll\")\n        else process.loadedModule(\"EngineWin64sv.dll\")\n        else throw \"engine module is not loaded yet\"\n    module\n}",
        FAILURE_SOURCE,
    ),
];
focused_example!(
    WHILE_EXAMPLE,
    "Repeat while attached",
    "while index < values.length() {\n    index += 1\n}",
    CONTROL_FLOW_SOURCE
);
const LOOP_EXAMPLES: &[Example] = &[
    Example::checked(
        "Repeat without falling through",
        "loop {\n    await nextTick()\n}",
        LOOP_SOURCE,
    ),
    Example::checked(
        "Break with a value",
        "let moduleName = loop {\n    if useVulkan {\n        break \"EngineWin64sv.dll\"\n    }\n    break \"EngineWin64s.dll\"\n}",
        LOOP_SOURCE,
    ),
];
const FOR_EXAMPLES: &[Example] = &[
    Example::checked(
        "Iterate over an array",
        "for label in [\"one\", \"two\"] {\n    print(label)\n}",
        FOR_SOURCE,
    ),
    Example::checked(
        "Declare a family of settings",
        "/// Controls each generated level split.\nfor level in 2..=36 {\n    `Level {level}` key `{level}`: true,\n}",
        FOR_SOURCE,
    ),
];
const BREAK_EXAMPLES: &[Example] = &[
    Example::checked(
        "Exit a conditional loop",
        "while !found {\n    if cannotContinue { break }\n}",
        CONTROL_FLOW_SOURCE,
    ),
    Example::checked(
        "Produce a loop value",
        "let moduleName = loop {\n    if useVulkan { break \"EngineWin64sv.dll\" }\n    break \"EngineWin64s.dll\"\n}",
        LOOP_SOURCE,
    ),
];
focused_example!(
    CONTINUE_EXAMPLE,
    "Skip an iteration",
    "while index < count {\n    index += 1\n    if index == ignored { continue }\n    inspect(index)\n}",
    CONTROL_FLOW_SOURCE
);
focused_example!(
    DEBUG_EXAMPLE,
    "Add development-only logging",
    "debug print(`level: {current.level}`)",
    CONTROL_FLOW_SOURCE
);
const MATCH_EXAMPLES: &[Example] = &[
    Example::checked(
        "Handle every enum variant",
        "let label = match mode {\n    Mode.Menu => \"Menu\",\n    Mode.Playing => \"Playing\"\n}",
        CONTROL_FLOW_SOURCE,
    ),
    Example::checked(
        "Dispatch on exact string contents",
        "return match name {\n    \"game.exe\" => \"full game\",\n    \"game-demo.exe\" => \"demo\",\n    _ => \"unsupported\",\n}",
        CONTROL_FLOW_SOURCE,
    ),
];
focused_example!(
    RETURN_EXAMPLE,
    "Return a value",
    "fn double(value) {\n    return value * 2\n}",
    FAILURE_SOURCE
);
focused_example!(
    THROW_EXAMPLE,
    "Return an error",
    "fn requireAddress(value) -> address! {\n    if value == 0 { throw \"address is null\" }\n    return value\n}",
    FAILURE_SOURCE
);
focused_example!(
    ASYNC_RESULT_EXAMPLE,
    "Declare an asynchronous helper",
    "fn loadGameAssembly() -> async Module {\n    let module = await process.module(\"GameAssembly.dll\")\n    return module\n}",
    ASYNC_RESULT_SOURCE
);
focused_example!(
    AWAIT_EXAMPLE,
    "Wait during attachment",
    "let module = await process.module(\"GameAssembly.dll\")",
    ASYNC_SOURCE
);
const RETRY_EXAMPLES: &[Example] = &[
    Example::checked(
        "Retry one fallible expression",
        "let marker = retry readMarker()",
        ASYNC_SOURCE,
    ),
    Example::checked(
        "Retry a complete fallible block",
        "let health = retry {\n    let player = process.follow(module.address, offsets)?\n    process.read<i32>(player)?\n}",
        ASYNC_SOURCE,
    ),
];
focused_example!(
    PROPAGATE_EXAMPLE,
    "Forward a read error",
    "let health = process.read<i32>(healthAddress)?",
    FAILURE_SOURCE
);
focused_example!(
    CAST_EXAMPLE,
    "Convert an integer",
    "let label = level as String",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    SOME_EXAMPLE,
    "Construct an optional value",
    "let selected: i32? = Some(7)",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    OK_EXAMPLE,
    "Construct a successful result",
    "let health: i32! = Ok(100)",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ERR_EXAMPLE,
    "Construct an error",
    "let health: i32! = Err(\"health is unavailable\")",
    FAILURE_SOURCE
);
focused_example!(
    SELF_EXAMPLE,
    "Use the method receiver",
    "fn Position.isOrigin() {\n    return self.x == 0.0 && self.y == 0.0\n}",
    DECLARATIONS_SOURCE
);
focused_example!(
    SIGNATURE_EXAMPLE,
    "Match machine code",
    "let marker = await module.scan(sig\"48 8B ?? B?\")",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    VERSION_EXAMPLE,
    "Match a Windows file version",
    "if version == v\"1.2.3.4\" { print(\"supported build\") }",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    TEMPLATE_EXAMPLE,
    "Interpolate a value",
    "let label = `Level {current.level}`",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ARRAY_TYPE_EXAMPLE,
    "Annotate an exact-length array",
    "let offsets: [i64; 2] = [0x100, 0x20]",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ARRAY_INDEX_EXAMPLE,
    "Read an array element",
    "let opcode = bytes[1]",
    "state \"game.exe\" {}\nwhileAttached {\n    let bytes: [u8; 3] = [0x48, 0x8b, 0x01]\n    let opcode = bytes[1]\n    print(opcode)\n}"
);
focused_example!(
    OPTION_TYPE_EXAMPLE,
    "Annotate an optional value",
    "let selectedLevel: i32? = None",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    RESULT_TYPE_EXAMPLE,
    "Return a fallible value",
    "fn readHealth() -> i32! {\n    return process.read<i32>(healthAddress)?\n}",
    TYPES_AND_LITERALS_SOURCE
);
const DOCUMENTATION_COMMENT_EXAMPLES: &[Example] = &[
    Example::checked(
        "Document a source symbol",
        "/// Formats a level number for display.\nfn levelLabel(level) {\n    return `Level {level}`\n}",
        DOCUMENTATION_COMMENT_SOURCE,
    ),
    Example::checked(
        "Add a setting tooltip",
        "/// Splits when the boss is defeated.\n\"Split boss\" => splitBoss: true",
        DOCUMENTATION_COMMENT_SOURCE,
    ),
];
focused_example!(
    CHOICE_EXAMPLE,
    "Choose an enum value",
    "\"Character\" => character: choice {\n    \"Hana\" => Character.Hana default\n    \"Toree\" => Character.Toree\n}",
    SETTINGS_SOURCE
);
focused_example!(
    FILE_EXAMPLE,
    "Choose a file",
    "\"Layout\" => layout: file {\n    \"Layout files\" => \"*.lsl\"\n}",
    SETTINGS_SOURCE
);
focused_example!(
    SETTINGS_FAMILY_EXAMPLE,
    "Declare numbered level settings",
    "for level in 2..=36 {\n    `Level {level}` key `{level}`: true,\n}",
    SETTINGS_FAMILY_SOURCE
);
focused_example!(
    EQUALITY_EXAMPLE,
    "Compare equal values",
    "left == right",
    EQUALITY_SOURCE
);
focused_example!(
    INEQUALITY_EXAMPLE,
    "Compare different values",
    "left != right",
    EQUALITY_SOURCE
);

macro_rules! language_item {
    ($id:ident, $name:expr, $kind:expr, $form:expr, $summary:expr, $details:expr, $examples:expr) => {
        LanguageItem {
            id: LanguageItemId::$id,
            name: $name,
            kind: $kind,
            form: $form,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: $examples,
                related: &[],
            },
        }
    };
}

macro_rules! action_item {
    ($id:ident, $action:ident, $name:literal, $summary:literal, $details:literal,
     $example:literal) => {
        language_item!(
            $id,
            $name,
            LanguageItemKind::Action(ActionKind::$action),
            concat!($name, " { ... }"),
            $summary,
            $details,
            &[Example::checked(
                concat!("Use ", $name),
                $example,
                LIFECYCLE_SOURCE,
            )]
        )
    };
}

macro_rules! builtin_type_item {
    ($ty:ident, $name:literal, $summary:literal, $details:literal, $example:literal) => {
        LanguageItem {
            id: LanguageItemId::BuiltinType(BuiltinType::$ty),
            name: $name,
            kind: LanguageItemKind::BuiltinType(BuiltinType::$ty),
            form: $name,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    concat!("Use ", $name),
                    $example,
                    TYPES_AND_LITERALS_SOURCE,
                )],
                related: &[],
            },
        }
    };
}

macro_rules! compiler_symbol_item {
    ($id:expr, $name:literal, $kind:expr, $form:literal, $summary:literal, $details:literal,
     $example:literal, $validation:expr) => {
        LanguageItem {
            id: $id,
            name: $name,
            kind: $kind,
            form: $form,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    concat!("Use ", $name),
                    $example,
                    $validation,
                )],
                related: &[],
            },
        }
    };
}

macro_rules! define_language_catalog {
    (
        ordinary_before { $(language_item!($before_id:ident, $($before:tt)*)),* $(,)? }
        builtins { $(builtin_type_item!($builtin_id:ident, $($builtin:tt)*)),* $(,)? }
        compiler_symbols { $(compiler_symbol_item!(LanguageItemId::$compiler_id:ident, $($compiler:tt)*)),* $(,)? }
        ordinary_after { $(language_item!($after_id:ident, $($after:tt)*)),* $(,)? }
        actions { $(action_item!($action_id:ident, $($action:tt)*)),* $(,)? }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum LanguageItemId {
            $($before_id,)*
            BuiltinType(BuiltinType),
            $($compiler_id,)*
            $($after_id,)*
            $($action_id),*
        }

        const ITEMS: &[LanguageItem] = &[
            $(language_item!($before_id, $($before)*),)*
            $(builtin_type_item!($builtin_id, $($builtin)*),)*
            $(compiler_symbol_item!(LanguageItemId::$compiler_id, $($compiler)*),)*
            $(language_item!($after_id, $($after)*),)*
            $(action_item!($action_id, $($action)*),)*
        ];
    };
}

define_language_catalog! {
    ordinary_before {
    language_item!(
        Let,
        "let",
        LanguageItemKind::Keyword,
        "let name = expression",
        "Declares an inferred variable.",
        "Bindings are mutable and their types are inferred bidirectionally from their initializer and uses.",
        LET_EXAMPLE
    ),
    language_item!(
        Function,
        "fn",
        LanguageItemKind::Declaration,
        "fn name(parameters) { ... }",
        "Declares a function or method.",
        "Parameter and result annotations are optional when constraints from the body and call sites determine them.",
        FUNCTION_EXAMPLE
    ),
    language_item!(
        Record,
        "record",
        LanguageItemKind::Declaration,
        "record Name { field: Type }",
        "Declares an immutable nominal record.",
        "Records provide named fields, structural equality when their fields support it, and fixed process-memory layouts when every field is readable.",
        RECORD_EXAMPLE
    ),
    language_item!(
        Enum,
        "enum",
        LanguageItemKind::Declaration,
        "enum Name { Variant, Payload(Type) }",
        "Declares a nominal sum type.",
        "Enums support optional variant payloads, exhaustive match expressions, and structural equality when their payloads support it.",
        ENUM_EXAMPLE
    ),
    language_item!(
        State,
        "state",
        LanguageItemKind::Declaration,
        "state \"game.exe\" { ... } | state [\"game.exe\", \"demo.exe\"] { ... } | state GBA { ... }",
        "Declares process attachment and persistent watched state.",
        "A native string is an exact host process identity. The current Windows host reports executable filenames including `.exe`, so a Windows candidate must include that extension. An array tries alternate executable names in order; it does not attach to several processes at once. Put build-specific memory shapes in named [`layout`] blocks instead of duplicating ASL-style state declarations. Every state expression has one implicit fallible boundary ([`T!`]): internal postfix [`?`] and a fallible final call propagate into that same boundary. Initialization requires all required fields to succeed in one poll and seeds [`old`] and [`current`] equally without running lifecycle actions. Later, failed fields retain their accepted values while successful sibling fields advance. Deliberately optional reads can discard their error into [`T?`] with [`discardError`](method@Result.discardError).",
        STATE_DECL_EXAMPLES
    ),
    language_item!(
        TickRate,
        "tickRate",
        LanguageItemKind::Declaration,
        "tickRate { attached: 60, detached: 2 }",
        "Overrides the lifecycle-owned polling rates.",
        "SplitScript defaults to 120 Hz while a process is attached and 1 Hz while detached. The attached rate is applied immediately after acquiring a process, before [`onAttach`] and its cooperative discovery run. The detached rate is applied during module startup and immediately when a process closes. Either field may be omitted to retain its default; [`setTickRate`] remains available for temporary dynamic changes until the next lifecycle transition.",
        &[Example::checked(
            "Override lifecycle polling rates",
            "tickRate {\n    attached: 60,\n    detached: 2,\n}",
            TICK_RATE_SOURCE,
        )]
    ),
    language_item!(
        StateLayout,
        "layout",
        LanguageItemKind::Declaration,
        "layout Name { field at address }",
        "Declares one named memory layout for a supported game build.",
        "Declare all build layouts inside one [`state`] block. Fields with the same compatible type in every named layout form the common state snapshot interface. Missing fields and conflicting same-named types are available after a direct [`match`] on the generated read-only [`layout`] value refines the selected `StateLayout` variant. The implicitly suspending [`onAttach`] block uses module metadata, signatures, or other typed evidence and returns that variant before polling begins; [`await`] [`Process.closed`] represents an unsupported build without falling back or repeatedly reattaching.",
        STATE_LAYOUT_EXAMPLE
    ),
    language_item!(
        StatePointerField,
        "state pointer field",
        LanguageItemKind::Syntax,
        "field: T at module?, offset, ... | field: T? at module?, offset, ...",
        "Reads a persistent state field through a pointer path.",
        "The optional module name selects the pointer base and each following integer is an address offset. A required `T` field is a [`T!`] boundary: initialization waits for it, while a later failed read retains its last accepted value. An explicitly optional [`T?`] field instead accepts read failure as [`None`] and a successful read as [`Some`]`(T)`, so absence is observable in [`current`] and [`old`]. The exact memory representation must be explicit or inferred from an exact use; optional read semantics require the [`T?`] annotation.",
        STATE_POINTER_EXAMPLE
    ),
    language_item!(
        NativeStringDecoder,
        "utf8",
        LanguageItemKind::Syntax,
        "field at address as utf8(maxBytes)",
        "Decodes a bounded native UTF-8 string state field.",
        "This is state-layout sugar for a bounded read-and-decode operation, not a string-size type. It follows the complete pointer path, reads at most 4096 bytes once, and stops at the first NUL byte. A required field rejects its candidate when memory cannot be read or the bytes are not valid UTF-8; an explicitly annotated optional [`String`] ([`T?`]) field observes that failure as [`None`]. Without the optional annotation, the field type is inferred as [`String`].",
        NATIVE_STRING_DECODER_EXAMPLE
    ),
    language_item!(
        NativeUtf16LeDecoder,
        "utf16le",
        LanguageItemKind::Syntax,
        "field at address as utf16le(maxUtf16Units)",
        "Decodes a bounded native UTF-16LE string state field.",
        "This state-layout sugar follows the complete pointer path, reads at most 2048 little-endian UTF-16 code units once, and stops at the first NUL code unit. Unpaired surrogate code units become the Unicode replacement character. A required field rejects its candidate when memory cannot be read; an explicitly annotated optional [`String`] ([`T?`]) field observes that failure as [`None`]. Without the optional annotation, the field type is inferred as [`String`].",
        NATIVE_UTF16LE_DECODER_EXAMPLE
    ),
    language_item!(
        Settings,
        "settings",
        LanguageItemKind::Declaration,
        "settings { \"Group\" { \"Label\" => name key \"host-key\": value } }",
        "Declares live user settings.",
        "Settings support nested headings, [`///`] tooltips, booleans, [`choice setting`] values, and [`file setting`] selectors. An optional [`stable setting key`] is the exact string stored in the host settings map; otherwise the source identifier is used. [`settings`] and [`oldSettings`] refresh every update.",
        SETTINGS_DECL_EXAMPLE
    ),
    language_item!(
        SettingFamily,
        "settings family",
        LanguageItemKind::Syntax,
        "for value in start..=end { `label {value}` key `{value}`: default }",
        "Declares a finite family of boolean settings at compile time.",
        "The inclusive [`u32`] range is expanded during compilation into ordinary host settings. Label and key templates may interpolate only the loop binding. Generated entries deliberately have no statically named settings member; query them with [`SettingsView.enabled`]. A [`///`] comment on the family becomes every generated setting's tooltip, and nesting it in a quoted group preserves the normal heading structure.",
        SETTINGS_FAMILY_EXAMPLE
    ),
    language_item!(
        StableSettingKey,
        "stable setting key",
        LanguageItemKind::Syntax,
        "\"Label\" => name key \"host-key\": value",
        "Assigns an explicit stable key in the host settings map.",
        "The quoted key is used for persistent host storage and dynamic [`SettingsView.enabled`] lookups. The source identifier remains the statically typed member exposed through [`settings`] and [`oldSettings`]. Without [`key`](syntax@stable setting key), the source identifier is also the host key.",
        SETTING_KEY_EXAMPLE
    ),
    language_item!(
        If,
        "if",
        LanguageItemKind::Keyword,
        "if condition { ... } else { ... }",
        "Branches as a statement or expression.",
        "Expression-valued [`if`] requires an [`else`] branch and infers both branch values against one result type. A branch may be a [`value block`] when it needs local steps before producing its value.",
        IF_EXAMPLES
    ),
    language_item!(
        ValueBlock,
        "value block",
        LanguageItemKind::Syntax,
        "{ statements; finalExpression }",
        "Runs scoped statements and yields its final expression.",
        "A value block may appear anywhere an expression is accepted, including an [`if`] branch, [`match`] arm, fallback [`else`], function argument, or state-field initializer. Its bindings are local to the block. The final expression supplies the block's value; a block without one yields [`None`], unless control always leaves through [`return`], [`break`], [`continue`], [`throw`], or another [`Never`] expression. A semicolon after the final expression is accepted for familiarity but warned about and removed by the formatter because the expression is still the value. Function and lifecycle bodies are statement blocks, not value blocks: use [`return`] explicitly when they return a value.",
        VALUE_BLOCK_EXAMPLES
    ),
    language_item!(
        Equality,
        "==",
        LanguageItemKind::Syntax,
        "left == right",
        "Compares two values for equality.",
        "Equality is typed and never coerces between unrelated representations. [`String`] values compare by text content, while records and enums compare structurally when their contents satisfy [`Equatable`].",
        EQUALITY_EXAMPLE
    ),
    language_item!(
        Inequality,
        "!=",
        LanguageItemKind::Syntax,
        "left != right",
        "Compares two values for inequality.",
        "Inequality is the typed negation of [`==`] and supports the same exact types and structural comparisons through [`Equatable`].",
        INEQUALITY_EXAMPLE
    ),
    language_item!(
        Else,
        "else",
        LanguageItemKind::Keyword,
        "value else fallback",
        "Provides a branch or unwrap fallback.",
        "After [`if`], [`else`] selects the alternate branch. After a [`T?`] or [`T!`] expression, it unwraps success or evaluates its right operand on absence or error. That operand is any ordinary expression; [`return`], [`break`], [`continue`], and [`throw`] work naturally because each has type [`Never`]. Chained fallbacks associate to the right.",
        ELSE_EXAMPLES
    ),
    language_item!(
        While,
        "while",
        LanguageItemKind::Keyword,
        "while condition { ... }",
        "Repeats a block while its condition is true.",
        "The condition must be [`bool`] and is evaluated before every iteration. The loop body has its own lexical scope. In [`onAttach`], [`await`] and [`retry`] resume through explicit loop-header and exit states without replaying completed iterations.",
        WHILE_EXAMPLE
    ),
    language_item!(
        Loop,
        "loop",
        LanguageItemKind::Keyword,
        "loop { ... }",
        "Repeats a block until it breaks.",
        "Without a reachable [`break`], [`loop`] has type [`Never`] and cannot fall through. `break value` exits the nearest [`loop`] expression and supplies its result; all such values are inferred bidirectionally as one type. A bare [`break`] supplies [`None`]. Value-carrying breaks are deliberately unavailable in [`while`] and runtime [`for`] loops. [`continue`] starts the next iteration. In an async context, [`await`] and [`retry`] preserve the loop and its live values across ticks.",
        LOOP_EXAMPLES
    ),
    language_item!(
        For,
        "for",
        LanguageItemKind::Keyword,
        "for value in array { ... } | for value in start..=end { setting }",
        "Iterates over an array or declares a finite settings family.",
        "In runtime code, the array expression is evaluated exactly once. The element binding is read-only, lexically scoped to the body, and inferred from [`[T]`] or [`[T; N]`]. [`break`] and [`continue`] target the nearest loop. In [`onAttach`], a body containing [`await`] or [`retry`] preserves the array, index, and current binding across suspension. Inside [`settings`], the inclusive range form expands a [`settings family`] at compile time instead of creating a runtime loop.",
        FOR_EXAMPLES
    ),
    language_item!(
        Break,
        "break",
        LanguageItemKind::Keyword,
        "break | break value",
        "Exits the nearest enclosing loop.",
        "[`break`] is an ordinary [`Never`]-typed expression which exits the innermost [`while`], runtime [`for`], or [`loop`]. Inside a [`loop`] expression, `break value` also supplies the expression's result; a bare break produces [`None`]. Value-carrying breaks are rejected in [`while`] and [`for`] so they can never accidentally skip an inner statement loop to target an outer value loop. Because it never yields locally, [`break`] can appear in any expression position whose evaluation may exit that loop, including a fallback [`else`].",
        BREAK_EXAMPLES
    ),
    language_item!(
        Continue,
        "continue",
        LanguageItemKind::Keyword,
        "continue",
        "Starts the next iteration of the nearest enclosing loop.",
        "[`continue`] is an ordinary [`Never`]-typed expression. It may appear anywhere an expression is accepted inside a loop, including a fallback [`else`]; evaluating it starts the next iteration instead of yielding a local value. The loop condition is then evaluated again.",
        CONTINUE_EXAMPLE
    ),
    language_item!(
        Debug,
        "debug",
        LanguageItemKind::Keyword,
        "debug statement",
        "Keeps a development-only statement in debug builds.",
        "Debug statements, bindings, globals, and `debug fn` declarations are fully parsed and type-checked in every profile, then erased from release lowering before dependency and reachability discovery. Debug-only names may only be used from debug code. Terminating statements remain rejected.",
        DEBUG_EXAMPLE
    ),
    language_item!(
        Match,
        "match",
        LanguageItemKind::Keyword,
        "match value { pattern => expression }",
        "Exhaustively matches a value.",
        "[`match`] supports enum payloads, optional [`None`]/[`Some`]`(value)` patterns, fallible [`Err`]`(error)`/[`Ok`]`(value)` patterns, string, character, integer, boolean, and file-version literals, guards, and a wildcard. String patterns compare contents, not WebAssembly GC identities. Enum and wrapper matches must cover every state; open-ended literal domains require `_`; guarded arms do not establish coverage.",
        MATCH_EXAMPLES
    ),
    language_item!(
        Return,
        "return",
        LanguageItemKind::Keyword,
        "return expression",
        "Returns from the current function or action.",
        "[`return`] is an ordinary [`Never`]-typed expression, so it may appear anywhere an expression is accepted and satisfies any surrounding expected type by leaving the current function or action instead of yielding locally. Functions infer their result from explicit [`return`] expressions and call-site constraints. Unlike a nested [`value block`], a function or lifecycle body never returns its final expression implicitly. Lifecycle actions apply their domain default when control falls through.",
        RETURN_EXAMPLE
    ),
    language_item!(
        Throw,
        "throw",
        LanguageItemKind::Keyword,
        "throw error",
        "Transfers an error to the nearest failure boundary.",
        "[`throw`] is an ordinary [`Never`]-typed expression. Inside [`retry`], it ends the current attempt and retries the complete operand on the next attached update. Otherwise it returns an error from the enclosing [`T!`] function or rejects the current state-field value. The error expression must be a [`String`]. Because it never yields locally, it can be used directly as a fallback [`else`] operand or in any other expression position.",
        THROW_EXAMPLE
    ),
    language_item!(
        Async,
        "async",
        LanguageItemKind::Keyword,
        "fn name() -> async T { ... }",
        "Marks an explicitly typed function result as asynchronous.",
        "A function containing [`await`] or [`retry`] has an async result. Write [`async`] `T` when its result type is explicit; when the result type is omitted, both [`async`] and `T` are inferred. Calling a source-defined async function creates a process-lifetime future value without polling it. That [`async`] `T` value can be stored in locals and aggregates, passed to functions, and awaited later. Its typed continuation frame retains parameters, live locals, nested futures, and the completed `T`. Futures cannot escape into globals because process closure owns their cancellation.",
        ASYNC_RESULT_EXAMPLE
    ),
    language_item!(
        Await,
        "await",
        LanguageItemKind::Keyword,
        "let value = await operation",
        "Waits for an asynchronous value and yields its result.",
        "[`await`] is an ordinary prefix expression available in [`onAttach`] and source-defined [`async`] helpers. It accepts any [`async`] `T` expression, yields `T`, and can be nested in calls, operators, member access, conditionals, matches, fallbacks, and loop conditions. Source future values may be stored and awaited repeatedly; an already completed future yields its retained result without rerunning its body. The process-lifetime continuation tree is cancelled when the attached process closes.",
        AWAIT_EXAMPLE
    ),
    language_item!(
        Retry,
        "retry",
        LanguageItemKind::Keyword,
        "let value = retry fallibleExpression | let value = retry { ... }",
        "Retries synchronous fallible work until it succeeds.",
        "[`retry`] creates a local error boundary around any ordinary expression. The complete operand is evaluated again once per attached update. A [`T!`] error, postfix [`?`], or [`throw`] ends only the current attempt; a successful final value yields `T`. A value block is not a special retry form: braces are ordinary expressions, so `retry { ... }` naturally retries every statement and its final expression. [`return`], [`break`], and [`continue`] keep their normal lexical targets. The attempt itself must remain synchronous and bounded: evaluating [`await`] or another [`retry`] inside it is rejected, while merely calling an [`async`] function to construct a future is allowed. A containing function infers an [`async`] result unless its result type is explicit, in which case write `-> async T`. Process closure cancels the retry boundary.",
        RETRY_EXAMPLES
    ),
    language_item!(
        Propagate,
        "?",
        LanguageItemKind::Syntax,
        "resultExpression?",
        "Propagates a [`T!`] error.",
        "Postfix [`?`] unwraps success or transfers the original error to the nearest failure boundary: a [`retry`] operand restarts on the next attached update, a state-field assignment rejects that field update, and a [`T!`] function returns the error.",
        PROPAGATE_EXAMPLE
    ),
    language_item!(
        AsCast,
        "as",
        LanguageItemKind::Keyword,
        "expression as Type",
        "Explicitly converts a value.",
        "Casts are checked statically. String interpolation uses the same [`Display`] conversion as an explicit [`as`] [`String`] cast.",
        CAST_EXAMPLE
    ),
    language_item!(
        SomeConstructor,
        "Some",
        LanguageItemKind::Syntax,
        "Some(value)",
        "Explicitly constructs a present optional value.",
        "[`Some`] infers `T` from its value and constructs [`T?`]. Plain `T` values still lift automatically whenever [`T?`] is expected.",
        SOME_EXAMPLE
    ),
    language_item!(
        SuccessConstructor,
        "Ok",
        LanguageItemKind::Syntax,
        "Ok(value)",
        "Explicitly constructs a successful result value.",
        "[`Ok`] infers `T` from its value and constructs [`T!`]. Plain `T` values still lift automatically whenever [`T!`] is expected.",
        OK_EXAMPLE
    ),
    language_item!(
        ErrorConstructor,
        "Err",
        LanguageItemKind::Syntax,
        "Err(message)",
        "Constructs a [`T!`] error.",
        "[`Err`] takes a [`String`] and obtains its successful `T` type from surrounding [`T!`] context.",
        ERR_EXAMPLE
    ),
    language_item!(
        SelfValue,
        "self",
        LanguageItemKind::Syntax,
        "self",
        "Refers to the current method receiver.",
        "A function declared as [`fn`] `Type.name` receives an implicit, precisely typed [`self`] value.",
        SELF_EXAMPLE
    ),
    language_item!(
        SignatureLiteral,
        "sig",
        LanguageItemKind::Syntax,
        "sig\"48 8B ?? B?\"",
        "Constructs a checked signature literal.",
        "Signatures are parsed at compile time and support full-byte and nibble wildcards.",
        SIGNATURE_EXAMPLE
    ),
    language_item!(
        VersionLiteral,
        "v",
        LanguageItemKind::Syntax,
        "v\"major.minor.build.private\"",
        "Constructs a checked Windows file-version literal.",
        "A version literal contains exactly four decimal [`u16`] components and has type [`FileVersion`]. It may also be used directly as a [`match`] pattern; an open-ended version match requires a wildcard arm. The quoted boundary keeps malformed versions from being parsed as unrelated numeric or member expressions.",
        VERSION_EXAMPLE
    ),
    language_item!(
        TemplateString,
        "template string",
        LanguageItemKind::Syntax,
        "`text {expression}`",
        "Interpolates values into a String.",
        "Backtick strings use braces without JavaScript's dollar marker. A dollar sign is ordinary text, so `${value}` emits `$` followed by the interpolated value rather than being treated as a typo. Non-[`String`] values use the same [`Display`] conversion as an [`as`] [`String`] cast.",
        TEMPLATE_EXAMPLE
    ),
    language_item!(
        ArrayType,
        "[T; N]",
        LanguageItemKind::Syntax,
        "[Element] or [Element; Length]",
        "Names a garbage-collected array type.",
        "[`[T]`] accepts any length. [`[T; N]`] carries an exact compile-time length, can be used wherever [`[T]`] is expected, and has a fixed process-memory layout when `T` satisfies [`MemoryReadable`].",
        ARRAY_TYPE_EXAMPLE
    ),
    language_item!(
        ArrayIndex,
        "array indexing",
        LanguageItemKind::Syntax,
        "array[index]",
        "Reads an array element.",
        "The receiver may be [`[T]`] or [`[T; N]`], the index is inferred as [`u32`], and the result has type `T`. WebAssembly performs the bounds check.",
        ARRAY_INDEX_EXAMPLE
    ),
    language_item!(
        OptionType,
        "T?",
        LanguageItemKind::Syntax,
        "Type?",
        "Names an optional type.",
        "A [`T?`] contains either [`Some`]`(T)` or [`None`]. Plain values lift to [`Some`], and [`match`] uses [`Some`]`(value)` plus [`None`].",
        OPTION_TYPE_EXAMPLE
    ),
    }
    builtins {
    builtin_type_item!(
        Never,
        "Never",
        "Describes an expression that cannot produce a value.",
        "The [`Never`] type is the bottom of SplitScript's type hierarchy and can flow into any expected value type. It is inferred for genuinely divergent control flow and is erased from WebAssembly. [`Process.closed`] returns [`async`] [`Never`] because process-lifetime cancellation prevents its await from resuming.",
        "fn ignoreUnsupportedBuild() -> async Never {\n    await process.closed()\n}"
    ),
    builtin_type_item!(
        None,
        "None",
        "Stores the single unit value [`None`].",
        "None is the ordinary return type for procedures and functions without a returned data value. Plain unit parameters, results, locals, globals, and async completions are erased from the physical Wasm representation. The same value converts to the empty side of T?, while Some(None) remains a present unit value.",
        "let unit: None = None"
    ),
    builtin_type_item!(
        Bool,
        "bool",
        "Stores a boolean value.",
        "Boolean values are true or false and are required by conditions and lifecycle decision blocks.",
        "let isBoss = current.level == 7"
    ),
    builtin_type_item!(
        Char,
        "char",
        "Stores one Unicode scalar value.",
        "Character literals use single quotes and contain exactly one Unicode scalar value. A char is distinct from u32, implements Display and equality, converts losslessly to u32 with [`as`], and has no implicit process-memory encoding.",
        "let marker: char = 'ß'"
    ),
    builtin_type_item!(
        I8,
        "i8",
        "Stores an 8-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let direction: i8 = -1"
    ),
    builtin_type_item!(
        U8,
        "u8",
        "Stores an 8-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let flags: u8 = 0xff"
    ),
    builtin_type_item!(
        I16,
        "i16",
        "Stores a 16-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let temperature: i16 = -20"
    ),
    builtin_type_item!(
        U16,
        "u16",
        "Stores a 16-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let room: u16 = 12"
    ),
    builtin_type_item!(
        I32,
        "i32",
        "Stores a 32-bit signed integer.",
        "Unconstrained integer literals and values specifically constrained as Integer default to i32. Memory reads never use this default and require an explicit or otherwise exact representation.",
        "let health: i32 = process.read(healthAddress) else 0"
    ),
    builtin_type_item!(
        U32,
        "u32",
        "Stores a 32-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let character: u32 = process.read(characterAddress) else 0"
    ),
    builtin_type_item!(
        I64,
        "i64",
        "Stores a 64-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let frameCount: i64 = 7200"
    ),
    builtin_type_item!(
        U64,
        "u64",
        "Stores a 64-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let sectionOffset: u64 = 0x1_0000"
    ),
    builtin_type_item!(
        Address,
        "address",
        "Stores a process address.",
        "Addresses use the target process pointer width and support checked address-oriented APIs without becoming ordinary untyped integers.",
        "let player: address = retry process.follow(base, offsets)"
    ),
    builtin_type_item!(
        F32,
        "f32",
        "Stores a 32-bit floating-point number.",
        "Floating-point values are useful for game coordinates, timers, and duration conversion. Decimal exponents and finite subnormal values are supported; target-width underflow and overflow are diagnosed.",
        "let smallestPositive: f32 = 1e-45"
    ),
    builtin_type_item!(
        F64,
        "f64",
        "Stores a 64-bit floating-point number.",
        "Floating-point values are useful for game coordinates, timers, and duration conversion. Decimal exponents and finite subnormal values are supported. Unconstrained floating-point literals and values specifically constrained as Float default to f64. Memory reads never use this default and require an explicit or otherwise exact representation.",
        "let smallestPositive: f64 = 5e-324"
    ),
    }
    compiler_symbols {
    compiler_symbol_item!(
        LanguageItemId::CurrentSnapshot,
        "current",
        LanguageItemKind::SnapshotRoot,
        "current.stateField",
        "Accesses the current committed state snapshot.",
        "State fields refresh before [`whileAttached`] and timer-decision actions run. Direct assignment replaces a field for the remainder of this tick and the resulting snapshot becomes [`old`] on the next successful poll. A failed field retains its last accepted value.",
        "current.level = 1",
        STATE_SOURCE
    ),
    compiler_symbol_item!(
        LanguageItemId::OldSnapshot,
        "old",
        LanguageItemKind::SnapshotRoot,
        "old.stateField",
        "Accesses the previous committed state snapshot.",
        "[`old`] contains the preceding emitted snapshot. A rejected field remains unchanged while successful sibling fields can advance into [`current`].",
        "return current.level != old.level",
        STATE_SOURCE
    ),
    compiler_symbol_item!(
        LanguageItemId::OldSettingsView,
        "oldSettings",
        LanguageItemKind::SnapshotRoot,
        "oldSettings.settingName",
        "Accesses the previous settings view.",
        "[`settings`] refreshes on every update; [`oldSettings`] retains the preceding values for change detection.",
        "let changed = settings.enabled != oldSettings.enabled",
        SETTINGS_SOURCE
    ),
    }
    ordinary_after {
    language_item!(
        ResultType,
        "T!",
        LanguageItemKind::Syntax,
        "Type!",
        "Names a fallible result type.",
        "A [`T!`] contains either [`Ok`]`(T)` or a [`String`] error. Plain values lift to [`Ok`], and [`match`] uses [`Ok`]`(value)` plus [`Err`]`(error)`.",
        RESULT_TYPE_EXAMPLE
    ),
    language_item!(
        DocumentationComment,
        "///",
        LanguageItemKind::Syntax,
        "/// documentation text",
        "Documents a source declaration, state field, setting, or heading.",
        "On functions and methods, global variables, state fields, records and their fields, and enums and their variants, the documentation appears in editor hovers. On settings and headings, it becomes a tooltip in the settings UI. Consecutive documentation-comment lines form paragraphs; use an empty [`///`] line to start a new paragraph.",
        DOCUMENTATION_COMMENT_EXAMPLES
    ),
    language_item!(
        ChoiceSetting,
        "choice setting",
        LanguageItemKind::Syntax,
        "\"Label\" => name: choice { \"Option\" => Enum.Variant default }",
        "Declares an enum-backed setting choice.",
        "Exactly one option may carry `default`; every option maps to a variant of one inferred [`enum`] type.",
        CHOICE_EXAMPLE
    ),
    language_item!(
        FileSetting,
        "file setting",
        LanguageItemKind::Syntax,
        "\"Label\" => name: file { \"Files\" => \"*.ext\" mime => \"type/*\" }",
        "Declares a file-selection setting.",
        "File settings support named glob filters, a wildcard fallback, and one or more MIME filters.",
        FILE_EXAMPLE
    ),
    }
    actions {
    action_item!(
        Setup,
        Setup,
        "setup",
        "Initializes one loaded script instance.",
        "Runs once from the module start entry point, after globals and [`settings`] are initialized and refreshed. The autosplitting runtime defers that entry point until the beginning of the first interruptible update. Settings and process-independent operations are available, but process providers, [`current`], [`old`], and suspension are not.",
        "setup {\n    print(\"Autosplitter loaded\")\n}"
    ),
    action_item!(
        OnDetach,
        OnDetach,
        "onDetach",
        "Handles closure of a previously attached process.",
        "Runs synchronously once when an attached process closes, after its unusable handle, provider state, selected layout, and pending continuations are cleared. It never runs for the initial detached state; use [`setup`] for one-time script initialization. Process and state snapshots are unavailable because closure may happen before initialization finishes.",
        "onDetach {\n    timer.pauseGameTime()\n}"
    ),
    action_item!(
        OnAttach,
        OnAttach,
        "onAttach",
        "Initializes one attached process.",
        "This action is implicitly suspending and owns process-lifetime cancellation for [`await`] and [`retry`] continuations. When the [`state`] declaration contains named [`layout`] declarations, it returns the generated layout variant that should be polled.",
        "onAttach {\n    let module = await process.module(\"GameAssembly.dll\")\n}"
    ),
    action_item!(
        OnStateReady,
        OnStateReady,
        "onStateReady",
        "Initializes one committed state snapshot.",
        "Runs synchronously once per attachment, immediately after the first complete state poll. Both [`old`] and [`current`] are available and equal. The attached process is available, but suspension is not. [`whileAttached`] and timer-decision actions begin on the following update.",
        "onStateReady {\n    print(`Initial level: {current.level}`)\n}"
    ),
    action_item!(
        WhileAttached,
        WhileAttached,
        "whileAttached",
        "Runs on every initialized attached update.",
        "State and [`settings`] data has already refreshed when this action runs. The initialization poll is deliberately skipped. Falling through or returning `true` continues to the timer-decision actions; returning `false` skips all of them for this update.",
        "whileAttached {\n    if !dataReady {\n        return false\n    }\n}"
    ),
    action_item!(
        Start,
        Start,
        "start",
        "Decides whether to start the timer.",
        "Falling through returns `false`.",
        "start {\n    return current.inGame && !old.inGame\n}"
    ),
    action_item!(
        Split,
        Split,
        "split",
        "Decides whether to advance the current split.",
        "Falling through returns `false`.",
        "split {\n    return current.level != old.level\n}"
    ),
    action_item!(
        Reset,
        Reset,
        "reset",
        "Decides whether to reset the timer.",
        "Falling through returns `false`.",
        "reset {\n    return current.newGame && !old.newGame\n}"
    ),
    action_item!(
        IsLoading,
        IsLoading,
        "isLoading",
        "Reports loading state when known.",
        "Falling through returns [`None`] so the runtime leaves the current loading state unchanged.",
        "isLoading {\n    return current.scene == \"Loading\"\n}"
    ),
    action_item!(
        GameTime,
        GameTime,
        "gameTime",
        "Reports the current game time when known.",
        "Falling through returns [`None`] so the runtime leaves the current game time unchanged.",
        "gameTime {\n    return Duration.fromSeconds(current.gameTime)\n}"
    ),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    pub const fn new() -> Self {
        Self
    }

    pub fn items(self) -> impl ExactSizeIterator<Item = &'static LanguageItem> {
        ITEMS.iter()
    }

    pub fn item(self, id: LanguageItemId) -> &'static LanguageItem {
        ITEMS
            .iter()
            .find(|item| item.id == id)
            .expect("every language item ID must have a catalog entry")
    }

    pub fn item_by_name(self, name: &str) -> Option<&'static LanguageItem> {
        ITEMS.iter().find(|item| item.name == name)
    }

    /// Resolves an exact source token or a short syntax spelling to its
    /// canonical documentation item.
    pub fn item_for_source_token(self, token: &str) -> Option<&'static LanguageItem> {
        self.item_by_name(token).or_else(|| {
            let id = match token {
                "Address" => LanguageItemId::BuiltinType(BuiltinType::Address),
                "[" => LanguageItemId::ArrayType,
                "?" => LanguageItemId::OptionType,
                "!" => LanguageItemId::ResultType,
                "///" => LanguageItemId::DocumentationComment,
                "`" => LanguageItemId::TemplateString,
                _ => return None,
            };
            Some(self.item(id))
        })
    }

    pub fn builtin_type(self, ty: BuiltinType) -> Option<&'static LanguageItem> {
        ITEMS
            .iter()
            .find(|item| item.id == LanguageItemId::BuiltinType(ty))
    }

    pub fn action(self, action: ActionKind) -> &'static LanguageItem {
        ITEMS
            .iter()
            .find(|item| item.kind == LanguageItemKind::Action(action))
            .expect("every action kind must have a language catalog entry")
    }

    pub fn validate(self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut example_sources = HashSet::new();
        for item in ITEMS {
            if !ids.insert(item.id) {
                errors.push(format!("duplicate language item ID `{:?}`", item.id));
            }
            if !names.insert(item.name) {
                errors.push(format!("duplicate language item name `{}`", item.name));
            }
            if item.form.trim().is_empty() {
                errors.push(format!("`{}` has no syntax form", item.name));
            }
            if item.documentation.summary.trim().is_empty() {
                errors.push(format!("`{}` has no documentation summary", item.name));
            }
            if item.documentation.details.trim().is_empty() {
                errors.push(format!("`{}` has no documentation details", item.name));
            }
            if item.documentation.examples.is_empty() {
                errors.push(format!("`{}` has no examples", item.name));
            }
            for example in item.documentation.examples {
                if example.title.trim().is_empty()
                    || example.source.trim().is_empty()
                    || !example.has_validation_source()
                {
                    errors.push(format!("`{}` has an incomplete example", item.name));
                }
                if !example_sources.insert(example.source) {
                    errors.push(format!(
                        "`{}` reuses another symbol's visible example",
                        item.name
                    ));
                }
            }
            for related in item.documentation.related {
                if !ITEMS.iter().any(|candidate| candidate.id == *related) {
                    errors.push(format!(
                        "`{}` links to missing language item `{:?}`",
                        item.name, related
                    ));
                }
            }
            if let LanguageItemKind::Action(action) = item.kind
                && item.name != action.name()
            {
                errors.push(format!(
                    "action catalog name `{}` does not match `{}`",
                    item.name,
                    action.name()
                ));
            }
        }
        for action in [
            ActionKind::Setup,
            ActionKind::OnDetach,
            ActionKind::OnAttach,
            ActionKind::OnStateReady,
            ActionKind::WhileAttached,
            ActionKind::Start,
            ActionKind::Split,
            ActionKind::Reset,
            ActionKind::IsLoading,
            ActionKind::GameTime,
        ] {
            if !ITEMS
                .iter()
                .any(|item| item.kind == LanguageItemKind::Action(action))
            {
                errors.push(format!("missing action catalog entry `{}`", action.name()));
            }
        }
        for builtin in [
            BuiltinType::Never,
            BuiltinType::None,
            BuiltinType::Bool,
            BuiltinType::I8,
            BuiltinType::U8,
            BuiltinType::I16,
            BuiltinType::U16,
            BuiltinType::I32,
            BuiltinType::U32,
            BuiltinType::I64,
            BuiltinType::U64,
            BuiltinType::Address,
            BuiltinType::F32,
            BuiltinType::F64,
        ] {
            if self.builtin_type(builtin).is_none() {
                errors.push(format!("missing built-in type catalog entry `{builtin}`"));
            }
        }
        errors
    }
}
