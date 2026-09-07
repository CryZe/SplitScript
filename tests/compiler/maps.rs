use wasmparser::{Validator, WasmFeatures};

use splitscript::tooling::database::CompilerDatabase;

#[test]
fn maps_support_lookup_mutation_and_entry_iteration() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let routes = Map.new<String, u32>()
            let inserted = routes.insert("Atrium", 12)
            routes["Vault"] = 20
            routes["Atrium"] = 14
            if routes.containsKey("Atrium") {
                print(routes["Atrium"])
            }
            print(routes)
            for { key, value } in routes {
                print(`{key}: {value}`)
            }
            for entry in routes {
                print(entry)
            }
            let removed = routes.remove("Vault")
            print(`{inserted} {removed} {routes.length()} {routes.isEmpty()}`)
            routes.clear()
        }
    "#;
    let wasm = splitscript::compile(source).expect("maps should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("map lowering should produce valid Wasm GC");
}

#[test]
fn empty_maps_infer_key_and_value_types_from_later_uses() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let routes = Map.new()
            routes.insert("Atrium", 12)
            let value: u32 = routes["Atrium"]
            print(value)
        }
    "#;
    splitscript::compile(source).expect("map operations should infer both type arguments");
}

#[test]
fn unconstrained_empty_maps_diagnose_each_missing_type_argument() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let routes = Map.new()
            }
        "#,
    )
    .expect_err("an unused map has no key or value constraints");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the key type of this empty map")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the value type of this empty map")
    }));
}

#[test]
fn map_index_compound_assignment_evaluates_as_a_read_and_write() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes["Atrium"] = 12
            routes["Atrium"] += 2
            print(routes["Atrium"])
        }
    "#;
    let wasm = splitscript::compile(source).expect("map compound indexing should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("map compound indexing should produce valid Wasm GC");
}

#[test]
fn map_editor_surface_exposes_only_the_approved_lookup_api() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes.
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("routes.").unwrap() + "routes.".len();
    let labels = database
        .completions(offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    for method in [
        "length",
        "isEmpty",
        "containsKey",
        "insert",
        "remove",
        "clear",
        "iterator",
    ] {
        assert!(labels.contains(&method.to_owned()), "missing `{method}`");
    }
    for hidden in ["at", "set", "get"] {
        assert!(!labels.contains(&hidden.to_owned()), "leaked `{hidden}`");
    }
}

#[test]
fn map_entry_binding_patterns_complete_public_fields() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            for {
            } in routes {}
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("            } in routes").unwrap();
    let labels = database
        .completions(offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    assert!(
        labels.contains(&"key".to_owned()),
        "missing key: {labels:?}"
    );
    assert!(
        labels.contains(&"value".to_owned()),
        "missing value: {labels:?}"
    );
}

#[test]
fn map_entry_patterns_participate_in_nested_usefulness() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes["Atrium"] = 12
            for entry in routes {
                match entry {
                    MapEntry { key: "Atrium", value } => print(value),
                    { key: _, value: _ } => {},
                }
            }
        }
    "#;
    splitscript::compile(source)
        .expect("generic standard-library structs should support nested patterns");
}
