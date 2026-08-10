//! catalogs types integration tests.

use super::*;

#[test]
fn source_defined_library_bodies_compile_without_leaking_hidden_declarations() {
    let library = StandardLibrary::new();
    for item in [
        StdlibItemId::DurationFromFrames,
        StdlibItemId::DurationFromMilliseconds,
        StdlibItemId::DurationFromParts,
        StdlibItemId::DurationFromSeconds,
        StdlibItemId::DurationFromNanoseconds,
        StdlibItemId::DurationFromWholeMilliseconds,
        StdlibItemId::DurationFromWholeSeconds,
        StdlibItemId::DurationWholeSeconds,
        StdlibItemId::DurationSubsecondNanoseconds,
        StdlibItemId::DurationTotalSeconds,
        StdlibItemId::DurationTotalMilliseconds,
        StdlibItemId::DurationAdd,
        StdlibItemId::DurationSubtract,
        StdlibItemId::DurationLessThan,
        StdlibItemId::DurationLessThanOrEqual,
        StdlibItemId::DurationGreaterThan,
        StdlibItemId::DurationGreaterThanOrEqual,
        StdlibItemId::DurationZero,
        StdlibItemId::InstantAdd,
        StdlibItemId::InstantDurationSince,
        StdlibItemId::InstantElapsed,
        StdlibItemId::InstantHasElapsed,
        StdlibItemId::FloatRoundTo,
        StdlibItemId::FloatIsNaN,
        StdlibItemId::FloatIsFinite,
        StdlibItemId::NumericClamp,
        StdlibItemId::ArrayIsEmpty,
        StdlibItemId::ArrayContains,
        StdlibItemId::ArrayIndexOf,
        StdlibItemId::ResultToOption,
        StdlibItemId::AddressOffset,
        StdlibItemId::UnityIl2Cpp,
        StdlibItemId::GbaEmulatorDiscover,
    ] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. }
        ));
    }

    let source = r#"
        state "game.exe" {}
        gameTime {
            return Duration.fromFrames(125, 60)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("a source-defined library function should use the ordinary checker");
    assert!(checked.syntax().functions.is_empty());
    assert_eq!(checked.typed_hir().function_bodies().count(), 0);
    assert_eq!(checked.semantics().calls().count(), 1);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("a call through a source-defined library body should produce valid Wasm");
}

#[test]
fn result_to_option_is_source_defined_and_preserves_generic_inference() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::ResultToOption),
        "T!.toOption() -> T?"
    );
    assert!(matches!(
        library.item(StdlibItemId::ResultToOption).implementation,
        Implementation::LibraryBody { .. }
    ));

    let source = r#"
        state "game.exe" {
            optional: i32? = process.read<i32>(0x1000).toOption()
        }

        whileAttached {
            let text = match current.optional {
                Some(value) => value as String,
                None => "missing",
            }
            print(text)
        }
    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("source-defined Result-to-Option conversion should produce valid Wasm GC");
}

#[test]
fn must_use_obligations_are_catalog_owned() {
    let library = StandardLibrary::new();
    assert!(
        library
            .type_constructor(StdlibTypeConstructorId::Option)
            .must_use
            .expect("Option values should carry a use obligation")
            .contains("inspected")
    );
    assert!(
        library
            .type_constructor(StdlibTypeConstructorId::Result)
            .must_use
            .expect("Result values should carry a use obligation")
            .contains("failures")
    );
    assert!(
        library
            .item(StdlibItemId::StringReplaceAll)
            .must_use
            .expect("immutable replacement should explain its returned value")
            .contains("immutable")
    );
    assert!(
        library
            .item(StdlibItemId::StringToAsciiLowerCase)
            .must_use
            .expect("immutable case conversion should explain its returned value")
            .contains("immutable")
    );
    assert!(
        library
            .must_use(StdlibItemId::F32FromBits)
            .expect("float construction should carry a use obligation")
            .contains("constructed")
    );
    assert!(
        library
            .must_use(StdlibItemId::ProcessRead)
            .expect("non-mutating reads should receive the catalog default")
            .contains("only produces")
    );
    assert_eq!(library.must_use(StdlibItemId::SetInsert), None);
}

#[test]
fn duration_convenience_constructors_are_source_defined() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::DurationZero),
        "Duration.zero() -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromMilliseconds),
        "Duration.fromMilliseconds<T>(milliseconds: T) -> Duration where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromMinutes),
        "Duration.fromMinutes<T>(minutes: T) -> Duration where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromHours),
        "Duration.fromHours<T>(hours: T) -> Duration where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromDays),
        "Duration.fromDays<T>(days: T) -> Duration where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromWholeMilliseconds),
        "Duration.fromWholeMilliseconds(milliseconds: i64) -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromWholeSeconds),
        "Duration.fromWholeSeconds(seconds: i64) -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromNanoseconds),
        "Duration.fromNanoseconds(nanoseconds: i64) -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationAdd),
        "Duration.add(other: Duration) -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationTotalSeconds),
        "Duration.totalSeconds() -> f64"
    );
    assert!(library.type_has_capability(
        splitscript::compiler::stdlib::StdlibTypeId::Duration,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
    ));

    for expression in [
        "Duration.zero()",
        "Duration.fromWholeSeconds(-12)",
        "Duration.fromWholeMilliseconds(1_500)",
        "Duration.fromNanoseconds(-1_250_000_100)",
        "Duration.fromMilliseconds(1_500.25)",
        "Duration.fromMilliseconds(1_500.25 as f32)",
        "Duration.fromMinutes(1.5)",
        "Duration.fromMinutes(1.5 as f32)",
        "Duration.fromHours(1.25)",
        "Duration.fromDays(1.5)",
        "Duration.fromSeconds(1.25)",
        "Duration.fromSeconds(1.25 as f32)",
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                gameTime {{
                    return {expression}
                }}
            "#
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&splitscript::compile(&source).unwrap())
            .expect("source-defined duration convenience constructors should produce valid Wasm");
    }
}

#[test]
fn instant_uses_one_monotonic_host_boundary_and_source_defined_arithmetic() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::InstantNow),
        "Instant.now() -> Instant"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::InstantAdd),
        "Instant.add(duration: Duration) -> Instant"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::InstantDurationSince),
        "Instant.durationSince(earlier: Instant) -> Duration"
    );
    assert!(matches!(
        library.item(StdlibItemId::InstantNow).implementation,
        Implementation::Intrinsic(IntrinsicId::InstantNow)
    ));
    for item in [
        StdlibItemId::InstantAdd,
        StdlibItemId::InstantDurationSince,
        StdlibItemId::InstantElapsed,
        StdlibItemId::InstantHasElapsed,
    ] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. }
        ));
    }
    let now_effects = library.operation_metadata(StdlibItemId::InstantNow).effects;
    assert!(now_effects.contains(&splitscript::compiler::stdlib::Effect::ReadsRuntime));
    assert!(now_effects.contains(&splitscript::compiler::stdlib::Effect::Allocates));

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let startedAt = Instant.now()
            let deadline: Instant = startedAt
            deadline += Duration.fromMilliseconds(250)
            if startedAt.hasElapsed(Duration.zero()) && deadline != startedAt {
                print("ready")
            }
        }
    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("monotonic instants and their source-defined methods should produce valid Wasm");
}

