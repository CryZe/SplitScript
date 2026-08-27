use wasmparser::{Validator, WasmFeatures};

#[test]
fn attachment_layout_dimensions_are_an_ordinary_typed_global_record() {
    let source = r#"
        enum Edition {
            BaseGame,
            DlcDemo,
        }

        enum Storefront {
            Steam,
            GOG,
        }

        state "game.exe" {
            layout {
                edition: Edition,
                storefront: Storefront,
            }

            level: u32 at 0x100
        }

        onAttach {
            return Layout {
                edition: Edition.BaseGame,
                storefront: Storefront.Steam,
            }
        }

        split {
            return layout.edition == Edition.BaseGame
                && layout.storefront == Storefront.Steam
                && old.level != current.level
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("provider-independent layout dimensions should compile as an ordinary record");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("layout records should lower to valid Wasm GC");
}

#[test]
fn managed_metadata_can_select_multiple_attachment_dimensions_automatically() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }

        image "Assembly-CSharp" {
            class GameManager {
                if layout.edition == Edition.Base && layout.storefront == Storefront.Steam {
                    u32 baseSteamMarker;
                }
                if layout.edition == Edition.Base && layout.storefront == Storefront.GOG {
                    u32 baseGogMarker;
                }
                if layout.edition == Edition.Demo && layout.storefront == Storefront.Steam {
                    u32 demoSteamMarker;
                }
                if layout.edition == Edition.Demo && layout.storefront == Storefront.GOG {
                    u32 demoGogMarker;
                }
            }
        }

        state Unity ["game.exe"] {
            layout {
                edition: Edition,
                storefront: Storefront,
            }
        }

        onAttach {
            print(layout.edition)
        }

        whileAttached {
            if layout.edition == Edition.Base {
                print("base")
            }
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("distinct managed presence patterns should select every dimension");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("automatic multi-dimensional layout selection should emit valid Wasm");
}

#[test]
fn automatic_layout_selection_requires_distinguishable_metadata_evidence() {
    let source = r#"
        enum Edition { Base, Demo }
        image "Assembly-CSharp" {
            class GameManager {
                u32 marker;
            }
        }
        state Unity ["game.exe"] {
            layout { edition: Edition }
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("unconditional metadata cannot identify either layout");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("do not distinguish every layout combination"))
    }));
}

#[test]
fn attachment_layout_dimensions_require_nonempty_source_enum_fields() {
    let non_enum = r#"
        state "game.exe" {
            layout {
                edition: u32,
            }
        }
        onAttach { return Layout { edition: 1 } }
    "#;
    let diagnostics = splitscript::compile(non_enum).expect_err("integers are not dimensions");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "layout dimension `edition` must use a source enum type"
    }));

    let empty = r#"
        state "game.exe" {
            layout {}
        }
        onAttach { return Layout {} }
    "#;
    let diagnostics = splitscript::compile(empty).expect_err("empty layouts are meaningless");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "an attachment layout needs at least one dimension"
    }));
}

#[test]
fn attachment_layout_type_value_and_dimensions_have_source_identity() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        enum Edition { BaseGame }
        state "game.exe" {
            /// The distributed game edition.
            layout {
                /// Selects the game's content set.
                edition: Edition,
            }
        }
        onAttach { return Layout { edition: Edition.BaseGame } }
        split { return layout.edition == Edition.BaseGame }
    "#;
    let mut database = CompilerDatabase::new(source);

    let type_use = source.find("Layout {").unwrap() + 1;
    let DefinitionTarget::Source(layout_type) = database.definition_at(type_use).unwrap().unwrap()
    else {
        panic!("Layout should navigate to the attachment declaration");
    };
    assert!(matches!(layout_type.id, SourceDefinitionId::Record(_)));
    assert_eq!(
        &source[layout_type.span.start..layout_type.span.end],
        "layout"
    );

    let value_use = source.find("layout.edition").unwrap() + 1;
    let DefinitionTarget::Source(layout_value) =
        database.definition_at(value_use).unwrap().unwrap()
    else {
        panic!("layout should navigate to the attachment declaration");
    };
    assert!(matches!(layout_value.id, SourceDefinitionId::Value(_)));
    assert_eq!(
        &source[layout_value.span.start..layout_value.span.end],
        "layout"
    );

    let dimension_use = source.find("layout.edition").unwrap() + "layout.".len();
    let DefinitionTarget::Source(dimension) =
        database.definition_at(dimension_use).unwrap().unwrap()
    else {
        panic!("the dimension should navigate to its declaration");
    };
    assert!(matches!(dimension.id, SourceDefinitionId::RecordField(_)));
    assert_eq!(&source[dimension.span.start..dimension.span.end], "edition");

    let hover = database.hover(dimension_use).unwrap().unwrap();
    assert!(hover.markdown.contains("Layout.edition: Edition"));
    assert!(hover.markdown.contains("Selects the game's content set."));
}

