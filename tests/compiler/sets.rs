use splitscript::tooling::database::CompilerDatabase;
use splitscript::tooling::highlight::SemanticTokenKind;
use wasmparser::{Validator, WasmFeatures};

const SETS: &str = r#"
let visited = Set.new<String>()

state "game.exe" {}

whileAttached {
    let firstVisit = visited.insert("Atrium")
    let alreadyVisited = visited.contains("Atrium")
    let removed = visited.remove("Atrium")
    visited.insert("Library")
    let count = visited.length()
    let empty = visited.isEmpty()
    for room in visited {
        print(room)
    }
    visited.clear()
    print(`{firstVisit} {alreadyVisited} {removed} {count} {empty}`)
}
"#;

#[test]
fn compiles_run_scoped_sets_and_iteration() {
    let wasm = splitscript::compile(SETS).expect("sets should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("set lowering should produce valid Wasm GC");
}

#[test]
fn empty_sets_infer_element_types_from_later_uses() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let visited = Set.new()
            visited.insert("Atrium")
            let found: bool = visited.contains("Atrium")
            print(found)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("insert and contains should infer an empty set's element type");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("usage-inferred sets should produce valid Wasm GC");
}

#[test]
fn genuinely_unconstrained_empty_sets_have_a_focused_diagnostic() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let visited = Set.new()
            }
        "#,
    )
    .expect_err("an unused empty set has no element constraint");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the element type of this empty set")
            && diagnostic.span.start > 0
    }));
}

#[test]
fn set_types_render_in_semantic_queries() {
    let checked = splitscript::check(splitscript::parse(SETS).unwrap()).unwrap();
    let visited = checked
        .syntax()
        .globals
        .iter()
        .find(|global| global.name == "visited")
        .unwrap();
    let ty = checked.semantics().value_type(visited.id).unwrap();
    assert!(matches!(
        checked.semantics().types().kind(ty),
        splitscript::compiler::types::TypeKind::Set { .. }
    ));
}

#[test]
fn set_elements_must_be_equatable() {
    let source = r#"
        let invalid = Set.new<[u8]>()
        state "game.exe" {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("arrays are not equatable");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("Equatable")
            || diagnostic.message.contains("does not support")
            || diagnostic.message.contains("constraints")
    }));
}

#[test]
fn explicit_set_types_enforce_source_declared_constructor_constraints() {
    let source = r#"
        state "game.exe" {}
        fn invalid(values: Set<[u8]>) {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("arrays are not equatable set keys");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("does not support")
            || diagnostic.message.contains("Equatable")
            || diagnostic.message.contains("constraints")
    }));
}

#[test]
fn generic_set_types_require_an_element_argument() {
    let source = r#"
        state "game.exe" {}
        fn invalid(values: Set) {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("bare Set must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("generic type constructor")
            && diagnostic.message.contains("Set<...>")
    }));
}

#[test]
fn named_spellings_of_structural_type_forms_are_rejected() {
    for (legacy, canonical) in [
        ("Array<i32>", "[T]"),
        ("Option<i32>", "T?"),
        ("Result<i32>", "T!"),
    ] {
        let source = format!("state \"game.exe\" {{}} record Legacy {{ values: {legacy} }}");
        let diagnostics =
            splitscript::compile(&source).expect_err("structural types use punctuation syntax");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .contains("is not SplitScript type syntax")
                    && diagnostic.labels.iter().any(|label| {
                        label
                            .message
                            .as_deref()
                            .is_some_and(|message| message.contains(canonical))
                    })
            }),
            "missing `{canonical}` guidance for `{legacy}`: {diagnostics:#?}"
        );
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("is not SplitScript type syntax")
            })
            .expect("focused structural-type diagnostic");
        let fix = diagnostic.fixes.first().expect("automatic syntax rewrite");
        assert_eq!(
            fix.applicability,
            splitscript::FixApplicability::MachineApplicable
        );
        let mut rewritten = source.clone();
        for edit in fix.edits.iter().rev() {
            rewritten.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }
        splitscript::compile(&rewritten).expect("the automatic rewrite must compile");
    }
}

#[test]
fn structural_type_rewrites_preserve_nested_arguments_and_fix_every_occurrence() {
    let source = r#"
        state "game.exe" {}
        record Legacy {
            first: Option<Array<u16> >,
            second: Option<Array<u16> >,
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("legacy forms must be rejected");
    let mut fixes = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .message
                .contains("is not SplitScript type syntax")
        })
        .flat_map(|diagnostic| diagnostic.fixes.iter())
        .collect::<Vec<_>>();
    assert_eq!(
        fixes.len(),
        2,
        "one fix should exist for each structural form: {diagnostics:#?}"
    );
    let mut edits = fixes
        .drain(..)
        .flat_map(|fix| fix.edits.iter())
        .collect::<Vec<_>>();
    edits.sort_by_key(|edit| edit.span.start);
    let mut rewritten = source.to_owned();
    for edit in edits.into_iter().rev() {
        rewritten.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(rewritten.contains("first: [u16] ?"), "{rewritten}");
    assert!(rewritten.contains("second: [u16] ?"), "{rewritten}");
    splitscript::compile(&rewritten).expect("nested automatic rewrites must compile");
}

#[test]
fn set_types_and_methods_are_available_to_editor_queries() {
    let source = r#"
        let visited = Set.new<String>()
        state "game.exe" {}
        whileAttached {
            visited.
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("visited.").unwrap() + "visited.".len();
    let labels = database
        .completions(offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    for method in ["length", "isEmpty", "contains", "insert", "remove", "clear"] {
        assert!(
            labels.contains(&method.to_owned()),
            "missing `{method}` completion"
        );
    }

    let hover_source = source.replace("visited.\n", "visited.length()\n");
    database.set_source(hover_source.clone());
    let use_site = hover_source.rfind("visited.length").unwrap() + 1;
    let hover = database.hover(use_site).unwrap().unwrap();
    assert!(hover.markdown.contains("Set<String>"));

    let highlights = database.semantic_highlights().unwrap();
    let set_offset = hover_source.find("Set").unwrap();
    assert!(highlights.highlights().iter().any(|highlight| {
        highlight.span.start <= set_offset
            && set_offset < highlight.span.end
            && highlight.kind == SemanticTokenKind::Type
    }));
    let set_hover = database
        .hover(set_offset + 1)
        .unwrap()
        .expect("type-constructor hover");
    assert!(
        set_hover
            .markdown
            .contains("```splitscript\nSet<T: Equatable>\n```")
    );
    assert!(
        set_hover
            .markdown
            .contains("Stores a growable collection of unique values")
    );

    let static_source = r#"
        state "game.exe" {}
        whileAttached { Set. }
    "#;
    database.set_source(static_source);
    let static_offset = static_source.find("Set.").unwrap() + "Set.".len();
    let static_labels = database
        .completions(static_offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    assert!(static_labels.contains(&"new".to_owned()));
}