#[test]
fn binary_syntax_resolves_through_catalog_declared_methods() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.item(StdlibItemId::InstantAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationSubtract).binary_operator,
        Some(StandardBinaryOperator::Subtract)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationLessThan).binary_operator,
        Some(StandardBinaryOperator::LessThan)
    );
    assert_eq!(
        library
            .item(StdlibItemId::DurationGreaterThanOrEqual)
            .binary_operator,
        Some(StandardBinaryOperator::GreaterThanOrEqual)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericMultiply).binary_operator,
        Some(StandardBinaryOperator::Multiply)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericDivide).binary_operator,
        Some(StandardBinaryOperator::Divide)
    );
    assert_eq!(
        library.item(StdlibItemId::IntegerRemainder).binary_operator,
        Some(StandardBinaryOperator::Remainder)
    );
    for (item, operator) in [
        (StdlibItemId::IntegerBitOr, StandardBinaryOperator::BitOr),
        (StdlibItemId::IntegerBitXor, StandardBinaryOperator::BitXor),
        (StdlibItemId::IntegerBitAnd, StandardBinaryOperator::BitAnd),
        (
            StdlibItemId::IntegerShiftLeft,
            StandardBinaryOperator::ShiftLeft,
        ),
        (
            StdlibItemId::IntegerShiftRight,
            StandardBinaryOperator::ShiftRight,
        ),
    ] {
        assert_eq!(library.item(item).binary_operator, Some(operator));
    }
    assert_eq!(
        library.item(StdlibItemId::EquatableEquals).binary_operator,
        Some(StandardBinaryOperator::Equal)
    );
    assert_eq!(
        library
            .item(StdlibItemId::EquatableNotEquals)
            .binary_operator,
        Some(StandardBinaryOperator::NotEqual)
    );

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let number = 20 + 22
            let product = number * 2
            let quotient = product / 3
            let remainder = quotient % 5
            let direct = 6.multiply(7).divide(3).remainder(5)
            let bits = ((0x10 | 0x04) ^ 0x01) & 0x1f
            let shifted = bits << 2 >> 1
            let directBits = 1u32.shiftLeft(5).bitOr(3).shiftRight(1).bitXor(2).bitAnd(0xff)
            let same = number.equals(42)
            let different = number.notEquals(41)
            let unitEqual = None.equals(None)
            let duration = Duration.fromSeconds(1.5) + Duration.fromSeconds(0.5)
            let difference = duration - Duration.fromWholeSeconds(1)
            let ordered = difference < duration && duration >= difference
            print(number)
            print(remainder)
            print(direct)
            print(shifted)
            print(directBits)
            print(same && different)
            print(unitEqual)
            print(difference.wholeSeconds())
            if ordered {
                print("ordered")
            }
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("catalog-declared operators should type-check as ordinary calls");
    let resolved_items = checked
        .semantics()
        .calls()
        .filter_map(|(_, call)| match call {
            ResolvedCall::StandardLibrary { item, .. } => Some(*item),
            _ => None,
        })
        .collect::<Vec<_>>();
    for expected in [
        StdlibItemId::NumericAdd,
        StdlibItemId::NumericMultiply,
        StdlibItemId::NumericDivide,
        StdlibItemId::IntegerRemainder,
        StdlibItemId::IntegerBitOr,
        StdlibItemId::IntegerBitXor,
        StdlibItemId::IntegerBitAnd,
        StdlibItemId::IntegerShiftLeft,
        StdlibItemId::IntegerShiftRight,
        StdlibItemId::EquatableEquals,
        StdlibItemId::EquatableNotEquals,
        StdlibItemId::DurationAdd,
        StdlibItemId::DurationSubtract,
        StdlibItemId::DurationLessThan,
        StdlibItemId::DurationGreaterThanOrEqual,
    ] {
        assert!(
            resolved_items.contains(&expected),
            "operator syntax should resolve to {expected:?}"
        );
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("catalog-declared operators should lower to valid Wasm");
}

#[test]
fn unary_syntax_resolves_through_catalog_declared_methods() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.item(StdlibItemId::BoolNot).unary_operator,
        Some(StandardUnaryOperator::Not)
    );
    assert_eq!(
        library.item(StdlibItemId::SignedNegate).unary_operator,
        Some(StandardUnaryOperator::Negate)
    );
    assert_eq!(
        library.item(StdlibItemId::IntegerBitNot).unary_operator,
        Some(StandardUnaryOperator::Not)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::BoolNot),
        "bool.not() -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SignedNegate),
        "T.negate() -> T where T: Signed"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::IntegerBitNot),
        "T.bitNot() -> T where T: Integer"
    );

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let offset: i32 = -42
            let reverse = offset.negate()
            let ready = !false
            let disabled = ready.not()
            let byte: u8 = !1
            let original = byte.bitNot()
            let full: u32 = !0u32
            let inferred = !1
            print(`{offset}:{reverse}:{ready}:{disabled}:{byte}:{original}:{full}:{inferred}`)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("catalog-declared unary operators should type-check as ordinary calls");
    let resolved_items = checked
        .semantics()
        .calls()
        .filter_map(|(_, call)| match call {
            ResolvedCall::StandardLibrary { item, .. } => Some(*item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::SignedNegate)
            .count(),
        2
    );
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::BoolNot)
            .count(),
        2
    );
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::IntegerBitNot)
            .count(),
        4
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("catalog-declared unary operators should lower to valid Wasm");
}

#[test]
fn compound_assignment_resolves_through_the_same_catalog_method() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let elapsed = Duration.zero()
            elapsed += Duration.fromWholeSeconds(1)
            print(elapsed.wholeSeconds())
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("Duration compound assignment should type-check through its operator binding");
    let assignment = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .unwrap()
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            splitscript::compiler::hir::TypedStatementKind::Assign { assignment, .. } => {
                Some(assignment)
            }
            _ => None,
        })
        .expect("fixture should contain a compound assignment");
    assert!(matches!(
        assignment.operator,
        Some(ResolvedCall::StandardLibrary {
            item: StdlibItemId::DurationAdd,
            ..
        })
    ));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("source-defined compound operators should produce valid Wasm");
}

#[test]
fn standard_library_bodies_can_read_only_their_own_private_representation() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let duration = Duration.zero()
            let leaked = duration.seconds
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("user source must not read private standard-library representation fields");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == splitscript::DiagnosticCode::Type
            && diagnostic
                .message
                .contains("Duration has no field `seconds`")
    }));
}

