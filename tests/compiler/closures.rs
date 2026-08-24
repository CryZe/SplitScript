use splitscript::compiler::ast::Span;
use splitscript::compiler::types::TypeKind;
use splitscript::tooling::{
    database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    highlight::SemanticTokenKind,
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

#[test]
fn closure_parameters_are_first_class_editor_symbols() {
    let source = r#"state "game.exe" {}
whileAttached {
    let combine: (u16, u16) -> u16 = (x, y) => x + y
    print(combine(1, 2))
}"#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("the closure fixture should type-check");

    let declaration_x = source.find("(x, y)").unwrap() + 1;
    let declaration_y = source.find("x, y").unwrap() + "x, ".len();
    let use_x = source.find("=> x + y").unwrap() + "=> ".len();
    let use_y = source.find("x + y").unwrap() + "x + ".len();

    let x_definition = database
        .definition_at(use_x)
        .unwrap()
        .expect("closure parameter use should navigate");
    let DefinitionTarget::Source(x_definition) = x_definition else {
        panic!("closure parameters should be source definitions");
    };
    assert!(matches!(x_definition.id, SourceDefinitionId::Value(_)));
    assert_eq!(
        x_definition.span,
        Span {
            start: declaration_x,
            end: declaration_x + 1,
        }
    );
    assert_eq!(
        database.definition_at(declaration_x).unwrap(),
        Some(DefinitionTarget::Source(x_definition))
    );

    let hover = database
        .hover(use_y)
        .unwrap()
        .expect("closure parameter use should have hover information");
    assert!(hover.markdown.contains("y: u16"), "{}", hover.markdown);
    assert!(hover.markdown.contains("Parameter"), "{}", hover.markdown);

    let hints = database
        .inlay_hints(Span {
            start: 0,
            end: source.len(),
        })
        .unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint.position == declaration_x + 1 && hint.label == ": u16")
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.position == declaration_y + 1 && hint.label == ": u16")
    );

    let highlights = database.semantic_highlights().unwrap();
    for offset in [declaration_x, declaration_y, use_x, use_y] {
        let highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start <= offset && offset < highlight.span.end)
            .expect("each closure parameter occurrence should be highlighted");
        assert_eq!(highlight.kind, SemanticTokenKind::Parameter);
    }

    assert_eq!(database.references_at(use_x, true).unwrap().len(), 2);
    let renamed = database.rename_at(use_x, "left").unwrap();
    assert_eq!(renamed.len(), 2);
    assert!(renamed.iter().any(|span| span.start == declaration_x));
    assert!(renamed.iter().any(|span| span.start == use_x));
}
