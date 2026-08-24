use wasmparser::{Validator, WasmFeatures};

use splitscript::{
    compiler::stdlib::StdlibItemId,
    tooling::{
        database::{CompilerDatabase, DefinitionTarget},
        language::LanguageItemId,
    },
};

const ITERATORS: &str = r#"
state "game.exe" {}

fn printStep(step: IteratorStep<u32>) {
    print(match step {
        Item(value) => value,
        End => 0,
    })
}

whileAttached {
    let array = [10u32, 20]
    let arrayIterator = array.iterator()
    printStep(arrayIterator.next())
    printStep(arrayIterator.next())
    printStep(arrayIterator.next())

    let set = Set.new<u32>()
    set.insert(30)
    let setIterator = set.iterator()
    printStep(setIterator.next())
    printStep(setIterator.next())

    let exclusive = (1u32..<2).iterator()
    printStep(exclusive.next())
    printStep(exclusive.next())

    let inclusive = (3u32..=3).iterator()
    printStep(inclusive.next())
    printStep(inclusive.next())

    let explicitItem: IteratorStep<u32> = Item(40)
    let explicitEnd: IteratorStep<u32> = End
    printStep(explicitItem)
    printStep(explicitEnd)
}
"#;

#[test]
fn compiles_first_class_iterators_and_step_patterns() {
    let wasm = splitscript::compile(ITERATORS).expect("iterators should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("iterator lowering should produce valid Wasm GC");
}

#[test]
fn iterator_exhaustion_is_distinct_from_an_optional_item() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            whileAttached {
                let values: [u32?] = [None, 7]
                let iterator = values.iterator()
                let first = iterator.next()
                let second = iterator.next()
                let exhausted = iterator.next()
                print(match first { Item(value) => value else 0, End => 1 })
                print(match second { Item(value) => value else 0, End => 1 })
                print(match exhausted { Item(_) => 0, End => 1 })
            }
        "#,
    )
    .expect("optional iterator items should compile without conflating None and End");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("optional iterator items should produce valid Wasm GC");
}

#[test]
fn lazy_iterator_adapters_transform_and_filter_remaining_items() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            whileAttached {
                let factor = 3u32
                let iterator = [1u32, 2, 3, 4]
                    .iterator()
                    .map(value => value * factor)
                    .filter(value => value > 6)
                let first = iterator.next()
                let second = iterator.next()
                let exhausted = iterator.next()
                print(match first { Item(value) => value, End => 0 })
                print(match second { Item(value) => value, End => 0 })
                print(match exhausted { Item(_) => 0, End => 1 })
            }
        "#,
    )
    .expect("map and filter should compose as lazy iterator cursors");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("lazy iterator adapters should produce valid Wasm GC");
}

#[test]
fn for_consumes_existing_iterator_cursors_and_lazy_adapters() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            whileAttached {
                let cursor = [1u32, 2, 3, 4].iterator()
                let first = cursor.next()
                for value in cursor.map(value => value * 2).filter(value => value > 4) {
                    print(value)
                }
                let raw = [5u32, 6].iterator()
                for value in raw {
                    print(value)
                }
                print(match first { Item(value) => value, End => 0 })
            }
        "#,
    )
    .expect("for should consume an existing iterator through its next protocol");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("iterator-consuming for loops should produce valid Wasm GC");
}

#[test]
fn iterator_consuming_for_survives_async_loop_bodies() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            onAttach {
                let cursor = [1u32, 2, 3]
                    .iterator()
                    .filter(value => value > 1)
                for value in cursor {
                    await nextTick()
                    print(value)
                }
            }
        "#,
    )
    .expect("iterator cursors should remain live across suspension in a for body");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("async iterator-consuming for loops should produce valid Wasm GC");
}

#[test]
fn discarded_iterator_values_are_must_use() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            [1, 2].iterator()
            let iterator = [3, 4].iterator()
            iterator.next()
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("must-use diagnostics must remain warnings");
    let diagnostics = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::MustUse)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2, "{:#?}", checked.diagnostics());
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.severity == splitscript::DiagnosticSeverity::Warning
            && diagnostic.message.starts_with("unused result of")
    }));
}

#[test]
fn iterator_methods_and_steps_are_available_to_editor_queries() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let iterator = [1u32, 2].iterator()
            iterator.
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let completion_offset = source.find("iterator.").unwrap() + "iterator.".len();
    let completions = database.completions(completion_offset).unwrap();
    assert!(
        completions.items.iter().any(|item| item.label == "next"),
        "{:#?}",
        completions.items
    );

    let resolved = source.replace("iterator.\n", "iterator.next()\n");
    database.set_source(resolved.clone());
    let next = resolved.find("iterator.next").unwrap() + "iterator.".len();
    assert_eq!(
        database.definition_at(next).unwrap(),
        Some(DefinitionTarget::StandardLibrary(
            StdlibItemId::ArrayIteratorNext,
        )),
    );
    let hover = database
        .hover(next)
        .unwrap()
        .expect("iterator method hover");
    assert!(hover.markdown.contains("ArrayIterator<u32>.next"));

    let patterns = r#"
        state "game.exe" {}
        fn inspect(step: IteratorStep<u32>) {
            print(match step { Item(value) => value, End => 0 })
        }
    "#;
    database.set_source(patterns.to_owned());
    for (spelling, item) in [
        ("Item", LanguageItemId::IteratorItem),
        ("End", LanguageItemId::IteratorEnd),
    ] {
        let offset = patterns.find(spelling).unwrap();
        assert_eq!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Language(item)),
        );
        assert!(database.hover(offset).unwrap().is_some());
    }
}