#[test]
fn generic_library_bodies_emit_only_reachable_concrete_instances() {
    let defined_functions = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(
                |payload| match payload.expect("generated Wasm should parse") {
                    Payload::FunctionSection(section) => Some(section.count()),
                    _ => None,
                },
            )
            .expect("generated Wasm has a function section")
    };
    let baseline = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let signed: i32 = 10
                let wide: f64 = 10.0
            }
        "#,
    )
    .expect("baseline should compile");
    let specialized = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let signed: i32 = 10
                let wide: f64 = 10.0
                let boundedSigned = signed.clamp(0, 7)
                let boundedWide = wide.clamp(0.0, 7.0)
            }
        "#,
    )
    .expect("generic source-body instances should compile");

    assert_eq!(
        defined_functions(&specialized),
        defined_functions(&baseline) + 2,
        "only the i32 and f64 clamp instances should survive reachability"
    );
}

#[test]
fn generic_array_library_bodies_share_demand_driven_specialization() {
    let defined_functions = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(
                |payload| match payload.expect("generated Wasm should parse") {
                    Payload::FunctionSection(section) => Some(section.count()),
                    _ => None,
                },
            )
            .expect("generated Wasm has a function section")
    };
    let baseline = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let numbers: [i32] = []
                let names: [String] = []
                let numberCount = numbers.length()
                let nameCount = names.length()
            }
        "#,
    )
    .expect("the intrinsic baseline should compile");
    let specialized = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let numbers: [i32] = []
                let names: [String] = []
                let noNumbers = numbers.isEmpty()
                let noNames = names.isEmpty()
            }
        "#,
    )
    .expect("generic array source bodies should compile");

    assert_eq!(
        defined_functions(&specialized),
        defined_functions(&baseline) + 2,
        "one concrete isEmpty body should be emitted for each reachable array element type"
    );
}

#[test]
fn array_search_methods_are_source_defined_and_preserve_element_constraints() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::ArrayContains),
        "[T].contains(value: T) -> bool where T: Equatable"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ArrayIndexOf),
        "[T].indexOf(value: T) -> u32? where T: Equatable"
    );
    for item in [StdlibItemId::ArrayContains, StdlibItemId::ArrayIndexOf] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. }
        ));
    }

    for expression in [
        "[2, 4, 6].contains(4)",
        "[2, 4, 6].indexOf(6) else 99",
        "[\"Moon\", \"Sun\"].contains(\"Sun\")",
        "[\"Moon\", \"Sun\"].indexOf(\"missing\") else 99",
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                whileAttached {{
                    print({expression})
                }}
            "#
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&splitscript::compile(&source).unwrap())
            .unwrap_or_else(|error| panic!("`{expression}` generated invalid Wasm GC: {error}"));
    }

    let errors = splitscript::compile(
        r#"
            record Marker { values: [i32] }
            state "game.exe" {}
            whileAttached {
                let markers: [Marker; 1] = [Marker { values: [1] }]
                print(markers.contains(Marker { values: [1] }))
            }
        "#,
    )
    .expect_err("searching arrays whose elements are not equatable should fail");
    assert!(
        errors.iter().any(|error| {
            error.message.contains("Equatable") && error.message.contains("Marker")
        }),
        "{errors:#?}"
    );
}

#[test]
fn source_defined_library_bodies_publish_compiler_derived_operation_metadata() {
    let library = StandardLibrary::new();
    let clamp = library.item(StdlibItemId::NumericClamp);
    assert!(matches!(
        clamp.implementation,
        Implementation::LibraryBody { .. }
    ));
    assert_eq!(
        library
            .effects(clamp.id)
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [Effect::Pure]
    );

    let timer = library.item(StdlibItemId::TimerIsRunning);
    assert!(matches!(
        timer.implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(library.effects(timer.id).contains(&Effect::ReadsTimer));
    assert!(!library.effects(timer.id).contains(&Effect::Pure));
    assert_eq!(
        library.render_operation_semantics(timer.id),
        "available everywhere; synchronous"
    );

    let relative = library.item(StdlibItemId::ModuleReadRelative32);
    assert!(matches!(
        relative.implementation,
        Implementation::LibraryBody { .. }
    ));
    let operation = library.operation_semantics(relative.id);
    assert!(library.effects(relative.id).contains(&Effect::ReadsProcess));
    assert!(operation.requires_attached_process);
    assert_eq!(operation.availability, Availability::Everywhere);
    assert_eq!(operation.suspension, SuspensionKind::None);
    assert_eq!(operation.cancellation, CancellationKind::None);

    let source = r#"
        state "game.exe" {}
        fn resolveRelative(module: Module) -> address! {
            return module.readRelative32(0)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("effectful library bodies should type check as ordinary calls");
    let function = checked.syntax().functions[0].id;
    let inferred = checked.effects().function(function);
    assert!(inferred.effects.contains(&Effect::ReadsProcess));
    assert!(inferred.requires_attached_process);
}

#[test]
fn user_code_cannot_construct_runtime_private_standard_library_records() {
    let parsed = splitscript::parse(
        r#"
        state "game.exe" {}
        gameTime {
            return Duration { seconds: 1, nanoseconds: 0 }
        }
        "#,
    )
    .unwrap();
    let diagnostics = splitscript::check(splitscript::lower(parsed)).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(
            "standard-library type `Duration` can only be constructed by standard-library source",
        )
    }));
}

#[test]
fn compiler_owned_library_function_names_are_reserved() {
    let parsed = splitscript::parse(
        r#"
        state "game.exe" {}
        fn __splitscript_stdlib_fake() {}
        "#,
    )
    .unwrap();
    let diagnostics = splitscript::check(splitscript::lower(parsed)).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`__splitscript_stdlib_` are reserved")
    }));
}