#[test]
fn conditional_state_fields_refine_multiple_attachment_dimensions() {
    let source = r#"
        enum Edition { BaseGame, DlcDemo }
        enum Storefront { Steam, GOG }

        state "game.exe" {
            layout {
                edition: Edition,
                storefront: Storefront,
            }
            common: u8 at 0x100;
            if layout.edition == Edition.BaseGame {
                baseLevel: u8 at 0x180;
            }
            if layout.edition == Edition.BaseGame
                && layout.storefront == Storefront.Steam
            {
                steamLevel: u16 at 0x200;
            }
        }

        onAttach {
            return Layout {
                edition: Edition.BaseGame,
                storefront: Storefront.Steam,
            }
        }

        split {
            let steamLevelChanged = layout.edition == Edition.BaseGame
                && layout.storefront == Storefront.Steam
                && current.steamLevel != old.steamLevel
            let baseLevelKnown = layout.edition == Edition.DlcDemo
                || current.common == 255
                || current.baseLevel > 0
            return steamLevelChanged || baseLevelKnown || current.common != old.common
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("layout predicates should refine and gate conditional state fields");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional state polling should produce valid Wasm GC");

    let unrefined = source.replace(
        "current.common != old.common",
        "current.steamLevel != old.steamLevel",
    );
    let diagnostics =
        splitscript::compile(&unrefined).expect_err("conditional fields need refinement");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state field `steamLevel` is conditional")
    }));
}

#[test]
fn conditional_state_field_chains_preserve_exact_remaining_layouts() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }

        state "game.exe" {
            layout {
                edition: Edition,
                storefront: Storefront,
            }
            if layout.edition == Edition.Base && layout.storefront == Storefront.Steam {
                steamLevel: u8 at 0x100;
            } else if layout.edition == Edition.Base {
                gogLevel: u8 at 0x200;
            } else {
                demoLevel: u8 at 0x300;
            }
        }

        onAttach {
            return Layout {
                edition: Edition.Base,
                storefront: Storefront.GOG,
            }
        }

        split {
            if layout.edition == Edition.Base && layout.storefront == Storefront.Steam {
                return current.steamLevel != old.steamLevel
            } else if layout.edition == Edition.Base && layout.storefront == Storefront.GOG {
                return current.gogLevel != old.gogLevel
            } else if layout.edition == Edition.Demo {
                return current.demoLevel != old.demoLevel
            }
            return false
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("else-if state fields should retain the exact layouts left by earlier branches");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional branch predicates should lower to valid Wasm GC");

    let insufficiently_refined = source.replace(
        "if layout.edition == Edition.Base && layout.storefront == Storefront.Steam {\n                return current.steamLevel != old.steamLevel\n            } else if layout.edition == Edition.Base && layout.storefront == Storefront.GOG {\n                return current.gogLevel != old.gogLevel\n            } else if layout.edition == Edition.Demo {\n                return current.demoLevel != old.demoLevel\n            }\n            return false",
        "if layout.edition == Edition.Base {\n                return current.gogLevel != old.gogLevel\n            }\n            return false",
    );
    let diagnostics = splitscript::compile(&insufficiently_refined)
        .expect_err("the else-if field is absent from the earlier Base/Steam branch");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state field `gogLevel` is conditional")
    }));
}

