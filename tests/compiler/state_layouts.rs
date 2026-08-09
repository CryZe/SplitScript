use wasmparser::{Validator, WasmFeatures};

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
