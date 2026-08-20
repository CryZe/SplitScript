//! failure semantics integration tests.

use super::*;

#[test]
fn bounded_native_string_reads_are_fallible_and_state_sugar_infers_string() {
    use splitscript::compiler::{
        ast::{StateMemoryDecoder, StateSource},
        stdlib::StdlibTypeId,
        types::TypeKind,
    };

    let source = r#"
        state "game.exe" {
            mapName at "game.dll", 0x100, 0x20 as utf8(32);
            chapterName at 0x200 as utf16le(64)
        }

        whileAttached {
            let direct: String! = process.readUtf8(0x2000, 32)
            let wide: String! = process.readUtf16Le(0x3000, 64)
            print(current.mapName)
            print(current.chapterName)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let field = &checked.syntax().state.as_ref().unwrap().fields[0];
    let StateSource::Pointer(path) = &field.source else {
        panic!("expected a pointer-backed state field");
    };
    assert!(matches!(
        path.decoder,
        Some(StateMemoryDecoder::Utf8 { max_bytes: 32, .. })
    ));
    let StateSource::Pointer(wide_path) =
        &checked.syntax().state.as_ref().unwrap().fields[1].source
    else {
        panic!("expected a pointer-backed UTF-16LE state field");
    };
    assert!(matches!(
        wide_path.decoder,
        Some(StateMemoryDecoder::Utf16Le { max_units: 64, .. })
    ));
    let field_type = checked.semantics().value_type(field.id).unwrap();
    assert_eq!(
        checked.semantics().types().kind(field_type),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("bounded UTF-8 reads after a pointer path should validate");

    for (source, expected) in [
        (
            "state \"game.exe\" { name at 0x100 as utf8(0) }",
            "must allow at least one byte",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf8(4097) }",
            "limited to 4096 bytes",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf16le(0) }",
            "must allow at least one code unit",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf16le(2049) }",
            "limited to 2048 code units",
        ),
        (
            "state GBA { name at 0x02000000 as utf8(32) }",
            "does not yet support decoded string fields",
        ),
    ] {
        let errors = splitscript::check(splitscript::parse(source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "missing `{expected}` diagnostic in {errors:#?}"
        );
    }
}