#[test]
fn conditional_layout_branch_enumeration_has_a_deterministic_bound() {
    let source = r#"
        enum Binary { A, B }
        state "game.exe" {
            layout {
                a: Binary,
                b: Binary,
                c: Binary,
                d: Binary,
                e: Binary,
                f: Binary,
                g: Binary,
                h: Binary,
                i: Binary,
            }
            if layout.a == Binary.A {
                value: u8 at 0x100;
            } else {
                other: u8 at 0x200;
            }
        }
        onAttach {
            return Layout {
                a: Binary.A,
                b: Binary.A,
                c: Binary.A,
                d: Binary.A,
                e: Binary.A,
                f: Binary.A,
                g: Binary.A,
                h: Binary.A,
                i: Binary.A,
            }
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("conditional declarations must not enumerate an unbounded layout product");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("conditional fields require a bounded attachment layout")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("at most 256 layout combinations"))
    }));
}

#[test]
fn managed_fields_share_the_attachment_layout_refinement_model() {
    let source = r#"
        enum Edition { BaseGame, Demo }

        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance;
                if layout.edition == Edition.BaseGame {
                    u32 level;
                }
                else {
                    u32 scene;
                }
            }
        }

        state Unity ["game.exe"] {
            layout { edition: Edition }
        }

        onAttach { return Layout { edition: Edition.BaseGame } }

        whileAttached {
            let manager = GameManager.instance else return
            if layout.edition == Edition.BaseGame {
                print(manager.level else 0)
            } else {
                print(manager.scene else 0)
            }
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("managed fields should consume the global layout predicate");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional managed bindings should produce valid Wasm GC");

    let unrefined = source.replace(
        "if layout.edition == Edition.BaseGame {\n                print(manager.level else 0)\n            }",
        "print(manager.level else 0)",
    );
    let diagnostics =
        splitscript::compile(&unrefined).expect_err("managed fields need layout refinement");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unknown field or method `level`")
                || diagnostic.message.contains("conditional")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn lunistice_shaped_unity_schema_reads_both_editions_without_manual_offsets() {
    let source = r#"
        enum Edition {
            BaseGame,
            DlcDemo,
        }

        record LevelTimeParts {
            minutes: f32,
            seconds: f32,
            hundredths: f32,
        }

        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance from "Instance";
                i32 gameState from ["gameState", "GameState"];
                u32 points from "_points";
                u32 deaths from "_deaths";

                if layout.edition == Edition.BaseGame {
                    i32 level from "currentLevel";
                }

                else {
                    address scene from "_currentScene";
                }
            }

            class Timer {
                static Timer instance from ["Instance", "_instance"];
                f32 levelTime from "currentLevelTime";
                LevelTimeParts levelTimeParts from "currentLevelTimeVector";
                bool stopped from "timerStopped";
                u32 character;
            }
        }

        state Unity.il2cpp(2020) ["Lunistice.exe", "Lunistice-Demo.exe"] {
            layout { edition: Edition }

            gameState: i32 = GameManager.instance?.gameState?;
            points: u32 = GameManager.instance?.points?;
            deaths: u32 = GameManager.instance?.deaths?;
            if layout.edition == Edition.BaseGame {
                level: i32 = GameManager.instance?.level?;
            }
            else {
                scene: String = process.readManagedString(GameManager.instance?.scene?, 16)?;
            }
            levelTime: f32 = Timer.instance?.levelTime?;
            levelTimeParts: LevelTimeParts = Timer.instance?.levelTimeParts?;
            timerStopped: bool = Timer.instance?.stopped?;
            character: u32 = Timer.instance?.character?;
        }

        whileAttached {
            print(current.levelTimeParts)
            if layout.edition == Edition.BaseGame {
                print(current.level)
            } else {
                print(current.scene)
            }
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("the Lunistice Unity schema should type check");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused record")
                || diagnostic.message.starts_with("unused enum")
        })
        .collect::<Vec<_>>();
    assert!(
        unused.is_empty(),
        "generated layout declarations are used: {unused:#?}"
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the Lunistice-shaped schema should produce valid Wasm GC");
}

