use splitscript::compiler::types::TypeKind;

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

    let callable_count = checked
        .semantics()
        .types()
        .iter()
        .filter(|(_, kind)| matches!(kind, TypeKind::Callable { .. }))
        .count();
    assert!(callable_count >= 1);
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