#[test]
fn for_loop_parameters_infer_the_iterable_contract_and_associated_item() {
    let source = r#"
        state "game.exe" {}

        fn inspect(values) {
            for value in values {
                print(value)
            }
        }

        whileAttached {
            let exclusive: u32..<u32 = 0..<10
            let inclusive: u32..=u32 = 0..=10
            inspect(exclusive)
            inspect(inclusive)
            inspect([1u32, 2])
            inspect(["forest", "castle"])
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("one inferred Iterable function should accept each built-in iterable shape");
    let function = checked.syntax().functions[0].id;
    let type_parameters = checked.semantics().function_type_parameters(function);
    assert_eq!(type_parameters.len(), 3);
    assert_eq!(
        checked
            .semantics()
            .generic_parameter_constraints(type_parameters[0]),
        [splitscript::compiler::stdlib::StdlibCapabilityId::Iterable]
    );
    assert!(
        checked
            .semantics()
            .generic_parameter_constraints(type_parameters[1])
            .contains(&splitscript::compiler::stdlib::StdlibCapabilityId::Display)
    );
    assert_eq!(
        checked
            .semantics()
            .generic_parameter_constraints(type_parameters[2]),
        [splitscript::compiler::stdlib::StdlibCapabilityId::Iterator]
    );
    let mut database = CompilerDatabase::new(source);
    let function_hover = database
        .hover(source.find("inspect(values)").unwrap())
        .unwrap()
        .expect("inferred iterator function hover");
    assert!(
        function_hover
            .markdown
            .contains("fn inspect(values: T) -> None where T: Iterable, T.Item: Display"),
        "{}",
        function_hover.markdown
    );
    let binding = source.find("value in values").unwrap();
    let binding_hover = database
        .hover(binding)
        .unwrap()
        .expect("projected iterator binding hover");
    assert!(binding_hover.markdown.contains("value: T.Item"));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("each projected iterator item should specialize to valid Wasm GC");
}

#[test]
fn inferred_iterable_items_participate_in_parameter_and_result_inference() {
    let source = r#"
        state "game.exe" {}

        fn firstOr(values, fallback) {
            for value in values {
                return value
            }
            return fallback
        }

        whileAttached {
            let number: u16 = firstOr(1u16..<2, 0u16)
            let text: String = firstOr(["forest"], "fallback")
            print(number)
            print(text)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("the associated item should unify with ordinary parameters and the result");
    let function = checked.syntax().functions[0].id;
    let mut database = CompilerDatabase::new(source);
    let function_hover = database
        .hover(source.find("firstOr(values").unwrap())
        .unwrap()
        .expect("projected result function hover");
    assert!(
        function_hover
            .markdown
            .contains("fn firstOr(values: T, fallback: T.Item) -> T.Item where T: Iterable"),
        "{}",
        function_hover.markdown
    );
    assert_eq!(
        checked.semantics().function_type_parameters(function).len(),
        3,
        "the iterable, projected item, and projected cursor are the generalized semantic types"
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("projected parameter and result types should specialize to valid Wasm GC");
}

#[test]
fn inferred_iterable_helpers_can_iterate_their_explicit_cursor() {
    let source = r#"
        state "game.exe" {}

        fn inspect(values) {
            print(values)
            for value in values.iterator().map(value => `{value}y`) {
                print(value)
            }
        }

        setup {
            inspect(["a", "b", "c"])
            inspect(0..<10)
            inspect(10..=20)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("the associated cursor should retain its Iterator constraint");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("specialized iterator helpers should produce valid Wasm GC");
}

#[test]
fn inferred_iterable_helpers_accept_iterator_cursors_as_identity_iterables() {
    let source = r#"
        state "game.exe" {}

        fn inspect(values) {
            for value in values {
                print(value)
            }
        }

        setup {
            inspect(["a", "b", "c"].iterator())
            inspect((0..<10).iterator())
            inspect((10..=20).iterator())
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database
        .check()
        .expect("iterator cursors should satisfy Iterable through identity iteration");
    assert!(
        database
            .hover(source.find("inspect(values)").unwrap())
            .unwrap()
            .is_some(),
        "invalid specializations must not poison editor analysis"
    );
    assert!(
        !database
            .semantic_highlights()
            .unwrap()
            .highlights()
            .is_empty()
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("identity-iterable cursor specializations should produce valid Wasm GC");
}