#[test]
fn state_providers_are_catalog_owned_and_resolved_after_parsing() {
    let library = StandardLibrary::new();
    let gba = library
        .state_provider_by_name("GBA")
        .expect("the bundled GBA provider should be discoverable by source name");
    assert_eq!(gba.id, StdlibStateProviderId::Gba);
    assert_eq!(gba.value_name, "gba");
    assert_eq!(gba.process_type, StdlibTypeId::GbaEmulator);
    assert_eq!(gba.direct_read, StdlibItemId::GbaEmulatorRead);
    assert_eq!(
        gba.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::GbaEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::GbaEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        gba.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"mGBA.exe")
    ));

    let lowered = splitscript::lower(splitscript::parse("state GBA {}").unwrap());
    let state = lowered.syntax().state.as_ref().unwrap();
    assert_eq!(
        state
            .provider
            .as_ref()
            .map(|provider| provider.name.as_str()),
        Some("GBA")
    );
    assert!(state.processes.is_empty());
    let checked = splitscript::check(lowered).unwrap();
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Gba)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile("state GBA { room: u8 at 0x03000010 }").unwrap())
        .expect("provider-backed attachment should compile to valid Wasm");

    let invalid = splitscript::parse("state GBA { room: u8 = process.read(0) else 0 }")
        .map(splitscript::lower)
        .unwrap();
    let diagnostics = splitscript::check(invalid)
        .expect_err("native process operations should not be available in a GBA state");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`process` is unavailable under `state GBA`; use `gba` instead")
    }));

    let invalid = splitscript::parse("state GBA { room: u8 at \"game.exe\", 0x10 }")
        .map(splitscript::lower)
        .unwrap();
    let diagnostics = splitscript::check(invalid)
        .expect_err("provider direct reads should reject native module paths");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("direct reads use hardware addresses and cannot name a module")
    }));

    let source = "state GBA { room: u8 at 0x03000010 }";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(source);
    let provider_offset = source.find("GBA").unwrap() + 1;
    assert_eq!(
        database.definition_at(provider_offset).unwrap(),
        Some(
            splitscript::tooling::database::DefinitionTarget::StandardLibrarySymbol(
                splitscript::compiler::stdlib::StdlibSymbolId::StateProvider(
                    StdlibStateProviderId::Gba
                )
            )
        )
    );
    let hover = database
        .hover(provider_offset)
        .unwrap()
        .expect("the provider name should have catalog documentation");
    assert!(hover.markdown.contains("state GBA { ... }"));
    assert!(hover.markdown.contains("gba: GbaEmulator"));
}

#[test]
fn native_processes_are_typed_provider_values_and_methods_use_the_receiver() {
    let source = r#"
        state "game.exe" {}

        fn keepProcess(value: Process) -> Process {
            return value
        }

        whileAttached {
            let attached = keepProcess(process)
            let name = attached.name()
            let value: u32! = attached.read<u32>(0x1000)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let attached = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "attached" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the attached-process binding should exist");
    let ty = checked
        .semantics()
        .value_type(attached)
        .expect("the provider value should flow through ordinary inference");
    assert_eq!(
        checked.semantics().types().kind(ty),
        &TypeKind::Standard(StdlibTypeId::Process)
    );
    let name = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "name" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the attached-process name binding should exist");
    assert_eq!(
        checked.semantics().types().kind(
            checked
                .semantics()
                .value_type(name)
                .expect("process.name should have a semantic type")
        ),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("captured Process receivers should lower to valid Wasm");
}

#[test]
fn abi_catalog_drives_wasm_imports_and_the_internal_reference() {
    let catalog = AbiCatalog::new();
    assert_eq!(catalog.validate(), Vec::<String>::new());
    assert_eq!(
        catalog.render_signature(AbiImportId::ProcessRead),
        "(i64, i64, i32, i32) -> i32"
    );
    let process_read = catalog.import(AbiImportId::ProcessRead);
    assert_eq!(
        process_read.parameters[0].ownership,
        AbiOwnership::BorrowedHandle
    );
    assert_eq!(
        process_read.parameters[2].ownership,
        AbiOwnership::OutputMemory
    );
    assert!(process_read.effects.contains(&AbiEffect::ReadsProcess));
    assert_eq!(
        catalog.import(AbiImportId::ProcessAttach).results[0].ownership,
        AbiOwnership::OwnedHandle
    );

    let wasm = splitscript::compile("state \"game.exe\" {}").unwrap();
    let mut emitted = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            for import in section.into_imports() {
                let import = import.unwrap();
                assert!(matches!(import.ty, TypeRef::Func(_)));
                emitted.push((import.module.to_owned(), import.name.to_owned()));
            }
        }
    }
    let required = [
        AbiImportId::TimerGetState,
        AbiImportId::ProcessAttach,
        AbiImportId::ProcessDetach,
        AbiImportId::ProcessIsOpen,
    ];
    let declared = catalog
        .imports()
        .filter(|import| required.contains(&import.id))
        .map(|import| (import.module.to_owned(), import.name.to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(emitted, declared);

    let abi_document = include_str!("../../docs/ABI.md");
    let table_start = abi_document
        .find("| Import | WebAssembly type |")
        .expect("ABI reference should contain its generated import table");
    let table_end = abi_document[table_start..]
        .find("\n\n")
        .map_or(abi_document.len(), |offset| table_start + offset);
    assert_eq!(
        abi_document[table_start..table_end].trim_end(),
        catalog.render_import_table().trim_end(),
        "docs/ABI.md import table must remain a verified catalog view"
    );
}

#[test]
fn settings_host_imports_are_filtered_by_setting_kind() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            settings {
                "Enabled" => enabled: true
            }
        "#,
    )
    .expect("Boolean settings should compile");
    let mut imports = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            imports.extend(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned()),
            );
        }
    }

    assert!(imports.iter().any(|name| name == "user_settings_add_bool"));
    assert!(imports.iter().any(|name| name == "setting_value_get_bool"));
    assert!(
        !imports
            .iter()
            .any(|name| name == "setting_value_get_string")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_choice")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_file_select")
    );
}

#[derive(Default)]
pub(super) struct TypedExpressionCounter(pub usize);

impl splitscript::compiler::hir::TypedVisitor for TypedExpressionCounter {
    fn visit_expression(
        &mut self,
        expression: &splitscript::compiler::hir::TypedExpression,
        program: &splitscript::compiler::hir::TypedProgram,
    ) {
        self.0 += 1;
        splitscript::compiler::hir::walk_typed_expression(self, expression, program);
    }
}

