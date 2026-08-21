use wasmparser::{Validator, WasmFeatures};

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
            .attachment_globals()
            .available_layouts(globals[0].id)
            .count()
            == 2
    );
    assert!(
        checked
            .attachment_globals()
            .available_layouts(globals[1].id)
            .count()
            == 1
    );
    assert!(
        checked
            .attachment_globals()
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

    let spans = database.rename_at(second_declaration, "stage").unwrap();
    assert_eq!(spans.len(), 4);
    assert!(
        spans
            .iter()
            .all(|span| &source[span.start..span.end] == "level")
    );

    let mut renamed = source.to_owned();
    for span in spans.iter().rev() {
        renamed.replace_range(span.start..span.end, "stage");
    }
    splitscript::compile(&renamed).expect("the renamed shared state interface should compile");

    let use_site = source.find("current.level").unwrap() + "current.".len();
    assert_eq!(database.rename_at(use_site, "stage").unwrap(), spans);
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
    let spans = database.rename_at(first_declaration, "vehicle").unwrap();
    assert_eq!(spans.len(), 2);
    assert!(
        spans
            .iter()
            .all(|span| &source[span.start..span.end] == "bike")
    );
    let v9_declaration = source.find("bike: u16").unwrap();
    let v9_use = source.rfind("current.bike").unwrap() + "current.".len();
    assert!(spans.iter().all(|span| span.start != v9_declaration));
    assert!(spans.iter().all(|span| span.start != v9_use));
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
