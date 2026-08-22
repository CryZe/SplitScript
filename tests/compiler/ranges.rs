use splitscript::compiler::types::TypeKind;
use splitscript::{
    compiler::stdlib::{StdlibFieldId, StdlibSymbolId},
    tooling::{
        completion::CompletionKind,
        database::{CompilerDatabase, DefinitionTarget},
    },
};
use wasmparser::{Parser, Payload, Validator, WasmFeatures};

const RANGES: &str = r#"
state "game.exe" {}

fn visit(exclusive: u16..<u16, inclusive: i64..=i64) {
    print(exclusive.start)
    print(exclusive.end)
    print(exclusive.contains(exclusive.start))
    print(exclusive.isEmpty())
    for value in exclusive {
        print(value)
    }
    print(inclusive.start)
    print(inclusive.end)
    print(inclusive.contains(inclusive.end))
    print(inclusive.isEmpty())
    for value in inclusive {
        print(value)
    }
}

whileAttached {
    let stored = 2u16..<5
    visit(stored, -2i64..=2)

    for value in 0u8..<3 {
        if value == 1 {
            continue
        }
        print(value)
    }

    // Reaching an inclusive integer maximum must terminate without wrapping.
    for maximum in 255u8..=255 {
        print(maximum)
    }
}
"#;

#[test]
fn compiles_first_class_and_direct_integer_ranges() {
    let wasm = splitscript::compile(RANGES).expect("integer ranges should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("range lowering should produce valid Wasm GC");
}

#[test]
fn preserves_inclusive_and_exclusive_range_types() {
    let checked = splitscript::check(splitscript::parse(RANGES).unwrap()).unwrap();
    let visit = checked
        .syntax()
        .functions
        .iter()
        .find(|function| function.name == "visit")
        .unwrap();
    let kinds = visit
        .params
        .iter()
        .map(|parameter| {
            let ty = checked.semantics().value_type(parameter.id).unwrap();
            match checked.semantics().types().kind(ty) {
                TypeKind::Range { kind, .. } => *kind,
                other => panic!("expected a range parameter, got {other:?}"),
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            splitscript::compiler::ast::RangeKind::Exclusive,
            splitscript::compiler::ast::RangeKind::Inclusive,
        ]
    );
}

#[test]
fn bare_ranges_explain_that_endpoint_inclusion_must_be_explicit() {
    for source in [
        r#"
            state "game.exe" {}
            whileAttached {
                for value in 0..3 {
                    print(value)
                }
            }
        "#,
        r#"
            state "game.exe" {}
            fn visit(values: u32..u32) {}
        "#,
    ] {
        let diagnostics =
            splitscript::parse(source).expect_err("bare `..` is intentionally not range syntax");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("upper bound is included"))
            .expect("the diagnostic should explain the explicit range operators");
        let replacements = diagnostic
            .fixes
            .iter()
            .flat_map(|fix| fix.edits.iter())
            .map(|edit| edit.replacement.as_str())
            .collect::<Vec<_>>();
        assert!(replacements.contains(&"..<"));
        assert!(replacements.contains(&"..="));
    }
}

#[test]
fn range_types_compose_with_postfix_wrappers() {
    splitscript::compile(
        r#"
            state "game.exe" {}

            fn visit(values: u32..<u32?) {
                let present = values else return
                for value in present {
                    print(value)
                }
            }

            whileAttached {
                visit(1u32 + 1..<8 / 2)
                visit(None)
            }
        "#,
    )
    .expect("a postfix wrapper applies to the complete range type");
}

#[test]
fn ranges_require_matching_integer_bounds() {
    for source in [
        r#"state "game.exe" {} whileAttached { let invalid = 1u8..<2u16 }"#,
        r#"state "game.exe" {} whileAttached { let invalid = 1.0..<2.0 }"#,
    ] {
        assert!(splitscript::compile(source).is_err());
    }
}

#[test]
fn suspending_range_loops_preserve_iteration_state() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                for value in 0u64..=2 {
                    await nextTick()
                    print(value)
                }
            }
        "#,
    )
    .expect("range loops should be resumable across await");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("suspending range loops should produce valid Wasm GC");
}

#[test]
fn direct_range_loops_do_not_construct_a_gc_range_value() {
    let direct = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                for value in 0u32..<3 {
                    print(value)
                }
            }
        "#,
    )
    .unwrap();
    let stored = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let values = 0u32..<3
                for value in values {
                    print(value)
                }
            }
        "#,
    )
    .unwrap();

    fn struct_new_count(wasm: &[u8]) -> usize {
        Parser::new(0)
            .parse_all(wasm)
            .filter_map(Result::ok)
            .filter_map(|payload| match payload {
                Payload::CodeSectionEntry(body) => Some(body),
                _ => None,
            })
            .flat_map(|body| body.get_operators_reader().unwrap().into_iter())
            .filter(|operator| matches!(operator, Ok(wasmparser::Operator::StructNew { .. })))
            .count()
    }

    assert_eq!(struct_new_count(&stored), struct_new_count(&direct) + 1);
}

#[test]
fn range_fields_and_methods_power_editor_queries() {
    let source = r#"
state "game.exe" {}

whileAttached {
    let exclusive = 2u16..<5
    print(exclusive.start)
    let inclusive = -2i64..=2
    print(inclusive.end)
    exclusive.
}
"#;
    let mut database = CompilerDatabase::new(source);
    let completion_offset = source.find("exclusive.\n").unwrap() + "exclusive.".len();
    let completions = database.completions(completion_offset).unwrap();
    for expected in ["start", "end", "contains", "isEmpty"] {
        assert!(
            completions.items.iter().any(|item| item.label == expected),
            "range completion is missing `{expected}`: {:#?}",
            completions.items
        );
    }
    for field in ["start", "end"] {
        assert_eq!(
            completions
                .items
                .iter()
                .find(|item| item.label == field)
                .unwrap()
                .kind,
            CompletionKind::Property,
        );
    }

    let start = source.find("exclusive.start").unwrap() + "exclusive.".len();
    assert_eq!(
        database.definition_at(start).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(StdlibFieldId::ExclusiveRangeStart),
        )),
    );
    let hover = database.hover(start).unwrap().expect("range field hover");
    assert!(
        hover.markdown.contains("T..<T.start: T"),
        "{}",
        hover.markdown
    );
    assert!(
        hover.markdown.contains("Range endpoints are immutable"),
        "{}",
        hover.markdown
    );

    let end = source.find("inclusive.end").unwrap() + "inclusive.".len();
    assert_eq!(
        database.definition_at(end).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(StdlibFieldId::InclusiveRangeEnd),
        )),
    );
}
