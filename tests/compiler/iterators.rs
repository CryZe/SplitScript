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