#[test]
fn sonic_three_air_shaped_range_discovery_and_filtered_state_compile_cleanly() {
    let source = r#"
        let wramBase

        enum Level {
            AngelIsland1,
            AngelIsland2,
        }

        settings {
            "Angel Island 1" => angelIsland1: true,
            "Angel Island 2" => angelIsland2: true,
        }

        state "Sonic3AIR.exe" {
            rawGameState: u8 = process.read(wramBase.offset(0xf600))?;
            rawSaveSlot: u8 = process.read(wramBase.offset(0xf61a))?;
            rawLevel: u8 = process.read(wramBase.offset(0xee4e))?;
            timeBonus: u16 = process.read<u16>(wramBase.offset(0xf7d2))?.swapBytes();
            saveSlot: u8 = 0;
            level: Level = Level.AngelIsland1;
        }

        onAttach {
            wramBase = loop {
                let mapping = await process.findMemoryRange(
                    0x521000,
                    MemoryRangeAccess.Read,
                )
                match mapping {
                    Some(range) => break range.address.offset(0x400020),
                    None => {
                        await nextTick()
                        continue
                    },
                }
            }
        }

        whileAttached {
            current.saveSlot = if current.rawSaveSlot < 8 {
                current.rawSaveSlot
            } else {
                old.saveSlot
            }
            current.level = match current.rawLevel {
                0 => Level.AngelIsland1,
                1 => Level.AngelIsland2,
                _ => old.level,
            }
        }

        start {
            return old.rawGameState != 0x0c && current.rawGameState == 0x0c
        }

        reset {
            return old.saveSlot != current.saveSlot
        }

        split {
            return old.level != current.level
                && match current.level {
                    Level.AngelIsland1 => settings.angelIsland1,
                    Level.AngelIsland2 => settings.angelIsland2,
                }
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("the Sonic 3 A.I.R. memory-range and endian patterns should be first-class");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the representative port should lower to valid Wasm GC");
}

#[test]
fn attachment_scoped_globals_infer_from_on_attach_and_support_layout_specific_values() {
    let source = r#"
        let module
        let steamBase
        let gogBase

        state "game.exe" {
            layout Steam { level: u32 = process.read(steamBase)? },
            layout GOG { level: u32 = process.read(gogBase)? },
        }

        onAttach {
            module = await process.mainModule()
            if module.size == 0x1000 {
                steamBase = module.address
                return StateLayout.Steam
            }
            gogBase = module.address
            return StateLayout.GOG
        }

        split {
            return match layout {
                StateLayout.Steam => steamBase != 0 && current.level != old.level,
                StateLayout.GOG => gogBase != 0 && current.level != old.level,
            }
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("attachment globals should infer from assignments and uses");
    let globals = checked.syntax().globals.iter().collect::<Vec<_>>();
    assert!(
        checked
            .scoped_globals()
            .available_layouts(globals[0].id)
            .count()
            == 2
    );
    assert!(
        checked
            .scoped_globals()
            .available_layouts(globals[1].id)
            .count()
            == 1
    );
    assert!(
        checked
            .scoped_globals()
            .available_layouts(globals[2].id)
            .count()
            == 1
    );

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("attachment-global WebAssembly GC should validate");

    let wrong_layout = source.replace(
        "layout Steam { level: u32 = process.read(steamBase)? }",
        "layout Steam { level: u32 = process.read(gogBase)? }",
    );
    let errors = splitscript::compile(&wrong_layout)
        .expect_err("state expressions need the attachment values for their own layout");
    assert!(errors.iter().any(|error| {
        error.message.contains(
            "attachment-scoped global `gogBase` is not initialized for `StateLayout.Steam`",
        )
    }));
}

#[test]
fn attachment_scoped_gc_values_use_nullable_storage_without_exposing_null() {
    let source = r#"
        let delay: Duration

        state "game.exe" {}

        onAttach {
            delay = Duration.fromSeconds(1)
        }

        split {
            return delay > Duration.zero()
        }
    "#;

    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            source,
            splitscript::CompilerOptions {
                profile,
                ..splitscript::CompilerOptions::default()
            },
        )
        .expect("attachment-scoped non-null GC values should compile");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("nullable attachment storage must validate as a non-null source value");
    }
}