#[test]
fn explicitly_optional_pointer_fields_observe_read_failure_as_none() {
    use splitscript::compiler::{
        stdlib::StdlibTypeId,
        types::{BuiltinType, TypeKind},
    };

    let source = r#"
        state "game.exe" {
            scalar: i32? at 0x1000;
            mapName: String? at 0x2000 as utf8(32);
            chapterName: String? at 0x3000 as utf16le(32)
        }

        whileAttached {
            print(match current.scalar {
                Some(value) => value as String,
                None => "missing"
            })
            print(match current.mapName {
                Some(value) => value,
                None => "missing"
            })
            print(match current.chapterName {
                Some(value) => value,
                None => "missing"
            })
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    assert!(matches!(
        parsed.syntax().state.as_ref().unwrap().fields[0].annotation,
        Some(splitscript::compiler::ast::TypeRef::Option(_))
    ));
    let checked = splitscript::check(parsed).expect("optional pointer fields should type-check");
    let fields = &checked.syntax().state.as_ref().unwrap().fields;
    for (field, expected) in [
        (&fields[0], TypeKind::Builtin(BuiltinType::I32)),
        (&fields[1], TypeKind::Standard(StdlibTypeId::String)),
        (&fields[2], TypeKind::Standard(StdlibTypeId::String)),
    ] {
        let field_type = checked.semantics().value_type(field.id).unwrap();
        let TypeKind::Option { value, .. } = checked.semantics().types().kind(field_type) else {
            panic!("optional pointer fields must retain their Option type")
        };
        assert_eq!(checked.semantics().types().kind(*value), &expected);
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("optional scalar and decoded-string pointer reads should validate");

    let gba = r#"
        state GBA {
            marker: u8? at 0x02000000
        }

        whileAttached {
            print(match current.marker {
                Some(value) => value as String,
                None => "missing"
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(gba).unwrap())
        .expect("provider-backed optional pointer fields should type-check");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("provider-backed optional pointer reads should validate");
}

#[test]
fn option_and_result_values_use_explicit_typed_hir_conversions() {
    use splitscript::compiler::semantic::{ResolvedCall, ValueConversionKind};

    let source = r#"
        state "game.exe" {}

        enum Chapter {
            Village
        }

        fn maybe(flag: bool) -> i32? {
            if flag { return 7 }
            return None
        }

        fn maybeChapter(flag: bool) -> Chapter? {
            if flag { return Chapter.Village }
            return None
        }

        fn attempt(flag: bool) -> i32! {
            if flag { return 9 }
            return Err("attempt failed")
        }

        whileAttached {
            let optional: i32? = 5
            let chapter: Chapter? = Chapter.Village
            let empty: i32? = None
            let successful: i32! = 11
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();

    let mut saw_option_lift = false;
    let mut saw_result_lift = false;
    let mut saw_optional_null = false;
    let mut error_constructors = 0;
    for expression in checked.typed_hir().expressions() {
        if let Some(conversion) = expression.conversion {
            match conversion.kind {
                ValueConversionKind::LiftOption => saw_option_lift = true,
                ValueConversionKind::LiftResult => saw_result_lift = true,
                ValueConversionKind::NoneToOptional | ValueConversionKind::NoneToDomainNullable => {
                }
            }
            assert_ne!(conversion.source, conversion.target);
        }
        if matches!(
            expression.kind,
            splitscript::compiler::hir::TypedExpressionKind::None
        ) && matches!(
            checked.semantics().types().kind(expression.ty),
            TypeKind::Option { .. }
        ) {
            saw_optional_null = true;
        }
        if matches!(
            checked.typed_hir().call(expression.id),
            Some(ResolvedCall::ResultError { .. })
        ) {
            error_constructors += 1;
        }
    }
    assert!(saw_option_lift);
    assert!(saw_result_lift);
    assert!(saw_optional_null);
    assert_eq!(error_constructors, 2);

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        assert_eq!(
            lowered
                .expression(expression.id)
                .expect("every typed expression should have a Wasm IR plan")
                .conversion,
            expression.conversion,
            "wrapper conversion edges must be copied into Wasm IR"
        );
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option/result constructors and lifts should produce valid Wasm GC");
}

#[test]
fn wasm_ir_owns_scalar_expression_operations_catalog_calls_and_resolved_paths() {
    use splitscript::compiler::wasm_ir::{CallTarget, ExpressionKind};

    let source = r#"
        state "game.exe" {}

        fn calculate(input: i32) {
            let negated = -(input + 2)
            let text = negated as String
            if !false && negated != 0 {
                print(text)
            }
        }

        whileAttached {
            calculate(4)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw_path = false;
    let mut saw_negate = false;
    let mut saw_not = false;
    let mut saw_binary = false;
    let mut saw_cast = false;
    for typed in checked.typed_hir().expressions() {
        let expression = lowered
            .expression(typed.id)
            .expect("visible typed expressions should have Wasm IR plans");
        match &expression.kind {
            ExpressionKind::Path { root, .. } => {
                assert!(root.is_some());
                saw_path = true;
            }
            ExpressionKind::Call {
                target: CallTarget::Intrinsic { intrinsic, .. },
                ..
            } if *intrinsic == IntrinsicId::SignedNegate => saw_negate = true,
            ExpressionKind::Call {
                target: CallTarget::Intrinsic { intrinsic, .. },
                ..
            } if *intrinsic == IntrinsicId::BoolNot => saw_not = true,
            ExpressionKind::Unary { .. } => {
                panic!("checked unary operators must lower through catalog calls")
            }
            ExpressionKind::Binary { .. } => saw_binary = true,
            ExpressionKind::Cast { .. } => saw_cast = true,
            _ => {}
        }
    }
    assert!(saw_path && saw_negate && saw_not && saw_binary && saw_cast);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR scalar expression lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_gc_constructors_interpolation_and_signatures() {
    use splitscript::compiler::wasm_ir::{ExpressionKind, InterpolatedPart};

    let source = r#"
        state "game.exe" {}

        record Pair {
            left: i32,
            right: i32
        }

        enum Event {
            Empty,
            Value(i32)
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let marker = await module.scan(sig"48 8B ?? B?")
            print(marker as String)
        }

        whileAttached {
            let values = [1, 2, 3]
            let pair = Pair { right: values[1], left: values[0] }
            let event = Event.Value(pair.left)
            print(`pair {pair.left}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 6];
    for typed in checked.typed_hir().expressions() {
        let expression = lowered
            .expression(typed.id)
            .expect("visible typed expressions should have Wasm IR plans");
        match &expression.kind {
            ExpressionKind::String(_) => saw[0] = true,
            ExpressionKind::InterpolatedString(parts) => {
                assert!(parts.iter().any(|part| matches!(
                    part,
                    InterpolatedPart::Expression {
                        string_conversion_source: Some(_),
                        ..
                    }
                )));
                saw[1] = true;
            }
            ExpressionKind::Signature(_) => saw[2] = true,
            ExpressionKind::Array(elements) => {
                assert_eq!(elements.len(), 3);
                saw[3] = true;
            }
            ExpressionKind::Record { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert_ne!(fields[0].0, fields[1].0);
                saw[4] = true;
            }
            ExpressionKind::Enum { payload, .. } if payload.is_some() => saw[5] = true,
            _ => {}
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR GC constructor lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_backend_call_targets_intrinsics_and_arguments() {
    use splitscript::{
        compiler::hir::TypedExpressionKind,
        compiler::stdlib::IntrinsicId,
        compiler::wasm_ir::{CallTarget, ExpressionKind},
    };

    let source = r#"
        state "game.exe" {}

        record Counter { value: i32 }

        fn answer() -> i32 {
            return 42
        }

        fn Counter.increment() -> i32 {
            return self.value + 1
        }

        whileAttached {
            let counter = Counter { value: 4 }
            let direct = answer()
            let method = counter.increment()
            let bounded = direct.min(method)
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 5];

    for expression in checked.typed_hir().expressions() {
        let Some(expected_target) = checked.typed_hir().call(expression.id) else {
            continue;
        };
        let ExpressionKind::Call { target, arguments } = &lowered
            .expression(expression.id)
            .expect("every checked call should have a Wasm IR plan")
            .kind
        else {
            panic!("resolved calls must not remain deferred to typed HIR")
        };
        match &expression.kind {
            TypedExpressionKind::Call {
                arguments: expected_arguments,
                ..
            } => assert_eq!(arguments, expected_arguments),
            TypedExpressionKind::Binary { right, .. } => {
                assert_eq!(arguments.as_slice(), [*right])
            }
            _ => unreachable!("only calls and catalog-backed operators resolve call targets"),
        }
        match (target, expected_target) {
            (
                CallTarget::UserFunction { function },
                ResolvedCall::UserFunction {
                    function: expected,
                    type_arguments,
                    ..
                },
            ) => {
                assert_eq!(function.function, *expected);
                assert_eq!(&function.type_arguments, type_arguments);
                saw[0] = true;
            }
            (
                CallTarget::UserMethod { function, .. },
                ResolvedCall::UserMethod {
                    function: expected,
                    type_arguments,
                    ..
                },
            ) => {
                assert_eq!(function.function, *expected);
                assert_eq!(&function.type_arguments, type_arguments);
                saw[1] = true;
            }
            (
                CallTarget::Intrinsic {
                    item, intrinsic, ..
                },
                ResolvedCall::StandardLibrary { item: expected, .. },
            ) => {
                assert_eq!(item, expected);
                match intrinsic {
                    IntrinsicId::NumericMin => saw[2] = true,
                    IntrinsicId::NumericAdd => saw[4] = true,
                    _ => panic!("unexpected intrinsic call target `{intrinsic:?}`"),
                }
            }
            (
                CallTarget::ResultError { result },
                ResolvedCall::ResultError { result: expected },
            ) => {
                assert_eq!(result, expected);
                saw[3] = true;
            }
            (CallTarget::OptionSome { .. }, ResolvedCall::OptionSome { .. })
            | (CallTarget::ResultSuccess { .. }, ResolvedCall::ResultSuccess { .. }) => {}
            _ => panic!("Wasm IR call target disagrees with semantic resolution"),
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR call lowering should preserve valid codegen");
}

#[test]
fn context_free_null_and_err_request_wrapper_annotations() {
    let unit = r#"
        state "game.exe" {}
        whileAttached { let value = None }
    "#;
    splitscript::check(splitscript::parse(unit).unwrap())
        .expect("None is the context-free unit value");

    let result = r#"
        state "game.exe" {}
        whileAttached { let value = Err("failed") }
    "#;
    let errors = splitscript::check(splitscript::parse(result).unwrap())
        .expect_err("Err still needs its successful type from context");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("add a `T!` annotation"))
    );
}

#[test]
fn none_is_a_first_class_unit_across_wrappers_storage_and_async_code() {
    let source = r#"
        state "game.exe" {}

        let globalUnit = None

        record UnitBox {
            value: None
        }

        fn identity(value: None) -> None {
            return value
        }

        fn optional(flag: bool) -> None? {
            if flag {
                return Some(None)
            }
            return None
        }

        fn attempt(flag: bool) -> None! {
            if !flag {
                return Err("failed")
            }
            return None
        }

        fn propagate(value: None!) -> None! {
            value?
            return None
        }

        fn waitOne() {
            await nextTick()
        }

        onAttach {
            let awaited: None = await waitOne()
            let values: [None; 2] = [globalUnit, awaited]
            let boxed = UnitBox { value: values[0] }
            let present = optional(true)
            let absent = optional(false)
            let succeeded = propagate(attempt(true)) else None
            let same = attempt(true) == attempt(true)
            if boxed.value == identity(succeeded) && same {
                print("unit")
            }
            if present == absent {
                print("unexpected")
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("None should be a first-class unit value and type");
    let none = checked.semantics().types().id_for_core(CoreTypeId::None);
    assert_eq!(
        checked
            .semantics()
            .value_type(checked.syntax().globals[0].id),
        Some(none)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("unit values should support erased and wrapped physical representations");
}

#[test]
fn void_is_not_a_type_spelling() {
    let source = r#"
        state "game.exe" {}
        fn legacy() -> void {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("void has no compatibility alias");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type `void`"))
    );
}

#[test]
fn plain_none_calls_need_no_bottom_reference_values() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            let globalUnit = None

            fn consume(value: None) -> None {
                return value
            }

            whileAttached {
                let localUnit = globalUnit
                consume(localUnit)
            }
        "#,
    )
    .expect("unit parameters and results should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("erased unit parameters and results should preserve the Wasm ABI");

    let bottom_nulls = Parser::new(0)
        .parse_all(&wasm)
        .filter_map(
            |payload| match payload.expect("generated Wasm should parse") {
                Payload::CodeSectionEntry(body) => Some(
                    body.get_operators_reader()
                        .expect("function operators should parse")
                        .into_iter()
                        .filter(|operator| {
                            matches!(
                                operator,
                                Ok(wasmparser::Operator::RefNull {
                                    hty: wasmparser::HeapType::Abstract {
                                        ty: wasmparser::AbstractHeapType::None,
                                        ..
                                    }
                                })
                            )
                        })
                        .count(),
                ),
                _ => None,
            },
        )
        .sum::<usize>();
    assert_eq!(
        bottom_nulls, 0,
        "a plain None argument and result should be erased rather than materialized"
    );
}

#[test]
fn else_unwraps_options_and_results_with_value_or_return_fallbacks() {
    use splitscript::{
        compiler::hir::{TypedExpressionKind, TypedFallbackBranch},
        compiler::wasm_ir::{BodyOwner, ExpressionKind, FallbackBranch, LocalPurpose},
    };

    let source = r#"
        state "game.exe" {}

        fn choose(value: i32?) -> i32 {
            return value else 41
        }

        fn propagate(value: i32!) -> i32! {
            let unwrapped = value else return Err("propagated")
            return unwrapped + 1
        }

        fn nested(optional: i32?, result: i32!) -> i32 {
            return optional else result else 7
        }

        fn observe(value: i32?) {
            let unwrapped = value else return
            print(unwrapped as String)
        }

        whileAttached {
            let empty = choose(None)
            let present = choose(3)
            let failed = propagate(Err("failed"))
            let successful = propagate(5)
            observe(None)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let fallbacks = checked
        .typed_hir()
        .expressions()
        .filter(|expression| matches!(expression.kind, TypedExpressionKind::Fallback { .. }))
        .collect::<Vec<_>>();
    let fallback_count = fallbacks.len();
    assert_eq!(
        fallback_count,
        5,
        "unexpected visible fallback spans: {:?}",
        fallbacks
            .iter()
            .map(|expression| expression.span)
            .collect::<Vec<_>>()
    );

    let lowered = splitscript::lower_wasm(&checked);
    let mut branches = [false; 3];
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Fallback { value, fallback } = &expression.kind else {
            continue;
        };
        let ExpressionKind::Fallback {
            value: lowered_value,
            fallback: lowered_fallback,
        } = &lowered
            .expression(expression.id)
            .expect("fallback expression should have a Wasm IR plan")
            .kind
        else {
            panic!("fallback expressions must not remain deferred to typed HIR")
        };
        assert_eq!(lowered_value, value);
        match (fallback, lowered_fallback) {
            (TypedFallbackBranch::Value(expected), FallbackBranch::Value(actual)) => {
                assert_eq!(actual, expected);
                branches[0] = true;
            }
            (TypedFallbackBranch::Return(Some(expected)), FallbackBranch::Return(Some(actual))) => {
                assert_eq!(actual, expected);
                branches[1] = true;
            }
            (TypedFallbackBranch::Return(None), FallbackBranch::Return(None)) => {
                branches[2] = true;
            }
            _ => panic!("Wasm IR must preserve the resolved fallback branch"),
        }
    }
    assert!(branches.into_iter().all(|branch| branch));
    let planned_fallbacks = lowered
        .bodies()
        .filter(|body| match &body.owner {
            BodyOwner::Function(function) => {
                function.function.index() < checked.syntax().functions.len()
            }
            BodyOwner::Action(_) => true,
        })
        .flat_map(|body| &body.locals)
        .filter(|local| matches!(local.purpose, LocalPurpose::FallbackValue(_)))
        .count();
    assert_eq!(planned_fallbacks, fallback_count);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("value and returning fallbacks should produce valid Wasm control flow");
}

#[test]
fn question_mark_propagates_to_function_and_state_field_boundaries() {
    use splitscript::compiler::hir::TypedExpressionKind;

    let source = r#"
        state "game.exe" {
            selected = if readMemory {
                process.read<u16>(0x1000)?
            } else {
                7
            }
        }

        let readMemory = true

        fn increment(value: i32!) -> i32! {
            return value? + 1
        }

        fn rejectNegative(value: i32) -> i32! {
            if value < 0 {
                throw "negative values are not supported"
            }
            return value
        }

        whileAttached {
            let incremented = increment(3) else 0
            let rejected = rejectNegative(-1) else 0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let propagation = checked
        .typed_hir()
        .expressions()
        .filter_map(|expression| match expression.kind {
            TypedExpressionKind::Propagate { target, .. } => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(propagation.len(), 2);
    assert!(propagation.into_iter().all(|target| matches!(
        checked.semantics().types().kind(target),
        TypeKind::Result { .. }
    )));

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Propagate { value, target } = &expression.kind else {
            continue;
        };
        let splitscript::compiler::wasm_ir::ExpressionKind::Propagate {
            value: lowered_value,
            target: lowered_target,
        } = &lowered
            .expression(expression.id)
            .expect("propagation expression should have a Wasm IR plan")
            .kind
        else {
            panic!("postfix propagation must not remain deferred to typed HIR")
        };
        assert_eq!((lowered_value, lowered_target), (value, target));
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("question-mark propagation should produce valid Wasm GC control flow");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let failed: i32! = Err("failed")
            let value = failed?
        }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid).unwrap())
        .expect_err("actions are not implicit result boundaries");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("state-field boundary or a function returning `T!`")
    }));

    let invalid_throw = r#"
        state "game.exe" {}
        whileAttached { throw "actions do not return results" }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid_throw).unwrap())
        .expect_err("throw requires an enclosing failure boundary");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("function returning `T!` or an explicit catch boundary")
    }));
}

#[test]
fn else_rejects_values_that_are_not_option_or_result() {
    let source = r#"
        state "game.exe" {}
        whileAttached { let value = 1 else 2 }
    "#;
    let errors = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("plain values cannot be unwrapped");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("`else` can only unwrap `T?` or `T!`")
    }));
}

#[test]
fn declared_record_enum_and_array_layouts_are_semantic_facts() {
    let source = r#"
        state "game.exe" {}

        record Inventory {
            names: [String],
            code: u16
        }

        enum Lookup {
            Missing,
            Found(Inventory)
        }

        whileAttached {
            let lookup = Lookup.Found(Inventory {
                names: ["Moon"],
                code: 7
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let inventory = &syntax.records[0];

    let names_type = semantics
        .record_field_type(inventory.fields[0].id)
        .expect("record field layouts should expose semantic types");
    let TypeKind::Array {
        layout,
        element: names_element,
        ..
    } = semantics.types().kind(names_type)
    else {
        panic!("the names field should have a constructed array type");
    };
    assert_eq!(
        semantics.types().kind(*names_element),
        &TypeKind::Standard(StdlibTypeId::String)
    );

    let splitscript::compiler::ast::TypeRef::Array(names_array) = inventory.fields[0].ty else {
        panic!("the source annotation should reference its array layout");
    };
    assert_eq!(*layout, names_array);
    assert_eq!(
        semantics.array_element_type(names_array),
        Some(*names_element)
    );

    let code_type = semantics.record_field_type(inventory.fields[1].id).unwrap();
    assert_eq!(
        semantics.types().kind(code_type),
        &TypeKind::Builtin(BuiltinType::U16)
    );

    let enumeration = &syntax.enums[0];
    assert!(
        semantics
            .enum_variant_payloads()
            .any(|(variant, payload)| variant == enumeration.variants[0].id && payload.is_none())
    );
    let found_payload = semantics
        .enum_variant_payload(enumeration.variants[1].id)
        .expect("payload variants should expose their semantic payload type");
    assert_eq!(
        semantics.types().kind(found_payload),
        &TypeKind::Record(inventory.id)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration layouts should drive valid Wasm GC types");
}

#[test]
fn replaceable_deep_pointer_flags_do_not_require_background_watcher_registration() {
    let source = r#"
        let teleporterLoadingPath: MemoryPath? = None
        let teleporterTransitionPath: MemoryPath? = None
        let loadingScreenPath: MemoryPath? = None

        state "AER.exe" {
            teleporterLoading: bool = readFlag(teleporterLoadingPath);
            teleporterTransition: bool = readFlag(teleporterTransitionPath);
            loadingScreen: bool = readFlag(loadingScreenPath);
        }

        tickRate {
            attached: 58,
            detached: 2,
        }

        fn readFlag(path: MemoryPath?) -> bool {
            let resolvedPath = path else return false
            let address = resolvedPath.resolve() else return false
            return process.read<bool>(address) else false
        }

        onAttach {
            let mono = await process.module("mono.dll")
            let teleporter = mono.address.offset(0x1f6964)
            teleporterLoadingPath = teleporter.memoryPath(
                [0x30, 0xd5c],
                0xe90 + 0x5a,
                PointerSize.Bit32,
            )
            teleporterTransitionPath = teleporter.memoryPath(
                [0x30, 0xd5c],
                0xe90 + 0x59,
                PointerSize.Bit32,
            )
            loadingScreenPath = mono.address.offset(0x1f696c).memoryPath(
                [0x80, 0x90, 0x40, 0x1c, 0x4, 0xc],
                0x4,
                PointerSize.Bit32,
            )
        }

        isLoading {
            return current.teleporterLoading
                || current.teleporterTransition
                || current.loadingScreen
        }
    "#;

    splitscript::compile(source).expect(
        "attachment-owned paths should re-resolve replaceable objects on every state update",
    );
}

#[test]
fn bounded_settings_and_static_singleton_paths_do_not_require_runtime_registration() {
    let source = r#"
        let levelPath: MemoryPath? = None
        let clickPath: MemoryPath? = None
        let mainMenuPath: MemoryPath? = None
        let pausePath: MemoryPath? = None

        state "Bzzzt.exe" {
            level: i32 = readLevel(levelPath);
            click: bool = readFlag(clickPath);
            mainMenu: bool = readFlag(mainMenuPath);
            pause: bool = readFlag(pausePath);
        }

        settings {
            "Split by Level" => levels key "levels": true,
            "Levels" {
                for level in 1..=12 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 13" => level13 key "13": true,
                for level in 14..=25 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 26" => level26 key "26": true,
                for level in 27..=38 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 39" => level39 key "39": true,
                for level in 40..=51 {
                    `Level {level}` key `{level}`: false,
                },
            },
        }

        fn readLevel(path: MemoryPath?) -> i32! {
            let initialized = path else return Err("path not initialized")
            let address = initialized.resolve()?
            return process.read<i32>(address)
        }

        fn readFlag(path: MemoryPath?) -> bool! {
            let initialized = path else return Err("path not initialized")
            let address = initialized.resolve()?
            return process.read<bool>(address)
        }

        onAttach {
            let mono = await Unity.mono(MonoVersion.V2)
            let image = await mono.image("Assembly-CSharp")
            let main = await image.class("Main")
            let instance = await main.staticFieldPath("instance")
            levelPath = instance.dereference((await main.field("ActualLevelId")) as i64)
            clickPath = instance.dereference((await main.field("ButtonClicked")) as i64)
            mainMenuPath = instance.dereference((await main.field("IsInMainMenu")) as i64)
            pausePath = instance.dereference((await main.field("Pause")) as i64)
        }

        start {
            return current.mainMenu != old.mainMenu && !current.mainMenu
        }

        split {
            return current.click
                && current.click != old.click
                && !current.pause
                && (current.level == 52
                    || (settings.levels && settings.enabled(current.level as String)))
        }

        reset {
            return current.mainMenu != old.mainMenu && current.mainMenu
        }
    "#;

    splitscript::compile(source).expect(
        "finite setting families and static singleton paths should cover bounded helper setup",
    );
}

#[test]
fn catalog_queries_expose_generic_calls_effects_and_docs_for_editor_tooling() {
    let library = StandardLibrary::new();
    let process_type = library
        .type_by_name("Process")
        .expect("Process should be an explicit standard-library type");
    assert_eq!(process_type.id, StdlibTypeId::Process);
    assert!(
        process_type
            .documentation
            .summary
            .contains("attached game process")
    );
    let unity_image = library
        .type_by_name("UnityImage")
        .expect("UnityImage should be a nominal library declaration");
    assert_eq!(
        unity_image.id,
        splitscript::compiler::stdlib::StdlibTypeId::UnityImage
    );
    assert_eq!(
        library
            .public_field(unity_image.id, "address")
            .expect("UnityImage.address should be declared")
            .ty,
        splitscript::compiler::stdlib::TypeRef::Core(
            splitscript::compiler::stdlib::CoreTypeId::Address
        )
    );
    assert!(
        library.public_field(unity_image.id, "module").is_none(),
        "runtime ownership slots must not leak into the public member surface"
    );
    let read = library
        .method_candidates("read")
        .into_iter()
        .next()
        .expect("generic process reads should resolve through the catalog");
    assert_eq!(read.item.id, StdlibItemId::ProcessRead);
    assert!(library.method_candidates("get").is_empty());
    let min = library.method_candidates("min");
    assert_eq!(min.len(), 1);
    assert_eq!(min[0].item.id, StdlibItemId::NumericMin);
    assert_eq!(min[0].item.signature.type_parameters[0].name, "T");
    assert_eq!(
        min[0].item.signature.type_parameters[0].constraints,
        [splitscript::compiler::stdlib::StdlibCapabilityId::Numeric]
    );
    assert!(library.method_candidates("missing").is_empty());
    assert_eq!(
        library.render_signature(read.item.id),
        "Process.read<T>(address: address) -> T! where T: MemoryReadable"
    );
    let managed_string = library
        .method_candidates("readManagedString")
        .into_iter()
        .next()
        .expect("managed strings should be a Process method");
    assert_eq!(
        managed_string.item.id,
        StdlibItemId::ProcessReadManagedString
    );
    assert_eq!(
        library.render_signature(managed_string.item.id),
        "Process.readManagedString(address: address, maxUtf16Units: u32) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerState),
        "timer.state() -> TimerState"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerCurrentSplitIndex),
        "timer.currentSplitIndex() -> u64?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerSegmentWasSplit),
        "timer.segmentWasSplit(index: u64) -> bool?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerSkipSplit),
        "timer.skipSplit() -> None"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerUndoSplit),
        "timer.undoSplit() -> None"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessPath),
        "Process.path() -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::RuntimeOperatingSystem),
        "runtime.operatingSystem() -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::RuntimeArchitecture),
        "runtime.architecture() -> String!"
    );
    let next_tick = library
        .item_by_name("nextTick")
        .expect("nextTick should be catalog-backed");
    assert_eq!(
        library.render_signature(next_tick.id),
        "nextTick() -> async None"
    );
    assert_eq!(
        library.operation_semantics(next_tick.id).suspension,
        SuspensionKind::Suspends
    );
    assert_eq!(
        library.operation_semantics(next_tick.id).cancellation,
        CancellationKind::ProcessClose
    );
    let process_closed = library
        .item_by_name("Process.closed")
        .expect("Process.closed should be catalog-backed");
    assert_eq!(
        library.render_signature(process_closed.id),
        "Process.closed() -> async Never"
    );
    assert_eq!(
        library.render_operation_semantics(process_closed.id),
        "available in onAttach; suspends; requires an attached process; cancels when the process closes"
    );

    let field_any = library
        .item_by_name("UnityClass.fieldAny")
        .expect("UnityClass.fieldAny should have a documented catalog entry");
    assert_eq!(
        library.operation_metadata(field_any.id).availability,
        Availability::OnAttach
    );
    assert!(library.effects(field_any.id).contains(&Effect::Suspends));
    assert!(
        library
            .effects(field_any.id)
            .contains(&Effect::RequiresAttachedProcess)
    );
    assert!(
        library
            .effects(field_any.id)
            .contains(&Effect::CancelsOnProcessClose)
    );
    let operation = library.operation_semantics(field_any.id);
    assert_eq!(operation.availability, Availability::OnAttach);
    assert_eq!(operation.suspension, SuspensionKind::Suspends);
    assert!(operation.requires_attached_process);
    assert_eq!(operation.cancellation, CancellationKind::ProcessClose);
    assert_eq!(
        library.render_operation_semantics(field_any.id),
        "available in onAttach; suspends; requires an attached process; cancels when the process closes"
    );
    assert_eq!(
        library.operation_semantics(read.item.id).suspension,
        SuspensionKind::None
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessFollow),
        "Process.follow(base: address, offsets: [i64]) -> address!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessReadRelative32),
        "Process.readRelative32(address: address) -> address!"
    );
    assert!(!field_any.documentation.summary.is_empty());
    assert_eq!(
        library.render_signature(field_any.id),
        "UnityClass.fieldAny(names: [String]) -> async UnityField"
    );
}

#[test]
fn process_closed_is_only_available_while_attaching() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                await process.closed()
            }
        "#,
    )
    .expect_err("waiting out an attachment should only be valid in onAttach");
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.message == "`Process.closed` must be awaited in `onAttach`"
        ),
        "{diagnostics:#?}"
    );
}

#[test]
fn process_operations_reject_detach_lifecycle_use() {
    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetach {
                let value = process.read<i32>(0x1000) else 0
                print(value as String)
            }
        "#,
    )
    .expect_err("process access should not be available before attachment");
    assert!(errors.iter().any(|error| {
        error.message
            == "`Process.read` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn detach_does_not_expose_a_closed_process_or_uninitialized_snapshots() {
    let process_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetach { process.read<i32>(0x1000) }
        "#,
    )
    .expect_err("a closed process handle must not remain usable");
    assert!(process_errors.iter().any(|diagnostic| {
        diagnostic.message
            == "`Process.read` requires an attached process and is unavailable in `onDetach`"
    }));

    let snapshot_errors = splitscript::compile(
        r#"
            state "game.exe" { level: u32 at 0x100 }
            onDetach { print(current.level) }
        "#,
    )
    .expect_err("a process can close before its first snapshot commits");
    assert!(snapshot_errors.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state snapshots are not guaranteed to exist in `onDetach`")
    }));
}

#[test]
fn setup_is_process_independent_and_cannot_suspend_or_read_snapshots() {
    let process_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            setup { process.read<i32>(0x1000) }
        "#,
    )
    .expect_err("setup must not access the process provider");
    assert!(process_errors.iter().any(|error| {
        error.message == "`Process.read` requires an attached process and is unavailable in `setup`"
    }));

    let snapshot_errors = splitscript::compile(
        r#"
            state "game.exe" { level: i32 at 0x1000 }
            setup { print(current.level) }
        "#,
    )
    .expect_err("setup runs before state snapshots exist");
    assert!(snapshot_errors.iter().any(|error| {
        error
            .message
            .contains("state snapshots are not available during `setup`")
    }));

    let suspension_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            setup { await nextTick() }
        "#,
    )
    .expect_err("setup must finish synchronously during module start");
    assert!(
        suspension_errors
            .iter()
            .any(|error| error.message == "`await` is not available in this synchronous body")
    );
}

#[test]
fn on_state_ready_has_the_attached_process_and_committed_snapshots_but_cannot_suspend() {
    splitscript::compile(
        r#"
            state "game.exe" { level: u32 at 0x100 }
            onStateReady {
                print(process.name())
                print(old.level)
                print(current.level)
            }
        "#,
    )
    .expect("post-snapshot initialization should expose process and state values");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onStateReady { await nextTick() }
        "#,
    )
    .expect_err("post-snapshot initialization must complete synchronously");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "`await` is not available in this synchronous body"
    }));
}

#[test]
fn call_result_fields_parse_before_detached_effects_are_checked() {
    let source = r#"
        state "game.exe" {}

        record LevelTimeParts {
            minutes: f32,
            seconds: f32,
            hundredths: f32
        }

        fn baz() {
            return process.read(0x200) else process.read(0x100) else LevelTimeParts {
                minutes: 0.0,
                seconds: 0.0,
                hundredths: 0.0
            }
        }

        onDetach {
            let minutes = baz().minutes
        }
    "#;

    splitscript::parse(source).expect("a field on a call result should parse");
    let attached = source.replace("onDetach", "whileAttached");
    let wasm = splitscript::compile(&attached)
        .expect("a call-result field should type-check and lower while attached");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("a call-result field should produce valid Wasm");
    let diagnostics = splitscript::compile(source)
        .expect_err("the process-dependent helper should still be rejected while detached");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`baz` requires an attached process and is unavailable in `onDetach`"
        }),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != splitscript::DiagnosticCode::Syntax)
    );
}

#[test]
fn immediate_process_failures_are_results_and_not_awaitable_intrinsics() {
    let source = include_str!("../fallible_process_operations.split");
    let wasm = splitscript::compile(source).expect("fallible process operations should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("process failure sentinels should lower to valid Result values");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let value = await process.read<i32>(0x1000)
            }
        "#,
    )
    .expect_err("immediate Result operations should use retry rather than await");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`await` expects an async value")
    }));
}

#[test]
fn attached_process_requirements_propagate_through_function_call_graphs() {
    let safe_source = r#"
        state "game.exe" {}

        record Reader {
            address: address
        }

        fn Reader.readValue() {
            return process.read<i32>(self.address) else 0
        }

        fn relay(reader: Reader) {
            return reader.readValue()
        }

        fn recursiveRelay(reader: Reader, recurse) {
            if recurse {
                return recursiveRelay(reader, false)
            }
            return relay(reader)
        }

        whileAttached {
            let reader = Reader { address: 0x1000 }
            print(recursiveRelay(reader, true) as String)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(safe_source).unwrap()))
        .expect("process-dependent helpers should be callable while attached");
    for name in ["readValue", "relay", "recursiveRelay"] {
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("test helper should exist");
        assert!(
            checked
                .effects()
                .function(function.id)
                .requires_attached_process,
            "{name} should inherit its process requirement"
        );
        let effects = checked.effects().function(function.id).effects;
        assert!(
            effects.contains(&Effect::ReadsProcess),
            "{name} should inherit its process-read effect"
        );
        assert!(effects.contains(&Effect::RequiresAttachedProcess));
    }

    let detached_source = safe_source.replace("whileAttached", "onDetach");
    let errors = splitscript::compile(&detached_source)
        .expect_err("a transitive process dependency should be rejected while detached");
    assert!(errors.iter().any(|error| {
        error.message
            == "`recursiveRelay` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn state_snapshot_requirements_propagate_through_function_call_graphs() {
    let source = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        fn enteredLevel(level) {
            return old.level != level && current.level == level
        }

        fn relay(level) {
            return enteredLevel(level)
        }

        split {
            return relay(7u32)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("snapshot-dependent helpers should be callable from timer actions");
    for name in ["enteredLevel", "relay"] {
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("test helper should exist");
        let operation = checked.effects().function(function.id);
        assert!(operation.requires_state_snapshots, "{name}");
        assert!(operation.effects.contains(&Effect::RequiresStateSnapshots));
    }

    let wasm = splitscript::compile(source).expect("snapshot helpers should lower to Wasm");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("snapshot helpers should produce valid Wasm GC");
}

#[test]
fn snapshot_dependent_helpers_are_rejected_without_committed_snapshots() {
    let declarations = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        fn changed() {
            return old.level != current.level
        }
    "#;
    for action in ["setup", "onAttach", "onDetach"] {
        let source = format!("{declarations}\n{action} {{ print(changed()) }}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("a snapshot-dependent helper needs committed snapshots");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message
                    == format!(
                        "`changed` requires state snapshots and is unavailable in `{action}`"
                    )
            }),
            "{action}: {diagnostics:#?}"
        );
    }

    let state_source = r#"
        state "game.exe" {
            level: u32 at 0x100;
            changed = didChange()
        }

        fn didChange() {
            return old.level != current.level
        }
    "#;
    let diagnostics = splitscript::compile(state_source)
        .expect_err("state polling must not call snapshot-dependent helpers");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "`didChange` requires state snapshots and is unavailable in a state field expression"
    }), "{diagnostics:#?}");
}

#[test]
fn explicit_generic_calls_accept_named_and_constructed_types() {
    let source = r#"
        state "game.exe" {}

        record Header {
            marker: u32
        }

        whileAttached {
            let header = process.read<Header>(0)
            let bytes = process.read<[u8; 4]>(4)
            print<u32>((header else Header { marker: 0 }).marker)
            print<u8>((bytes else [0, 0, 0, 0])[0])
        }
    "#;
    splitscript::compile(source)
        .expect("explicit generic calls should accept every MemoryReadable source type");

    for rejected_type in ["String", "char"] {
        let source = format!(
            "state \"game.exe\" {{}}\nwhileAttached {{ let value = process.read<{rejected_type}>(0) }}"
        );
        let errors = splitscript::compile(&source)
            .expect_err("generic constraints still apply to explicit type arguments");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("MemoryReadable")),
            "{errors:#?}"
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value = process.read.u32(0) }
        "#,
    )
    .expect_err("the former dotted type-selector syntax must not remain available");
}
