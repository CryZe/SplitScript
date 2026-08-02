//! failure semantics integration tests.

use super::*;

#[test]
fn bounded_utf8_reads_are_fallible_strings_and_state_sugar_infers_string() {
    use splitscript::compiler::{
        ast::{StateMemoryDecoder, StateSource},
        stdlib::StdlibTypeId,
        types::TypeKind,
    };

    let source = r#"
        state "game.exe" {
            mapName at "game.dll", 0x100, 0x20 as utf8(32)
        }

        whileAttached {
            let direct: String! = process.readUtf8(0x2000, 32)
            print(current.mapName)
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
    let field_type = checked.semantics().value_type(field.id).unwrap();
    assert_eq!(
        checked.semantics().types().kind(field_type),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("bounded UTF-8 state and expression reads should validate");

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
fn option_and_result_values_use_explicit_typed_hir_conversions() {
    use splitscript::compiler::semantic::{ResolvedCall, ValueConversionKind};

    let source = r#"
        state "game.exe" {}

        fn maybe(flag: bool) -> i32? {
            if flag { return 7 }
            return None
        }

        fn attempt(flag: bool) -> i32! {
            if flag { return 9 }
            return Err("attempt failed")
        }

        whileAttached {
            let optional: i32? = 5
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
fn wasm_ir_owns_scalar_expression_operations_and_resolved_paths() {
    use splitscript::compiler::wasm_ir::ExpressionKind;

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
    let mut saw_unary = false;
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
            ExpressionKind::Unary { .. } => saw_unary = true,
            ExpressionKind::Binary { .. } => saw_binary = true,
            ExpressionKind::Cast { .. } => saw_cast = true,
            _ => {}
        }
    }
    assert!(saw_path && saw_unary && saw_binary && saw_cast);

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
            left: i32
            right: i32
        }

        enum Event {
            Empty
            Value(i32)
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let marker = await module.scan(sig"48 8B ?? B?")
            print(marker as String)
        }

        whileAttached {
            let values = [1, 2, 3]
            let pair = Pair { right: values.get(1), left: values.get(0) }
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
    for (initializer, expected_message) in [
        ("None", "add a `T?` annotation"),
        ("Err(\"failed\")", "add a `T!` annotation"),
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                whileAttached {{ let value = {initializer} }}
            "#
        );
        let errors = splitscript::check(splitscript::parse(&source).unwrap())
            .expect_err("a wrapper constructor needs its contained type from context");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains(expected_message)),
            "missing focused diagnostic in {errors:#?}"
        );
    }
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
            names: [String]
            code: u16
        }

        enum Lookup {
            Missing
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
    let get = library.method_candidates("get");
    assert_eq!(get.len(), 1);
    assert_eq!(get[0].item.id, StdlibItemId::ArrayGet);
    assert_eq!(
        get[0].receiver(),
        Some(splitscript::compiler::stdlib::TypeRef::Application {
            constructor: splitscript::compiler::stdlib::StdlibTypeConstructorId::Array,
            arguments: &[splitscript::compiler::stdlib::TypeRef::Parameter("T")],
        })
    );
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
    let next_tick = library
        .item_by_name("nextTick")
        .expect("nextTick should be catalog-backed");
    assert_eq!(
        library.render_signature(next_tick.id),
        "nextTick() -> async void"
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
        "Process.closed() -> async void"
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
        "Process.follow(base: address, offsets: [u64]) -> address!"
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
fn process_operations_reject_detached_lifecycle_use() {
    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetached {
                let value = process.read<i32>(0x1000) else 0
                print(value as String)
            }
        "#,
    )
    .expect_err("process access should not be available before attachment");
    assert!(errors.iter().any(|error| {
        error.message
            == "`Process.read` requires an attached process and is unavailable in `onDetached`"
    }));
}

#[test]
fn call_result_fields_parse_before_detached_effects_are_checked() {
    let source = r#"
        state "game.exe" {}

        record LevelTimeParts {
            minutes: f32
            seconds: f32
            hundredths: f32
        }

        fn baz() {
            return process.read(0x200) else process.read(0x100) else LevelTimeParts {
                minutes: 0.0,
                seconds: 0.0,
                hundredths: 0.0
            }
        }

        onDetached {
            let minutes = baz().minutes
        }
    "#;

    splitscript::parse(source).expect("a field on a call result should parse");
    let attached = source.replace("onDetached", "whileAttached");
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
                == "`baz` requires an attached process and is unavailable in `onDetached`"
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

    let detached_source = safe_source.replace("whileAttached", "onDetached");
    let errors = splitscript::compile(&detached_source)
        .expect_err("a transitive process dependency should be rejected while detached");
    assert!(errors.iter().any(|error| {
        error.message
            == "`recursiveRelay` requires an attached process and is unavailable in `onDetached`"
    }));
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
            print<u8>((bytes else [0, 0, 0, 0]).get(0))
        }
    "#;
    splitscript::compile(source)
        .expect("explicit generic calls should accept every MemoryReadable source type");

    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value = process.read<String>(0) }
        "#,
    )
    .expect_err("generic constraints still apply to explicit type arguments");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("MemoryReadable")),
        "{errors:#?}"
    );

    splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value = process.read.u32(0) }
        "#,
    )
    .expect_err("the former dotted type-selector syntax must not remain available");
}