#[test]
fn debug_attachment_globals_are_initialized_and_erased_with_their_profile() {
    let source = r#"
        debug let inspectedAddress

        state "game.exe" {}

        onAttach {
            debug inspectedAddress = 0x1000
        }

        whileAttached {
            debug print(inspectedAddress)
        }
    "#;

    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            source,
            splitscript::CompilerOptions {
                profile,
                ..splitscript::CompilerOptions::default()
            },
        )
        .expect("debug attachment globals should follow debug statement lifetime");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("both attachment-global profiles should validate");
    }
}

#[test]
fn attachment_globals_require_definite_initialization_and_layout_refinement() {
    let missing = r#"
        let base: address
        state "game.exe" {}
        onAttach {
            if process.name() == "game.exe" {
                base = 0x1000
            }
        }
        split { return base != 0 }
    "#;
    let errors = splitscript::compile(missing)
        .expect_err("single-layout attachment values need assignment on every completion path");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `base` is never initialized by `onAttach`"
            || error.message.contains("not initialized for the attachment")
    }));

    let unrefined = r#"
        let steamBase: address
        let gogBase: address
        state "game.exe" {
            layout Steam { level: u32 at 0x10 },
            layout GOG { level: u32 at 0x20 },
        }
        onAttach {
            if process.name() == "game.exe" {
                steamBase = 0x1000
                return StateLayout.Steam
            }
            gogBase = 0x2000
            return StateLayout.GOG
        }
        fn steamReady() -> bool { return steamBase != 0 }
        split { return steamReady() }
    "#;
    let errors = splitscript::compile(unrefined)
        .expect_err("a layout-specific helper needs a matching refinement");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("`steamReady` requires attachment values unavailable for `StateLayout.GOG`")
    }));

    let refined = unrefined.replace(
        "split { return steamReady() }",
        r#"split {
            return match layout {
                StateLayout.Steam => steamReady(),
                StateLayout.GOG => gogBase != 0,
            }
        }"#,
    );
    splitscript::compile(&refined)
        .expect("a direct layout match should prove attachment-global availability");
}

#[test]
fn on_attach_rejects_attachment_global_reads_before_assignment() {
    let source = r#"
        let module: Module
        state "game.exe" {}
        onAttach {
            let copy = module
            module = await process.mainModule()
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("backend defaults must not be observable during initialization");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `module` may be read before it is initialized"
    }));
}

#[test]
fn attachment_globals_are_viral_and_unavailable_after_detach() {
    let source = r#"
        let base: address
        state "game.exe" {}
        onAttach { base = 0x1000 }
        fn hasBase() -> bool { return base != 0 }
        onDetach {
            let copy = base
            base = 0x2000
            hasBase()
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("detached code must not observe cleared attachment storage");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `base` is unavailable in `onDetach`"
            && error.labels.iter().any(|label| {
                label
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("write occurs"))
            })
    }));
    assert!(errors.iter().any(|error| {
        error.message == "`hasBase` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn attempt_scoped_globals_infer_from_on_start_and_are_viral() {
    let source = r#"
        let accumulated
        state "game.exe" {}

        onStart {
            accumulated = 0.0
        }

        fn add(value: f64) {
            accumulated += value
        }

        gameTime {
            add(1.5)
            return Duration.fromSeconds(accumulated)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("onStart should infer attempt-scoped globals and helper requirements");
    let accumulated = checked.syntax().globals[0].id;
    assert!(checked.scoped_globals().is_attempt_global(accumulated));
    let helper = checked.syntax().functions[0].id;
    assert!(checked.scoped_globals().function_requires_attempt(helper));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("attempt-scoped global WebAssembly GC should validate");
}

#[test]
fn attempt_scoped_globals_require_one_definite_initializer() {
    let partial = r#"
        let accumulated: f64
        state "game.exe" {}
        onStart {
            if timer.state() == TimerState.Running {
                accumulated = 0.0
            }
        }
        gameTime { return Duration.fromSeconds(accumulated) }
    "#;
    let diagnostics = splitscript::compile(partial)
        .expect_err("attempt globals need assignment on every completed onStart path");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(
            "attempt-scoped global `accumulated` is not initialized on every `onStart` path",
        )
    }));

    let read_before_write = r#"
        let accumulated: f64
        state "game.exe" {}
        onStart {
            print(accumulated)
            accumulated = 0.0
        }
    "#;
    let diagnostics = splitscript::compile(read_before_write)
        .expect_err("backend defaults must not be observable in onStart");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "attempt-scoped global `accumulated` may be read before it is initialized"
    }));

    let ambiguous = r#"
        let value: u32
        state "game.exe" {}
        onAttach { value = 1 }
        onStart { value = 2 }
    "#;
    let diagnostics = splitscript::compile(ambiguous)
        .expect_err("one bare global cannot have two lifecycle owners");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "bare global `value` has both attachment and attempt initializers"
    }));
}