#[test]
fn standard_library_catalog_is_valid_documented_and_compilable() {
    let library = StandardLibrary::new();
    assert_eq!(library.validate(), Vec::<String>::new());
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::I32,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::F64,
        splitscript::compiler::stdlib::StdlibCapabilityId::Float,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::F64,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Integer,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
    ));
    assert!(library.type_has_capability(
        splitscript::compiler::stdlib::StdlibTypeId::String,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert_eq!(
        library.item_by_name("Numeric.clamp").map(|item| item.id),
        Some(StdlibItemId::NumericClamp)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::NumericClamp),
        "T.clamp(minimum: T, maximum: T) -> T where T: Numeric"
    );
    assert_eq!(
        library.item_by_name("setVariable").map(|item| item.id),
        Some(StdlibItemId::SetVariable)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SetTickRate),
        "setTickRate(hz: f64) -> None"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromSeconds),
        "Duration.fromSeconds<T>(seconds: T) -> Duration where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringContains),
        "String.contains(substring: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringIndexOf),
        "String.indexOf(substring: String) -> u32?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringLastIndexOf),
        "String.lastIndexOf(substring: String) -> u32?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringPadStart),
        "String.padStart(width: u32, fill: char) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringPadEnd),
        "String.padEnd(width: u32, fill: char) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringStartsWith),
        "String.startsWith(prefix: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringEndsWith),
        "String.endsWith(suffix: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringEqualsIgnoreAsciiCase),
        "String.equalsIgnoreAsciiCase(other: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringToAsciiLowerCase),
        "String.toAsciiLowerCase() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringIsEmpty),
        "String.isEmpty() -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringToAsciiUpperCase),
        "String.toAsciiUpperCase() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringTrimAsciiWhitespace),
        "String.trimAsciiWhitespace() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringJoin),
        "String.join(values: [String], separator: String) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringReplaceAll),
        "String.replaceAll(search: String, replacement: String) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringSplit),
        "String.split(delimiter: String) -> [String]!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringParse),
        "String.parse<T>() -> T! where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringByteAt),
        "String.byteAt(byteIndex: u32) -> u8!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringCharAt),
        "String.charAt(byteIndex: u32) -> char!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringSlice),
        "String.slice(start: u32, end: u32) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatAbs),
        "T.abs() -> T where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatSqrt),
        "T.sqrt() -> T where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatTruncate),
        "T.truncate() -> T where T: Float"
    );
    assert!(matches!(
        library.item(StdlibItemId::FloatRound).implementation,
        Implementation::Intrinsic { .. }
    ));
    assert_eq!(
        library.render_signature(StdlibItemId::FloatRoundTo),
        "T.roundTo(decimalPlaces: u32) -> T where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatIsFinite),
        "T.isFinite() -> bool where T: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32FromBits),
        "f32.fromBits(bits: u32) -> f32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32ToBits),
        "f32.toBits() -> u32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F64FromBits),
        "f64.fromBits(bits: u64) -> f64"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F64ToBits),
        "f64.toBits() -> u64"
    );
    for removed in [
        "timer.setVariable",
        "runtime.setTickRate",
        "Duration.saturatingSecondsF32",
    ] {
        assert!(
            library.item_by_name(removed).is_none(),
            "prototype alias `{removed}` must not remain in the catalog"
        );
    }

    for item in library.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            splitscript::compile(&example.validation_program()).unwrap_or_else(|errors| {
                panic!(
                    "standard-library example `{}: {}` failed: {errors:#?}",
                    item.qualified_name, example.title
                )
            });
        }
    }
    for provider in library.state_providers() {
        assert!(!provider.documentation.summary.is_empty());
        for example in provider.documentation.examples {
            splitscript::compile(&example.validation_program()).unwrap_or_else(|errors| {
                panic!(
                    "state-provider example `{}: {}` failed: {errors:#?}",
                    provider.name, example.title
                )
            });
        }
    }
    let declaration_documentation = library
        .namespaces()
        .iter()
        .map(|value| (value.name, value.documentation))
        .chain(
            library
                .capabilities()
                .iter()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .type_constructors()
                .iter()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .types()
                .iter()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .fields()
                .iter()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .variants()
                .iter()
                .map(|value| (value.name, value.documentation)),
        );
    let mut checked_declaration_examples = 0;
    for (name, documentation) in declaration_documentation {
        for example in documentation.examples {
            checked_declaration_examples += 1;
            splitscript::compile(&example.validation_program()).unwrap_or_else(|errors| {
                panic!(
                    "standard-library declaration example `{name}: {}` failed: {errors:#?}",
                    example.title
                )
            });
        }
    }
    assert!(
        checked_declaration_examples >= 7,
        "representative non-callable declarations should retain checked examples"
    );
    let missing_declaration_examples = library
        .namespaces()
        .iter()
        .map(|value| ("namespace", value.name, value.documentation))
        .chain(
            library
                .capabilities()
                .iter()
                .map(|value| ("capability", value.name, value.documentation)),
        )
        .chain(
            library
                .type_constructors()
                .iter()
                .map(|value| ("type constructor", value.name, value.documentation)),
        )
        .chain(
            library
                .types()
                .iter()
                .map(|value| ("type", value.name, value.documentation)),
        )
        .chain(
            library
                .fields()
                .iter()
                .filter(|value| value.visibility == FieldVisibility::Public)
                .map(|value| ("field", value.name, value.documentation)),
        )
        .chain(
            library
                .variants()
                .iter()
                .map(|value| ("variant", value.name, value.documentation)),
        )
        .filter(|(_, _, documentation)| documentation.examples.is_empty())
        .map(|(kind, name, _)| format!("{kind} {name}"))
        .collect::<Vec<_>>();
    assert_eq!(missing_declaration_examples, Vec::<String>::new());
}

#[test]
fn language_catalog_is_valid_documented_and_compilable() {
    let language = LanguageCatalog::new();
    assert_eq!(language.validate(), Vec::<String>::new());
    let retry = language
        .item_by_name("retry")
        .expect("retry should be a catalog-backed language construct");
    assert_eq!(retry.id, LanguageItemId::Retry);
    assert_eq!(retry.kind, LanguageItemKind::Keyword);
    assert_eq!(retry.form, "let value = retry resultExpression");
    assert!(retry.documentation.details.contains("T!"));
    assert_eq!(
        language
            .builtin_type(splitscript::compiler::types::BuiltinType::I32)
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::compiler::types::BuiltinType::I32
        ))
    );
    assert_eq!(
        language
            .item_for_source_token("Address")
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::compiler::types::BuiltinType::Address
        ))
    );
    assert_eq!(
        language.item_for_source_token("[").map(|item| item.id),
        Some(LanguageItemId::ArrayType)
    );
    for contextual in ["at", "key", "choice", "default", "file", "mime", "in"] {
        assert!(
            language.item_for_source_token(contextual).is_none(),
            "`{contextual}` needs its grammatical context"
        );
    }
    assert!(language.item_for_source_token("Array").is_none());
    assert_eq!(
        StandardLibrary::new()
            .field(StdlibFieldId::ModuleAddress)
            .name,
        "address"
    );
    assert_eq!(
        StandardLibrary::new()
            .variant(StdlibVariantId::TimerStateRunning)
            .name,
        "Running"
    );

    for action in [
        splitscript::compiler::ast::ActionKind::Setup,
        splitscript::compiler::ast::ActionKind::OnDetached,
        splitscript::compiler::ast::ActionKind::OnAttach,
        splitscript::compiler::ast::ActionKind::WhileAttached,
        splitscript::compiler::ast::ActionKind::Start,
        splitscript::compiler::ast::ActionKind::Split,
        splitscript::compiler::ast::ActionKind::Reset,
        splitscript::compiler::ast::ActionKind::IsLoading,
        splitscript::compiler::ast::ActionKind::GameTime,
    ] {
        let item = language.action(action);
        assert_eq!(item.name, action.name());
        assert_eq!(item.kind, LanguageItemKind::Action(action));
    }

    for item in language.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            splitscript::compile(&example.validation_program()).unwrap_or_else(|errors| {
                panic!(
                    "language example `{}: {}` failed: {errors:#?}",
                    item.name, example.title
                )
            });
        }
    }
}

