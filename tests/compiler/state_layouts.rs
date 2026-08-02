use wasmparser::{Validator, WasmFeatures};

#[test]
fn named_state_layouts_select_a_typed_layout_and_validate() {
    let source = r#"
        state "game.exe" {
            /// Steam memory layout.
            layout Steam {
                level: u32 at 0x100
                loading: bool at 0x104
            }

            /// GOG memory layout.
            layout GOG {
                loading: bool at 0x204
                level: u32 at 0x200
            }
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
fn named_state_layouts_require_the_same_interface() {
    let source = r#"
        state "game.exe" {
            layout Steam { level: u32 at 0x100 }
            layout GOG { loading: bool at 0x200 }
        }
        onAttach { return StateLayout.Steam }
    "#;

    let errors = splitscript::check(splitscript::parse(source).unwrap()).unwrap_err();
    let messages = errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>();
    assert!(messages.contains(&"state layout field `loading` is not present in the first layout"));
    assert!(messages.contains(&"state layout is missing field `level`"));
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
