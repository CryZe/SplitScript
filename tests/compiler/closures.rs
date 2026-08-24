use splitscript::compiler::types::TypeKind;
use splitscript::tooling::{
    database::{CompilerDatabase, DefinitionTarget},
    language::LanguageItemId,
};
use wasmparser::{Validator, WasmFeatures};

#[test]
fn infers_closure_parameters_and_results_bidirectionally() {
    let source = r#"
        state "game.exe" {}

        fn apply(value: u32, transform: (u32) -> u32) -> u32 {
            return transform(value)
        }

        whileAttached {
            let offset = 2u32
            let addOffset = value => value + offset
            let direct = addOffset(3)
            let contextual = apply(4, value => value * 2)
            print(direct)
            print(contextual)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("closures should type-check");
    assert!(
        checked
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("unused variable `addOffset`")),
        "invoking a callable variable is a read: {:#?}",
        checked.diagnostics()
    );

    let callable_count = checked
        .semantics()
        .types()
        .iter()
        .filter(|(_, kind)| matches!(kind, TypeKind::Callable { .. }))
        .count();
    assert!(callable_count >= 1);
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("closures should lower to valid WebAssembly GC");
}

#[test]
fn callable_annotations_accept_multiple_parameters_and_async_results() {
    let source = r#"
        state "game.exe" {}

        fn consume(callback: (u32, String) -> async bool) {}
    "#;
    splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("callable type syntax should be accepted");
}

#[test]
fn closure_and_callable_arrows_link_to_their_language_documentation() {
    let source = r#"
        state "game.exe" {}

        fn apply(value: u32, transform: (u32) -> u32) -> u32 {
            return transform(value)
        }

        whileAttached {
            print(apply(4, value => value * 2))
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let callable_arrow = source.find("(u32) -> u32").unwrap() + "(u32) ".len();
    let closure_arrow = source.find("value => value").unwrap() + "value ".len();

    assert_eq!(
        database.definition_at(callable_arrow).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::CallableType))
    );
    assert_eq!(
        database.definition_at(closure_arrow).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Closure))
    );
    assert!(
        database
            .hover(closure_arrow)
            .unwrap()
            .expect("closure arrow hover")
            .markdown
            .contains("lexical captures")
    );
}

#[test]
fn higher_order_helpers_infer_callable_parameters_from_usage() {
    let source = r#"
        state "game.exe" {}

        fn apply(value, transform) {
            return transform(value)
        }

        whileAttached {
            print(apply(4u32, value => value + 1))
            print(apply("ready", value => `{value}!`))
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("higher-order helper types should infer from each call site");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generic higher-order helper specializations should validate");
}