#[test]
fn compiler_database_resolves_language_catalog_syntax() {
    use splitscript::{
        compiler::types::BuiltinType,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
        tooling::language::LanguageItemId,
    };

    let source = r#"
        enum Mode {
            A,
            B
        }

        settings {
            /// Select the active mode.
            "Mode" => selected key "selected-mode": choice {
                "First" => Mode.A default,
                "Second" => Mode.B
            },
            /// Select an input file.
            "Input" => input: file {
                mime => "application/octet-stream"
            }
        }

        state "game.exe" {
            level: i32 at 0x1000;
            mapName at 0x2000 as utf8(32);
            chapterName at 0x3000 as utf16le(64)
        }

        fn maybe(value: i32) -> i32? {
            return Some(value)
        }

        fn preserveBytes(value: [u8]) -> [u8] {
            return value
        }

        fn fallible() -> i32! {
            return Err("unavailable")
        }

        fn propagated() -> i32! {
            return fallible()?
        }

        setup {}

        onAttach {
            let module = await process.module("GameAssembly.dll")
            print(module.address as String)
        }

        whileAttached {
            let firstByte = preserveBytes([1u8])[0]
            for byte in [firstByte] {
                print(byte)
            }
            let timerState = TimerState.Running
            let levelChanged = current.level != old.level
            let settingChanged = settings.selected != oldSettings.selected
            let value = match maybe(1) {
                Some(value) => value,
                None => 0
            }
            print(`value {value}`)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let mode = checked.syntax().enums[0].id;
    let variant = checked.syntax().enums[0].variants[0].id;

    let i32_type = source.find("value: i32").unwrap() + "value: ".len();
    assert_eq!(
        database.definition_at(i32_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::BuiltinType(
            BuiltinType::I32
        )))
    );
    let option_type = source.find("i32?").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(option_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::OptionType))
    );
    let result_type = source.find("i32!").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(result_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ResultType))
    );
    let array_type = source.find("[u8]").unwrap();
    assert_eq!(
        database.definition_at(array_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ArrayType))
    );
    let array_index = source.find("[0]").unwrap();
    assert_eq!(
        database.definition_at(array_index).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ArrayIndex))
    );
    let propagation = source.find("fallible()?").unwrap() + "fallible()".len();
    assert_eq!(
        database.definition_at(propagation).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Propagate))
    );
    let utf8_decoder = source.find("utf8(32)").unwrap();
    assert_eq!(
        database.definition_at(utf8_decoder).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::NativeStringDecoder
        ))
    );
    let decoder_hover = database
        .hover(utf8_decoder)
        .unwrap()
        .expect("the state decoder should have language documentation");
    assert!(decoder_hover.markdown.contains("bounded native UTF-8"));
    assert!(decoder_hover.markdown.contains("not a string-size type"));
    let utf16_decoder = source.find("utf16le(64)").unwrap();
    assert_eq!(
        database.definition_at(utf16_decoder).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::NativeUtf16LeDecoder
        ))
    );
    let decoder_hover = database
        .hover(utf16_decoder)
        .unwrap()
        .expect("the UTF-16LE state decoder should have language documentation");
    assert!(decoder_hover.markdown.contains("bounded native UTF-16LE"));
    assert!(decoder_hover.markdown.contains("replacement character"));

    let module_field = source.find("module.address").unwrap() + "module.".len();
    assert_eq!(
        database.definition_at(module_field).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Field(StdlibFieldId::ModuleAddress)
        ))
    );
    let module_field_hover = database
        .hover(module_field)
        .unwrap()
        .expect("standard-library field hover");
    assert!(module_field_hover.markdown.contains("**Examples**"));
    assert!(
        module_field_hover
            .markdown
            .contains("let baseAddress = executable.address")
    );
    let timer_state = source.find("TimerState.Running").unwrap();
    assert_eq!(
        database.definition_at(timer_state).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Type(StdlibTypeId::TimerState)
        ))
    );
    assert_eq!(
        database
            .definition_at(timer_state + "TimerState.".len())
            .unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Variant(
                StdlibVariantId::TimerStateRunning
            )
        ))
    );
    let running_variant_hover = database
        .hover(timer_state + "TimerState.".len())
        .unwrap()
        .expect("standard-library variant hover");
    assert!(
        running_variant_hover
            .markdown
            .contains("timer.state() == TimerState.Running")
    );

    for (root, expected) in [
        ("current.level", SourceDefinitionId::State),
        ("old.level", SourceDefinitionId::State),
        ("settings.selected", SourceDefinitionId::Settings),
        ("oldSettings.selected", SourceDefinitionId::Settings),
    ] {
        let offset = source.find(root).unwrap();
        assert!(
            matches!(
                database.definition_at(offset).unwrap(),
                Some(DefinitionTarget::Source(definition)) if definition.id == expected
            ),
            "wrong snapshot declaration target for `{root}`"
        );
    }
    for (root, expected) in [
        ("current.level", "current / old: state snapshot"),
        ("settings.selected", "settings / oldSettings: settings view"),
    ] {
        let hover = database
            .hover(source.find(root).unwrap())
            .unwrap()
            .expect("snapshot roots should have source-aware documentation");
        assert!(hover.markdown.contains(expected));
    }

    for (spelling, expected) in [
        ("Some(value)", LanguageItemId::SomeConstructor),
        ("Err(\"unavailable\")", LanguageItemId::ErrorConstructor),
        ("None =>", LanguageItemId::BuiltinType(BuiltinType::None)),
        ("choice {", LanguageItemId::ChoiceSetting),
        ("default", LanguageItemId::ChoiceSetting),
        ("file {", LanguageItemId::FileSetting),
        ("mime =>", LanguageItemId::FileSetting),
        ("at 0x1000", LanguageItemId::StatePointerField),
        ("key \"selected-mode\"", LanguageItemId::StableSettingKey),
        ("in [firstByte]", LanguageItemId::For),
        ("setup", LanguageItemId::Setup),
        ("whileAttached", LanguageItemId::WhileAttached),
    ] {
        let offset = source.find(spelling).unwrap();
        assert_eq!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Language(expected)),
            "wrong catalog target for `{spelling}`"
        );
        let hover = database
            .hover(offset)
            .unwrap()
            .unwrap_or_else(|| panic!("missing catalog hover for `{spelling}`"));
        assert!(hover.markdown.contains("```splitscript"));
        assert!(hover.markdown.contains("**Examples**"));
    }

    let doc_comment = source.find("/// Select").unwrap();
    assert_eq!(
        database.definition_at(doc_comment).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::DocumentationComment
        ))
    );
    let doc_hover = database
        .hover(doc_comment)
        .unwrap()
        .expect("documentation comments should have language hover");
    assert!(
        doc_hover
            .markdown
            .contains("source declaration, state field")
    );
    assert!(doc_hover.markdown.contains("functions and methods"));
    assert!(doc_hover.markdown.contains("Document a source symbol"));
    assert!(doc_hover.markdown.contains("Add a setting tooltip"));
    let template = source.find('`').unwrap();
    assert_eq!(
        database.definition_at(template).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::TemplateString))
    );

    let choice = source.find("Mode.A default").unwrap();
    assert!(matches!(
        database.definition_at(choice).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(mode)
    ));
    assert!(matches!(
        database.definition_at(choice + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(variant)
    ));
}