#[test]
fn attempt_scoped_globals_are_rejected_outside_attempt_actions() {
    for action in ["setup", "onAttach", "onDetach", "whileAttached", "start"] {
        let source = format!(
            r#"
                let accumulated: f64
                state "game.exe" {{}}
                onStart {{ accumulated = 0.0 }}
                {action} {{ print(accumulated) }}
            "#
        );
        let diagnostics = match splitscript::compile(&source) {
            Err(diagnostics) => diagnostics,
            Ok(_) => panic!("`{action}` can execute before attempt initialization"),
        };
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(&format!(
                    "attempt-scoped global `accumulated` is unavailable in `{action}`"
                )))
        );
    }
}

#[test]
fn named_state_layouts_select_a_typed_layout_and_validate() {
    let source = r#"
        state "game.exe" {
            /// Steam memory layout.
            layout Steam {
                level: u32 at 0x100;
                loading: bool at 0x104;
            },

            /// GOG memory layout.
            layout GOG {
                loading: bool at 0x204;
                level: u32 at 0x200;
            },
        }

        onAttach {
            let module = await process.mainModule()
            if module.size == 0x1000 {
                return StateLayout.Steam
            }
            if module.size == 0x2000 {
                return StateLayout.GOG
            }
            await process.closed()
        }

        split {
            return layout == StateLayout.Steam && current.level != old.level
        }
    "#;

    let wasm = splitscript::compile(source).expect("named layouts should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("named-layout WebAssembly GC should validate");
}

#[test]
fn alternate_process_names_can_select_matching_named_layouts() {
    let source = r#"
        state ["CrazyMachines.exe", "cm_family.exe", "cmnftl.exe"] {
            layout Original {
                win: u8 at 0x10F344, 0xE0, 0xC, 0x4, 0x4, 0x8, 0x50;
            },
            layout Family {
                win: u8 at 0x110484, 0xE0, 0xC, 0x4, 0x4, 0x8, 0x50;
            },
            layout NewFromTheLab {
                win: u8 at 0x112764, 0xE0, 0xC, 0x4, 0x4, 0x8, 0x50;
            },
        }

        tickRate { attached: 120 }

        onAttach {
            return match process.name() {
                "CrazyMachines.exe" => StateLayout.Original,
                "cm_family.exe" => StateLayout.Family,
                "cmnftl.exe" => StateLayout.NewFromTheLab,
                _ => await process.closed(),
            }
        }

        split { return current.win > old.win }
    "#;

    let wasm = splitscript::compile(source)
        .expect("alternate exact process identities should select named layouts");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("alternate-process named-layout WebAssembly GC should validate");
}

#[test]
fn named_state_layouts_refine_layout_specific_fields_and_types() {
    let source = r#"
        state "game.exe" {
            layout V8 {
                loading: i32 at 0x100;
                bike: i16 at 0x104;
            },
            layout V9 {
                loading: i32 at 0x200;
                bike: u16 at 0x204;
                video: u8 at 0x206;
            },
        }
        onAttach { return StateLayout.V8 }
        isLoading { return current.loading == 1 }
        split {
            return match layout {
                StateLayout.V8 => old.bike != 21368 && current.bike == 21368,
                StateLayout.V9 => old.bike != 52688 && current.bike == 52688 && current.video == 0,
            }
        }
    "#;

    let wasm =
        splitscript::compile(source).expect("layout refinement should expose concrete fields");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("non-uniform state layout Wasm GC should validate");

    let outside = source.replace(
        "isLoading { return current.loading == 1 }",
        "isLoading { return current.bike == 1 }",
    );
    let errors = splitscript::compile(&outside).expect_err("specific fields need refinement");
    assert!(errors.iter().any(|error| error.message ==
        "state field `bike` is layout-specific; access it inside the corresponding `match layout` arm"));
}

#[test]
fn named_state_layouts_require_a_total_on_attach_selection() {
    for (source, expected) in [
        (
            r#"
                state "game.exe" {
                    layout Steam { level: u32 at 0x100 }
                }
            "#,
            "named state layouts require an `onAttach` block that returns the selected layout",
        ),
        (
            r#"
                state "game.exe" {
                    layout Steam { level: u32 at 0x100 }
                }
                onAttach {
                    if process.name() == "game.exe" {
                        return StateLayout.Steam
                    }
                }
            "#,
            "`onAttach` must return a layout on every completing path",
        ),
        (
            r#"
                state "game.exe" {
                    layout Steam { level: u32 at 0x100 }
                }
                onAttach {
                    print(layout)
                    return StateLayout.Steam
                }
            "#,
            "`layout` is only available after `onAttach` has returned it",
        ),
    ] {
        let errors = splitscript::compile(source).expect_err("invalid layout selection must fail");
        assert!(
            errors.iter().any(|error| error.message == expected),
            "missing `{expected}` in {errors:#?}"
        );
    }
}

#[test]
fn named_layout_diagnostics_offer_safe_selection_fixes() {
    use splitscript::FixApplicability;

    let missing = r#"state "game.exe" {
    layout Steam { level: u32 at 0x100 },
    layout GOG { level: u32 at 0x200 },
}"#;
    let diagnostics = splitscript::compile(missing).unwrap_err();
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .starts_with("named state layouts require")
        })
        .expect("missing selector diagnostic");
    let fix = diagnostic.fixes.first().expect("selector skeleton fix");
    assert_eq!(fix.applicability, FixApplicability::HasPlaceholders);
    let fixed = apply_fix(missing, fix);
    assert!(fixed.contains("return StateLayout.Steam"));
    assert!(fixed.contains("return StateLayout.GOG"));
    assert!(fixed.ends_with("await process.closed()\n}"));
    splitscript::compile(&fixed).expect("the inert skeleton should compile safely");

    let fallthrough = r#"state "game.exe" {
    layout Steam { level: u32 at 0x100 },
}
onAttach {
    if process.name() == "game.exe" {
        return StateLayout.Steam
    }
}"#;
    let diagnostics = splitscript::compile(fallthrough).unwrap_err();
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.starts_with("`onAttach` must return"))
        .expect("non-total selector diagnostic");
    let fix = diagnostic.fixes.first().expect("unsupported-build fix");
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    let fixed = apply_fix(fallthrough, fix);
    splitscript::compile(&fixed).expect("the process-close fallback should make selection total");

    let provider = r#"state GBA {
    layout English { level: u8 at 0x100 },
}"#;
    let diagnostics = splitscript::compile(provider).unwrap_err();
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .starts_with("named state layouts require")
        })
        .expect("provider selector diagnostic");
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("no generic process-close wait"))
    );
}