#[test]
fn checked_program_exposes_resolved_standard_library_calls_without_codegen() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let value: i32 = 10
            let bounded = value.clamp(0, 7)
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let calls = checked.semantics().calls().collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let ResolvedCall::StandardLibrary {
        item,
        type_arguments,
        ..
    } = calls[0].1
    else {
        panic!("the call should resolve to the standard library");
    };
    assert_eq!(*item, StdlibItemId::NumericClamp);
    assert_eq!(type_arguments.len(), 1);
    assert_eq!(
        checked.semantics().types().kind(type_arguments[0]),
        &TypeKind::Builtin(BuiltinType::I32)
    );

    let action = checked
        .syntax()
        .actions
        .first()
        .expect("whileAttached action");
    let splitscript::compiler::ast::Stmt::Variable(bounded) = &action.body.statements[1] else {
        panic!("the second statement should declare the bounded value");
    };
    assert_eq!(calls[0].0, bounded.value.id);
    let result_type = checked
        .semantics()
        .expression_type(bounded.value.id)
        .expect("every checked expression has a semantic type");
    assert_eq!(
        checked.semantics().types().kind(result_type),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn expression_ids_distinguish_nested_nodes_and_expose_their_types() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let sum: i32 = 1 + 2
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let splitscript::compiler::ast::Stmt::Variable(sum) =
        &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the sum variable");
    };
    let splitscript::compiler::ast::ExprKind::Binary { left, right, .. } = &sum.value.kind else {
        panic!("expected a binary expression");
    };
    assert_ne!(sum.value.id, left.id);
    assert_ne!(sum.value.id, right.id);
    assert_ne!(left.id, right.id);
    for expression in [&sum.value, left.as_ref(), right.as_ref()] {
        let ty = checked
            .semantics()
            .expression_type(expression.id)
            .expect("the expression should have a semantic type");
        assert_eq!(
            checked.semantics().types().kind(ty),
            &TypeKind::Builtin(BuiltinType::I32)
        );
    }
}

#[test]
fn inferred_declaration_types_are_semantic_and_syntax_annotations_stay_optional() {
    let source = r#"
        let seed = 7

        state "game.exe" {
            score = seed
        }

        fn identity(value) {
            return value
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let copy = identity(seed)
            let address = module.address
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();

    let assert_builtin = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Builtin(expected));
    };
    let assert_standard = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Standard(expected));
    };

    let global = &syntax.globals[0];
    assert_eq!(global.annotation, None);
    assert_builtin(global.id, BuiltinType::I32);

    let state_field = &syntax.state.as_ref().unwrap().fields[0];
    assert_eq!(state_field.annotation, None);
    assert_builtin(state_field.id, BuiltinType::I32);

    let function = &syntax.functions[0];
    assert_eq!(function.params[0].annotation, None);
    assert_eq!(function.return_annotation, None);
    let parameter = semantics
        .value_type(function.params[0].id)
        .expect("inferred function parameters have semantic types");
    assert!(matches!(
        semantics.types().kind(parameter),
        TypeKind::GenericParameter { owner, index: 0 } if *owner == function.id
    ));
    let result = semantics
        .function_result(function.id)
        .expect("every function should have a semantic result type");
    assert_eq!(result, parameter);
    assert_eq!(semantics.function_type_parameters(function.id), [parameter]);

    let statements = &syntax.actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(module) = &statements[0] else {
        panic!("expected an awaited module binding");
    };
    assert!(matches!(
        module.value.kind,
        splitscript::compiler::ast::ExprKind::Suspend { .. }
    ));
    assert_eq!(module.annotation, None);
    assert_standard(module.id, StdlibTypeId::Module);

    let splitscript::compiler::ast::Stmt::Variable(copy) = &statements[1] else {
        panic!("expected the inferred function-call binding");
    };
    assert_eq!(copy.annotation, None);
    assert_builtin(copy.id, BuiltinType::I32);

    let splitscript::compiler::ast::Stmt::Variable(address) = &statements[2] else {
        panic!("expected the inferred member-path binding");
    };
    assert_eq!(address.annotation, None);
    assert_builtin(address.id, BuiltinType::Address);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration types should drive valid Wasm lowering");
}

#[test]
fn parsed_type_references_are_inference_free_syntax() {
    let source = r#"
        state "game.exe" {
            level: u16 at "game.exe", 0x10
        }

        record Buffer {
            values: [u32]
        }

        fn widen(value: i32) -> u64 {
            return value as u64
        }

        whileAttached {
            let count: i64 = 1i64
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    {
        use splitscript::compiler::ast::TypeRef;

        let syntax = parsed.syntax();
        assert_eq!(
            syntax.state.as_ref().unwrap().fields[0].annotation,
            Some(TypeRef::core(CoreTypeId::U16))
        );

        let values = &syntax.records[0].fields[0];
        let TypeRef::Array(array) = values.ty else {
            panic!("the record field should retain its parsed array reference");
        };
        assert_eq!(
            syntax
                .array_types
                .iter()
                .find(|declaration| declaration.id == array)
                .unwrap()
                .element,
            TypeRef::core(CoreTypeId::U32)
        );

        let function = &syntax.functions[0];
        assert_eq!(
            function.params[0].annotation,
            Some(TypeRef::core(CoreTypeId::I32))
        );
        assert_eq!(
            function.return_annotation,
            Some(TypeRef::core(CoreTypeId::U64))
        );
        let splitscript::compiler::ast::Stmt::Return {
            value: Some(cast), ..
        } = &function.body.statements[0]
        else {
            panic!("expected the cast return expression");
        };
        let splitscript::compiler::ast::ExprKind::Cast { target, .. } = &cast.kind else {
            panic!("expected a parsed cast");
        };
        assert_eq!(*target, TypeRef::core(CoreTypeId::U64));

        let splitscript::compiler::ast::Stmt::Variable(count) =
            &syntax.actions[0].body.statements[0]
        else {
            panic!("expected the annotated local");
        };
        assert_eq!(count.annotation, Some(TypeRef::core(CoreTypeId::I64)));
        let splitscript::compiler::ast::ExprKind::Int { suffix, .. } = &count.value.kind else {
            panic!("expected the suffixed integer literal");
        };
        assert_eq!(*suffix, Some(TypeRef::core(CoreTypeId::I64)));
    }

    let checked = splitscript::check(parsed).unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("syntax type references should adapt to semantic types and valid Wasm");
}

#[test]
fn source_standard_type_names_resolve_after_parsing() {
    use splitscript::compiler::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            fn base(module: Module) -> address { return module.address }
        "#,
    )
    .unwrap();
    let function = &parsed.syntax().functions[0];
    let Some(TypeRef::Named(name)) = function.params[0].annotation else {
        panic!("source nominal annotations should retain a name identity");
    };
    assert_eq!(parsed.syntax().type_name(name), "Module");

    let checked = splitscript::check(parsed).unwrap();
    let parameter_type = checked
        .semantics()
        .value_type(checked.syntax().functions[0].params[0].id)
        .unwrap();
    assert_eq!(
        parameter_type,
        checked
            .semantics()
            .types()
            .id_for_standard(StdlibTypeId::Module),
        "name resolution, inference, and published semantics must preserve one standard TypeId",
    );
    assert_eq!(
        checked.semantics().types().kind(parameter_type),
        &TypeKind::Standard(StdlibTypeId::Module)
    );
}

#[test]
fn source_record_and_enum_annotations_resolve_after_parsing() {
    use splitscript::compiler::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            record Point {
                x: i32
            }
            enum Location {
                Known(Point),
                Unknown
            }
            fn identity(point: Point) -> Point { return point }
            fn keep(location: Location) -> Location { return location }
        "#,
    )
    .unwrap();
    for (function, expected_name) in parsed.syntax().functions.iter().zip(["Point", "Location"]) {
        let Some(TypeRef::Named(parameter_name)) = function.params[0].annotation else {
            panic!("source nominal parameters should retain name identities");
        };
        let Some(TypeRef::Named(result_name)) = function.return_annotation else {
            panic!("source nominal results should retain name identities");
        };
        assert_eq!(parameter_name, result_name);
        assert_eq!(parsed.syntax().type_name(parameter_name), expected_name);
    }

    let checked = splitscript::check(parsed).unwrap();
    let point_parameter = checked.syntax().functions[0].params[0].id;
    let location_parameter = checked.syntax().functions[1].params[0].id;
    assert_eq!(
        checked.semantics().value_type(point_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_record(checked.syntax().records[0].id),
    );
    assert_eq!(
        checked.semantics().value_type(location_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_enum(checked.syntax().enums[0].id),
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(point_parameter).unwrap()),
        &TypeKind::Record(checked.syntax().records[0].id)
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(location_parameter).unwrap()),
        &TypeKind::Enum(checked.syntax().enums[0].id)
    );
}

#[test]
fn semantic_capabilities_query_declared_and_derived_types_by_type_id() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
            state "game.exe" {}
            record Pair {
                left: i32,
                right: i32
            }
            enum MaybePair {
                Pair(Pair),
                Empty
            }
            fn keep(value: MaybePair) -> MaybePair { return value }
        "#,
        )
        .expect("the capability fixture should parse"),
    )
    .expect("the capability fixture should type-check");
    let types = checked.semantics().types();
    let pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Record(record) if *record == checked.syntax().records[0].id)
                .then_some(id)
        })
        .expect("the source record should have a semantic type");
    let maybe_pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Enum(enumeration) if *enumeration == checked.syntax().enums[0].id)
                .then_some(id)
        })
        .expect("the source enum should have a semantic type");
    let string = types.id_for_standard(StdlibTypeId::String);
    let capabilities = checked.capabilities();

    assert!(capabilities.has(
        pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        maybe_pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(!capabilities.has(
        maybe_pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        string,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
        checked.semantics(),
    ));
}

#[test]
fn semantic_type_ids_intern_constructed_generic_arguments() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let matrix = [[1i32], [2i32]]
            let row = matrix[0]
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let row = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "row" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the row binding should exist");
    let row = checked
        .semantics()
        .value_type(row)
        .expect("the row binding should have a semantic type");
    let TypeKind::Array { element, .. } = checked.semantics().types().kind(row) else {
        panic!("the indexed row should retain its interned array type");
    };
    assert_eq!(
        checked.semantics().types().kind(*element),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn option_and_result_annotations_are_distinct_interned_semantic_types() {
    let source = r#"
        state "game.exe" {}

        record Wrappers {
            maybe: i32?,
            attempt: String!
        }

        fn inspect(maybe: i32?, attempt: String!) {}
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let function = &syntax.functions[0];

    let maybe = semantics.value_type(function.params[0].id).unwrap();
    let TypeKind::Option {
        layout: maybe_layout,
        value: maybe_value,
    } = semantics.types().kind(maybe)
    else {
        panic!("`i32?` should remain a first-class option type");
    };
    assert_eq!(
        semantics.types().kind(*maybe_value),
        &TypeKind::Builtin(BuiltinType::I32)
    );
    let splitscript::compiler::ast::TypeRef::Option(parsed_maybe_layout) =
        function.params[0].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its option layout");
    };
    assert_eq!(*maybe_layout, parsed_maybe_layout);

    let attempt = semantics.value_type(function.params[1].id).unwrap();
    let TypeKind::Result {
        layout: attempt_layout,
        value: attempt_value,
    } = semantics.types().kind(attempt)
    else {
        panic!("`String!` should remain a first-class result type");
    };
    assert_eq!(
        semantics.types().kind(*attempt_value),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    let splitscript::compiler::ast::TypeRef::Result(parsed_attempt_layout) =
        function.params[1].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its result layout");
    };
    assert_eq!(*attempt_layout, parsed_attempt_layout);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option and result annotations should have valid Wasm GC layouts");
}

#[test]
fn option_and_result_equality_is_structural_and_payload_checked() {
    let source = include_str!("../wrapper_equality.split");
    let wasm = splitscript::compile(source).expect("wrapper equality should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("Option and Result equality helpers should produce valid Wasm GC");

    let invalid = r#"
        state "game.exe" {}

        fn same(left: [i32]?, right: [i32]?) -> bool {
            return left == right
        }

        whileAttached {
            let value = same(None, None)
        }
    "#;
    let diagnostics = splitscript::compile(invalid)
        .expect_err("wrappers should require equality on their contained values");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("optional value does not support equality")
    }));
}

#[test]
fn option_and_result_matches_bind_values_and_require_exhaustiveness() {
    let source = include_str!("../wrapper_match.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("wrapper matches should type-check");
    assert!(
        checked
            .typed_hir()
            .patterns()
            .any(|pattern| pattern.wrapper.is_some())
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("wrapper match lowering should produce valid Wasm GC");

    for (source, missing) in [
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32?) -> i32 {
                    return match value { Some(present) => present }
                }
            "#,
            "missing `None`",
        ),
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32!) -> i32 {
                    return match value { Ok(success) => success }
                }
            "#,
            "missing `Err(error)`",
        ),
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("wrapper matches without both states must be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(missing)),
            "expected `{missing}` in {diagnostics:?}"
        );
    }

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            fn unwrap(value: i32?) -> i32 {
                return match value {
                    present => present,
                    None => 0
                }
            }
        "#,
    )
    .expect_err("bare wrapper bindings should be rejected as misleading");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("use `Some(present)` or `Ok(present)`")
    }));
}