fn apply_fix(source: &str, fix: &splitscript::DiagnosticFix) -> String {
    let mut fixed = source.to_owned();
    for edit in fix.edits.iter().rev() {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    fixed
}

#[test]
fn generated_layout_type_and_value_navigate_but_are_not_renameable() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        state "game.exe" {
            layout Steam { level: u32 at 0x100 }
        }
        onAttach { return StateLayout.Steam }
        split { return layout == StateLayout.Steam }
    "#;
    let mut database = CompilerDatabase::new(source);

    let layout = source.find("layout ==").unwrap() + 1;
    let DefinitionTarget::Source(definition) = database.definition_at(layout).unwrap().unwrap()
    else {
        panic!("layout should navigate to a source domain declaration");
    };
    assert!(matches!(definition.id, SourceDefinitionId::Value(_)));
    assert_eq!(definition.name, "layout");
    assert!(database.rename_target_at(layout).unwrap().is_none());

    let state_layout = source.rfind("StateLayout").unwrap() + 1;
    let DefinitionTarget::Source(definition) =
        database.definition_at(state_layout).unwrap().unwrap()
    else {
        panic!("StateLayout should navigate to its generated source identity");
    };
    assert!(matches!(definition.id, SourceDefinitionId::Enum(_)));
    assert_eq!(definition.name, "StateLayout");
    assert!(database.rename_target_at(state_layout).unwrap().is_none());

    let steam = source.rfind("Steam").unwrap() + 1;
    assert!(database.rename_target_at(steam).unwrap().is_some());
}

#[test]
fn renaming_a_named_layout_field_updates_the_shared_state_interface() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        state "game.exe" {
            layout Steam { level: u32 at 0x100 },
            layout GOG { level: u32 at 0x200 },
        }
        onAttach { return StateLayout.Steam }
        split { return current.level != old.level }
    "#;
    let second_declaration = source.match_indices("level: u32").nth(1).unwrap().0;
    let mut database = CompilerDatabase::new(source);

    let plan = database.rename_at(second_declaration, "stage").unwrap();
    assert_eq!(plan.edits.len(), 4);
    assert!(
        plan.edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "level")
    );

    let mut renamed = source.to_owned();
    for edit in plan.edits.iter().rev() {
        renamed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    splitscript::compile(&renamed).expect("the renamed shared state interface should compile");

    let use_site = source.find("current.level").unwrap() + "current.".len();
    assert_eq!(database.rename_at(use_site, "stage").unwrap(), plan);
}

#[test]
fn renaming_a_conflicting_layout_field_keeps_the_other_layout_independent() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        state "game.exe" {
            layout V8 { bike: i16 at 0x100 },
            layout V9 { bike: u16 at 0x200 },
        }
        onAttach { return StateLayout.V8 }
        split {
            return match layout {
                StateLayout.V8 => current.bike == 1,
                StateLayout.V9 => current.bike == 2,
            }
        }
    "#;
    let first_declaration = source.find("bike: i16").unwrap();
    let mut database = CompilerDatabase::new(source);
    let plan = database.rename_at(first_declaration, "vehicle").unwrap();
    assert_eq!(plan.edits.len(), 2);
    assert!(
        plan.edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "bike")
    );
    let v9_declaration = source.find("bike: u16").unwrap();
    let v9_use = source.rfind("current.bike").unwrap() + "current.".len();
    assert!(
        plan.edits
            .iter()
            .all(|edit| edit.span.start != v9_declaration)
    );
    assert!(plan.edits.iter().all(|edit| edit.span.start != v9_use));
}

#[test]
fn layout_refinement_drives_hover_and_definition_identity() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget};

    let source = r#"
        state "game.exe" {
            layout V8 { bike: i16 at 0x100 },
            layout V9 { bike: u16 at 0x200 },
        }
        onAttach { return StateLayout.V8 }
        split {
            return match layout {
                StateLayout.V8 => current.bike == 1,
                StateLayout.V9 => current.bike == 2,
            }
        }
    "#;
    let uses = source
        .match_indices("current.bike")
        .map(|(offset, _)| offset + "current.".len())
        .collect::<Vec<_>>();
    let mut database = CompilerDatabase::new(source);
    let v8_hover = database.hover(uses[0]).unwrap().unwrap();
    let v9_hover = database.hover(uses[1]).unwrap().unwrap();
    assert!(
        v8_hover.markdown.contains("bike: i16"),
        "{}",
        v8_hover.markdown
    );
    assert!(
        v9_hover.markdown.contains("bike: u16"),
        "{}",
        v9_hover.markdown
    );

    let DefinitionTarget::Source(v8) = database.definition_at(uses[0]).unwrap().unwrap() else {
        panic!("V8 field should navigate to source")
    };
    let DefinitionTarget::Source(v9) = database.definition_at(uses[1]).unwrap().unwrap() else {
        panic!("V9 field should navigate to source")
    };
    assert_ne!(v8.id, v9.id);
    assert_eq!(&source[v8.span.start..v8.span.end], "bike");
    assert_eq!(&source[v9.span.start..v9.span.end], "bike");
}
