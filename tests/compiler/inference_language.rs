//! inference language integration tests.

use super::*;

#[test]
fn user_function_types_are_inferred_across_bodies_and_call_sites() {
    let source = r#"
        state "game.exe" {}

        record Clock {
            minutes: f32,
            seconds: f32
        }

        record Snapshot {
            clock: Clock
        }

        fn increment(value) {
            return value + 1
        }

        fn same(left, right) {
            return left == right
        }

        fn formatClock(snapshot) {
            return snapshot.clock.seconds
        }

        whileAttached {
            let result: u64 = increment(41)
            if (same(result, 42)) {
                print("inferred through the call graph")
            }
            let seconds: f32 = formatClock(Snapshot {
                clock: Clock { minutes: 1.0, seconds: 2.0 }
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("function and record receiver types should be inferred");
    let snapshot = checked.syntax().records[1].id;
    let format_clock = &checked.syntax().functions[2];
    assert_eq!(
        checked.semantics().types().kind(
            checked
                .semantics()
                .value_type(format_clock.params[0].id)
                .unwrap()
        ),
        &TypeKind::Record(snapshot)
    );
    let splitscript::compiler::ast::Stmt::Return {
        value: Some(returned),
        ..
    } = &format_clock.body.statements[0]
    else {
        panic!("expected formatClock's return expression");
    };
    let path = returned;
    assert_eq!(
        checked.semantics().path_members(path.id),
        Some(
            [
                ResolvedMember::RecordField(checked.syntax().records[1].fields[0].id),
                ResolvedMember::RecordField(checked.syntax().records[0].fields[1].id),
            ]
            .as_slice()
        )
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred function signatures should produce valid Wasm");

    let ambiguous = r#"
        record First { value: i32 }
        record Second { value: i32 }
        state "game.exe" {}
        fn inspect(item) { return item.value }
    "#;
    let errors = splitscript::check(splitscript::parse(ambiguous).unwrap())
        .expect_err("shared field names need enough call-site context");
    assert!(errors.iter().any(|error| {
        error.message.contains("does not uniquely determine")
            && error.message.contains("First")
            && error.message.contains("Second")
    }));
}

#[test]
fn inferred_functions_are_independently_instantiated_at_each_call_site() {
    let source = r#"
        state "game.exe" {}

        fn identity(value) {
            return value
        }

        fn singleton(value) {
            return [value]
        }

        fn localArrayLength(value) -> u32 {
            let values = [value]
            return values.length()
        }

        fn throughOption(value) {
            let wrapped = Some(value)
            return match wrapped {
                Some(inner) => inner,
                None => value
            }
        }

        fn throughResult(value) {
            let wrapped = Ok(value)
            return match wrapped {
                Ok(inner) => inner,
                Err(_) => value
            }
        }

        fn addOne(value) {
            return value + 1
        }

        whileAttached {
            let number: i32 = identity(7)
            let text: String = identity("seven")
            let numbers: [i32] = singleton(number)
            let texts: [String] = singleton(text)
            let numberCount = localArrayLength(number)
            let textCount = localArrayLength(text)
            let boolCount = localArrayLength(true)
            let optional: bool = throughOption(true)
            let successful: bool = throughResult(optional)
            let small: i32 = addOne(1)
            let large: u64 = addOne(1)
            if successful {
                print(`{numbers.length()}: {texts[0u32]} ({numberCount + textCount + boolCount + small as u32 + large as u32})`)
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("ordinary inferred functions should be polymorphic");
    for function in &checked.syntax().functions {
        assert_eq!(
            checked
                .semantics()
                .function_type_parameters(function.id)
                .len(),
            1
        );
    }
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("every concrete primitive and constructed instance should validate");
}

#[test]
fn inferred_capability_bounds_are_enforced_at_every_call() {
    let source = r#"
        state "game.exe" {}
        fn addOne(value) { return value + 1 }
        whileAttached { let invalid = addOne(true) }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("a concrete type must satisfy the inferred Numeric bound");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("does not support")
            || diagnostic.message.contains("type") && diagnostic.message.contains("bool")
    }));
}

#[test]
fn equality_infers_none_from_the_opposite_optional_operand() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let value: i32? = None
            if value == None { print("none") }
            if None != value { print("some") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("either equality operand should determine None's optional type");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("contextually typed None equality should produce valid Wasm GC");

    let unit = r#"
        state "game.exe" {}
        whileAttached {
            if None == None { print("equal") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(unit).unwrap())
        .expect("two None values compare as ordinary unit values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("unit equality should preserve operand effects and fold to its sole outcome");
}

#[test]
fn none_values_flow_through_locals_into_options() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let unit = None
            let optional: i32? = unit
            if optional == None { print("none") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("the None unit value should convert to the empty side of any Option");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("an erased None local converted to an Option should produce valid Wasm GC");
}

#[test]
fn state_snapshots_flow_through_locals_parameters_and_returns() {
    let source = r#"
        state "game.exe" {
            /// Current level number.
            level: u32 at 0x100
        }

        fn levelOf(snapshot) {
            return snapshot.level
        }

        fn identity(value) {
            return value
        }

        whileAttached {
            let snapshot = identity(current)
            let previous = old
            let level: u32 = levelOf(snapshot)
            if level != previous.level {
                print(level)
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("state snapshots should be ordinary inferred read-only values");
    let snapshot_type = checked.semantics().types().id_for_state_snapshot();
    assert_eq!(
        checked
            .semantics()
            .value_type(checked.syntax().functions[0].params[0].id),
        Some(snapshot_type)
    );
    assert!(
        checked
            .semantics()
            .types()
            .iter()
            .any(|(id, kind)| id == snapshot_type && matches!(kind, TypeKind::StateSnapshot))
    );

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("first-class state snapshot references should produce valid Wasm GC");
}

#[test]
fn settings_views_flow_through_locals_parameters_and_returns_without_gc_objects() {
    let source = r#"
        enum CaptureMode {
            WindowTitle,
            FullPath
        }

        settings {
            /// Enables splitting.
            "Enabled" => enabled: true,
            /// Selects the capture source.
            "Capture Mode" => captureMode: choice {
                "Window Title" => CaptureMode.WindowTitle default,
                "Full Path" => CaptureMode.FullPath
            }
        }

        state "game.exe" {}

        fn changed(currentSettings, previousSettings) {
            return currentSettings.enabled != previousSettings.enabled
                || currentSettings.captureMode != previousSettings.captureMode
        }

        fn identity(value) {
            return value
        }

        whileAttached {
            let captured = identity(settings)
            let previous = oldSettings
            if changed(captured, previous) {
                if captured.enabled {
                    print("enabled")
                }
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("settings views should be ordinary inferred read-only values");
    let view_type = checked.semantics().types().id_for_settings_view();
    assert_eq!(
        checked
            .semantics()
            .value_type(checked.syntax().functions[0].params[0].id),
        Some(view_type)
    );
    assert!(
        checked
            .semantics()
            .types()
            .iter()
            .any(|(id, kind)| id == view_type && matches!(kind, TypeKind::SettingsView))
    );

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("selector-backed settings views should produce valid Wasm");
}

#[test]
fn recursive_generic_components_reuse_the_callers_concrete_instance() {
    let source = r#"
        state "game.exe" {}

        fn alternate(value, remaining: u32) {
            if remaining == 0u32 {
                return value
            }
            return continueAlternate(value, remaining - 1u32)
        }

        fn continueAlternate(value, remaining: u32) {
            return alternate(value, remaining)
        }

        whileAttached {
            let number: i32 = alternate(7, 2u32)
            let text: String = alternate("seven", 2u32)
            print(`{number}: {text}`)
        }
    "#;
    let wasm = splitscript::compile(source).expect("mutually recursive generics should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("recursive generic instances should produce valid Wasm");
}

#[test]
fn inferred_method_parameters_are_instantiated_independently() {
    let source = r#"
        record Selector { marker: i32 }
        state "game.exe" {}

        fn Selector.choose(value, fallback) {
            if self.marker == 0 {
                return value
            }
            return fallback
        }

        whileAttached {
            let selector = Selector { marker: 0 }
            let number: i32 = selector.choose(1, 2)
            let text: String = selector.choose("one", "two")
            print(`{number}: {text}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("method arguments should support inferred schemes");
    assert_eq!(
        checked
            .semantics()
            .function_type_parameters(checked.syntax().functions[0].id)
            .len(),
        1
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("generic method instances should produce valid Wasm");
}

#[test]
fn polymorphic_recursion_has_a_focused_diagnostic() {
    let source = r#"
        state "game.exe" {}
        fn recurse(value) {
            return recurse([value])
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("a recursive call may not instantiate its own component polymorphically");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("polymorphic recursion is not supported")
    }));
}

#[test]
fn generic_instance_expansion_has_a_deterministic_depth_limit() {
    let mut source = String::from("state \"game.exe\" {}\n");
    for index in 0..65 {
        if index == 64 {
            source.push_str(&format!("fn step{index}(value) {{ return value }}\n"));
        } else {
            source.push_str(&format!(
                "fn step{index}(value) {{ return step{}(value) }}\n",
                index + 1
            ));
        }
    }
    source.push_str("whileAttached { let value: i32 = step0(1) }\n");
    let diagnostics = splitscript::compile(&source)
        .expect_err("generic instance expansion should have a stable safety limit");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("recursion-depth limit of 64") })
    );
}

#[test]
fn generic_instance_expansion_has_a_deterministic_total_limit() {
    let mut source = String::from(
        "state \"game.exe\" {}\nfn identity(value) { return value }\nwhileAttached {\n",
    );
    let declarations = (0..257)
        .map(|index| format!("record Item{index} {{ value: i32 }}\n"))
        .collect::<String>();
    source.insert_str(0, &declarations);
    for index in 0..257 {
        source.push_str(&format!(
            "let item{index}: Item{index} = identity(Item{index} {{ value: {index} }})\n"
        ));
    }
    source.push_str("}\n");
    let diagnostics = splitscript::compile(&source)
        .expect_err("generic instance expansion should have a total safety limit");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("limit of 256 concrete instances")
    }));
}

#[test]
fn integer_looking_literals_flow_into_float_contexts_exactly() {
    let valid = r#"
        let global: f32 = 16_777_216
        state "game.exe" {}

        fn identity(value) {
            return value
        }

        whileAttached {
            let small: f32 = 2
            let large: f64 = 9_007_199_254_740_992
            let inferred: f32 = identity(3)
        }
    "#;
    let wasm = splitscript::compile(valid)
        .expect("exact integer literals should satisfy float expectations");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("contextual float literals should lower to valid local and global constants");

    for (ty, value) in [("f32", "16_777_217"), ("f64", "9_007_199_254_740_993")] {
        let source =
            format!("state \"game.exe\" {{}} whileAttached {{ let value: {ty} = {value} }}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("an inexact integer literal must not silently lose precision");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == splitscript::DiagnosticCode::Type
                    && diagnostic.message.contains("integer literal")
                    && diagnostic.message.contains(ty)
            }),
            "missing exactness diagnostic for {value} as {ty}: {diagnostics:#?}"
        );
    }
}

#[test]
fn decimal_exponents_preserve_representable_subnormal_float_literals() {
    let valid = r#"
        let smallestF32: f32 = 1e-45
        let smallestF64: f64 = 5e-324
        state "game.exe" {}

        whileAttached {
            let scientific: f64 = 6.022e+23
        }
    "#;
    let wasm =
        splitscript::compile(valid).expect("finite subnormal decimal literals should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("subnormal constants should lower to valid Wasm");

    for (ty, literal, expected) in [
        ("f32", "1e-46", "underflows `f32` to zero"),
        ("f32", "1e39", "overflows the finite `f32` range"),
        ("f64", "1e-325", "underflows `f64` to zero"),
        ("f64", "1e309", "overflows the finite `f64` range"),
    ] {
        let source = format!("let value: {ty} = {literal}\nstate \"game.exe\" {{}}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("an out-of-range floating-point literal must be diagnosed");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` diagnostic for {literal} as {ty}: {diagnostics:#?}"
        );
    }
}

#[test]
fn numeric_defaults_do_not_select_process_memory_representations() {
    let source = r#"
        state "game.exe" {
            literal = process.read(0x80);
            integer = process.read(0x100);
            floating = process.read(0x200);
        }

        whileAttached {
            let withLiteral = current.literal + 1
            let masked = current.integer & current.integer
            let rounded = current.floating.round()
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("numeric defaults must not decide a process-memory layout");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic
                .message
                .contains("cannot infer the memory type read"))
            .count(),
        3,
        "{diagnostics:#?}"
    );
}

#[test]
fn global_types_are_inferred_from_uses_and_assignments() {
    let source = r#"
        let base = 0
        let fieldOffset = 0
        let timerState = TimerState.NotRunning
        let largeCounter = 0

        state "game.exe" {
            value: i32 = process.read<i32>(base.offset(fieldOffset))
        }

        fn consumeU64(value: u64) {}

        whileAttached {
            timerState = timer.state()
            consumeU64(largeCounter)
        }
    "#;
    let wasm = splitscript::compile(source).expect("global uses should determine their types");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred globals should produce type-correct Wasm globals");
}

#[test]
fn none_initialized_globals_infer_options_from_later_assignments() {
    let source = r#"
        let pending = None
        let unit = None

        state "game.exe" {}

        whileAttached {
            pending = Instant.now()
            let startedAt = pending else return
            if startedAt == Instant.now() { print("same instant") }
            if unit == None { print("unit remains None") }
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("a later assignment should infer an option-valued global");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the inferred optional global should produce valid Wasm GC");
}

#[test]
fn state_field_types_are_inferred_from_expressions_and_uses() {
    let source = r#"
        state "game.exe" {
            expressionValue = process.read<u16>(0);
            usageValue = 0;
            pointerValue at 0x1234
        }

        fn consumeU32(value: u32) {}
        fn consumeU64(value: u64) {}

        whileAttached {
            consumeU32(current.usageValue)
            consumeU64(current.pointerValue)
        }
    "#;
    let wasm = splitscript::compile(source).expect("state field types should be inferred");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred state fields should produce a concrete GC state type");

    let ambiguous = r#"
        state "game.exe" {
            mystery at 0x1234
        }

        whileAttached {
            let rounded = current.mystery.round()
        }
    "#;
    let diagnostics = splitscript::compile(ambiguous)
        .expect_err("even a float-constrained pointer field needs an explicit memory type");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the memory type of state field `mystery`")
    }));
}

#[test]
fn lifecycle_blocks_use_event_and_polling_names_without_prototype_aliases() {
    use splitscript::compiler::ast::ActionKind;

    let source = r#"
        state "game.exe" {}
        setup { setTickRate(30.0) }
        onDetached { setTickRate(1.0) }
        onAttach { setTickRate(120.0) }
        whileAttached { print("tick") }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert_eq!(
        checked
            .syntax()
            .actions
            .iter()
            .map(|action| action.kind)
            .collect::<Vec<_>>(),
        [
            ActionKind::Setup,
            ActionKind::OnDetached,
            ActionKind::OnAttach,
            ActionKind::WhileAttached,
        ]
    );
    assert_eq!(ActionKind::parse("update"), None);
    assert_eq!(ActionKind::parse("detached"), None);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("canonical lifecycle blocks should produce valid Wasm");
}

#[test]
fn action_fallthroughs_use_domain_defaults_and_null_is_scoped() {
    let source = r#"
        state "game.exe" {}

        start {}
        split { return }
        reset {
            if (false) {
                return true
            }
        }
        isLoading { return None }
        gameTime {}
    "#;
    let wasm = splitscript::compile(source).expect("action fallthroughs should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("nullable action results should produce type-correct Wasm");

    let invalid = r#"
        state "game.exe" {}
        start { return None }
    "#;
    let diagnostics =
        splitscript::compile(invalid).expect_err("start must not expose a nullable result");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("types do not match")
            && diagnostic.message.contains("None")
            && diagnostic.message.contains("bool")
    }));
}

#[test]
fn as_casts_lower_all_numeric_representations_and_integer_strings() {
    let source = r#"
        state "game.exe" {}

        fn exercise(small: i8, wide: u64, fraction: f32, pointer: address) {
            let widened = small as i64
            let narrowed = wide as u8
            let floating = widened as f64
            let integral = fraction as i16
            let addressValue = wide as address
            let rawAddress = pointer as u64
            print(widened as String)
            print(wide as String)
            print(addressValue as String)
        }
    "#;
    let wasm = splitscript::compile(source).expect("supported `as` casts should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("numeric casts should lower to type-correct Wasm");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let value = "not a number" as u32
        }
    "#;
    let diagnostics = splitscript::compile(invalid).expect_err("String-to-number must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not support this operation")
    }));
}

#[test]
fn gc_records_support_nesting_functions_and_async_frames() {
    let source = r#"
        state "game.exe" {}

        fn isHana(timer: TimerInfo) -> bool {
            return timer.digits.minutes == 0.0 && timer.character == "Hana"
        }

        record Digits {
            minutes: f32,
            seconds: f32,
            hundredths: f32
        }

        record TimerInfo {
            digits: Digits,
            character: String
        }

        onAttach {
            let timer = TimerInfo {
                character: "Hana",
                digits: Digits {
                    hundredths: 0.0,
                    seconds: 0.0,
                    minutes: 0.0
                }
            }
            await process.module("GameAssembly.dll")
            if (isHana(timer)) {
                print(timer.character)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("nested records should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("record GC structs should produce valid Wasm");
}

#[test]
fn timer_state_is_a_compiler_provided_exhaustive_enum() {
    let source = r#"
        let previous = TimerState.NotRunning

        state "game.exe" {}

        whileAttached {
            let current = timer.state()
            let justStarted = previous == TimerState.NotRunning
                && current != TimerState.NotRunning
            let label = match current {
                TimerState.NotRunning => "Not Running",
                TimerState.Running => "Running",
                TimerState.Paused => "Paused",
                TimerState.Ended => "Ended",
                TimerState.Unknown => "Unknown"
            }
            previous = current
            if justStarted {
                setVariable("Transition", "Started")
            }
            setVariable("Timer State", label)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert!(
        checked
            .syntax()
            .enums
            .iter()
            .all(|enumeration| enumeration.name != "TimerState"),
        "compiler-provided declarations must not be injected into source syntax"
    );
    assert!(
        checked
            .enum_types()
            .iter()
            .all(|enumeration| enumeration.name != "TimerState"),
        "standard-library enums must not be materialized as source enum layouts"
    );
    let library = StandardLibrary::new();
    let timer_state = library.type_decl(StdlibTypeId::TimerState);
    assert_eq!(
        library
            .variants_of(timer_state.id)
            .map(|variant| variant.name)
            .collect::<Vec<_>>(),
        ["NotRunning", "Running", "Paused", "Ended", "Unknown"]
    );
    let timer_state_call = checked
        .typed_hir()
        .expressions()
        .find(|expression| {
            matches!(
                checked.typed_hir().call(expression.id),
                Some(ResolvedCall::StandardLibrary {
                    item: StdlibItemId::TimerState,
                    ..
                })
            )
        })
        .expect("timer.state should resolve through the standard-library catalog");
    assert_eq!(
        checked.semantics().types().kind(timer_state_call.ty),
        &TypeKind::Standard(StdlibTypeId::TimerState)
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("TimerState host conversion and exhaustive matches should produce valid Wasm GC");

    let mut imported_functions = 0_u32;
    let mut start_function = None;
    let mut defined_function = 0_u32;
    let mut initializes_previous = false;
    for payload in Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            Payload::ImportSection(section) => {
                imported_functions += section.into_imports().count() as u32;
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    if export.name == "_start" {
                        start_function = Some(export.index);
                    }
                }
            }
            Payload::CodeSectionEntry(body)
                if Some(imported_functions + defined_function) == start_function =>
            {
                let mut constructed_enum = false;
                for operator in body.get_operators_reader().unwrap() {
                    match operator.unwrap() {
                        wasmparser::Operator::StructNew { .. } => constructed_enum = true,
                        wasmparser::Operator::GlobalSet { .. } if constructed_enum => {
                            initializes_previous = constructed_enum;
                            break;
                        }
                        _ => {}
                    }
                }
                defined_function += 1;
            }
            Payload::CodeSectionEntry(_) => defined_function += 1,
            _ => {}
        }
    }
    assert!(
        initializes_previous,
        "start must materialize standard-enum global initializers instead of leaving null GC references"
    );

    let incomplete = r#"
        state "game.exe" {}
        whileAttached {
            let running = match timer.state() {
                TimerState.Running => true,
                TimerState.NotRunning => false
            }
        }
    "#;
    let errors = splitscript::compile(incomplete)
        .expect_err("TimerState matches must handle every state or use a wildcard");
    for missing in ["Paused", "Ended", "Unknown"] {
        assert!(errors.iter().any(|error| {
            error.message.contains("non-exhaustive match") && error.message.contains(missing)
        }));
    }

    let redeclared = r#"
        enum TimerState { Custom }
        state "game.exe" {}
    "#;
    let parsed = splitscript::parse(redeclared)
        .expect("a nominal declaration conflict does not make the syntax invalid");
    let error = splitscript::check(parsed)
        .expect_err("standard-library nominal types cannot be redeclared during resolution");
    assert!(error[0].message.contains("standard-library enum"));
    assert_eq!(error[0].code, splitscript::DiagnosticCode::Type);
    assert_ne!(error[0].span, splitscript::compiler::ast::Span::default());
}

#[test]
fn aggregate_global_constants_are_materialized_once_at_module_start() {
    let source = r#"
        record Point {
            x: f32,
            y: f32
        }

        let points: [Point; 2] = [
            Point { x: 1.0, y: 2.0 },
            Point { x: 3.0, y: 4.0 }
        ]
        let label = "route"

        state "game.exe" {
            selected: u32 at 0x100
        }

        split {
            return label == "route"
                && points[current.selected].x == 3.0
        }
    "#;

    let wasm = splitscript::compile(source).expect("aggregate globals should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("aggregate globals should produce valid Wasm GC");

    let mut imported_functions = 0_u32;
    let mut start_function = None;
    let mut defined_function = 0_u32;
    let mut start_record_count = 0_u32;
    let mut start_array_count = 0_u32;
    let mut stores_constructed_array = false;
    for payload in Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            Payload::ImportSection(section) => {
                imported_functions += section.into_imports().count() as u32;
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    if export.name == "_start" {
                        start_function = Some(export.index);
                    }
                }
            }
            Payload::CodeSectionEntry(body)
                if Some(imported_functions + defined_function) == start_function =>
            {
                let mut constructed_array = false;
                for operator in body.get_operators_reader().unwrap() {
                    match operator.unwrap() {
                        wasmparser::Operator::StructNew { .. } => start_record_count += 1,
                        wasmparser::Operator::ArrayNewFixed { .. } => {
                            start_array_count += 1;
                            constructed_array = true;
                        }
                        wasmparser::Operator::GlobalSet { .. } if constructed_array => {
                            stores_constructed_array = true;
                        }
                        _ => {}
                    }
                }
                defined_function += 1;
            }
            Payload::CodeSectionEntry(_) => defined_function += 1,
            _ => {}
        }
    }

    assert!(
        start_record_count >= 2,
        "start should construct both records"
    );
    assert_eq!(
        start_array_count, 2,
        "start should construct the point array and string global once each"
    );
    assert!(
        stores_constructed_array,
        "start should store the constructed array in its source global"
    );
}

#[test]
fn enums_and_their_payloads_use_structural_equality() {
    let source = r#"
        record Position {
            x: i32,
            y: i32
        }

        enum Value {
            Position(Position),
            Label(String),
            Empty
        }

        state "game.exe" {}

        whileAttached {
            let left = Value.Position(Position { x: 4, y: 8 })
            let right = Value.Position(Position { x: 4, y: 8 })
            let same = left == right
            let different = Value.Label("four") != Value.Label("five")
            let empty = Value.Empty == Value.Empty
            let recordsEqual = Position { x: 1, y: 2 } == Position { x: 1, y: 2 }
            if same && different && empty && recordsEqual {
                setVariable("Equality", "works")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("structural enum equality should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("structural enum equality should produce valid Wasm GC");

    let unsupported = r#"
        enum Values {
            Items([i32])
        }

        state "game.exe" {}

        whileAttached {
            let left = Values.Items([1, 2])
            let right = Values.Items([1, 2])
            let same = left == right
        }
    "#;
    let errors = splitscript::compile(unsupported)
        .expect_err("enum payloads without equality must be rejected precisely");
    assert!(errors.iter().any(|error| {
        error.message.contains("Values.Items")
            && error.message.contains("does not support equality")
    }));
}

#[test]
fn payload_enums_are_exhaustively_matched_and_survive_await() {
    let source = r#"
        state "game.exe" {}

        enum LevelOrScene {
            Level(i32),
            Scene(String)
        }

        fn isFirst(value: LevelOrScene) -> bool {
            return match value {
                LevelOrScene.Level(level) if level == 0 => true,
                LevelOrScene.Level(level) => false,
                LevelOrScene.Scene(scene) => scene == "Shrine01"
            }
        }

        onAttach {
            let location = LevelOrScene.Scene("Shrine01")
            await process.module("GameAssembly.dll")
            if (isFirst(location)) {
                print("first")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("payload enum should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("enum GC structs and match lowering should validate");
}

#[test]
fn match_requires_every_enum_variant() {
    let source = r#"
        state "game.exe" {}
        enum Choice {
            Yes,
            No
        }
        fn choose(value: Choice) -> bool {
            return match value {
                Choice.Yes => true
            }
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("match must be exhaustive");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-exhaustive match"))
    );
}

#[test]
fn literal_matches_support_guards_wildcards_and_bidirectional_inference() {
    let source = r#"
        state "game.exe" {}

        fn character(value, dlc) {
            return match value {
                3 if dlc => "Accel",
                3 => "Erika",
                _ => "Unknown"
            }
        }

        fn booleanName(value) {
            return match value {
                true => "yes",
                false => "no"
            }
        }

        onAttach {
            print(character(3, true))
            print(booleanName(false))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("literal matches should compile");
    let lowered = splitscript::lower_wasm(&checked);
    let mut patterns = [false; 3];
    let mut guarded = false;
    for expression in lowered.expressions() {
        let splitscript::compiler::wasm_ir::ExpressionKind::Match { arms, .. } = &expression.kind
        else {
            continue;
        };
        for arm in arms {
            match arm.pattern {
                splitscript::compiler::wasm_ir::LoweredPattern::Bool(_) => patterns[0] = true,
                splitscript::compiler::wasm_ir::LoweredPattern::Int(_) => patterns[1] = true,
                splitscript::compiler::wasm_ir::LoweredPattern::Wildcard => patterns[2] = true,
                splitscript::compiler::wasm_ir::LoweredPattern::Enum { .. }
                | splitscript::compiler::wasm_ir::LoweredPattern::OptionNone(_)
                | splitscript::compiler::wasm_ir::LoweredPattern::OptionSome { .. }
                | splitscript::compiler::wasm_ir::LoweredPattern::ResultSuccess { .. }
                | splitscript::compiler::wasm_ir::LoweredPattern::ResultError { .. } => {}
            }
            guarded |= arm.guard.is_some();
        }
    }
    assert!(patterns.into_iter().all(|pattern| pattern));
    assert!(guarded);

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("guarded literal match lowering should validate");
}

#[test]
fn integer_matches_require_a_wildcard() {
    let source = r#"
        state "game.exe" {}
        fn character(value: u32) -> String {
            return match value {
                0 => "Hana"
            }
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("integer match must be exhaustive");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("non-exhaustive integer match") })
    );
}

#[test]
fn else_if_chains_parse_type_check_and_satisfy_return_analysis() {
    let source = r#"
        state "game.exe" {}

        fn signName(value: i32) -> String {
            if value < 0 {
                return "negative"
            } else if value & 1 == 0 {
                return "even"
            } else if value == 1 {
                return "one"
            } else {
                return "positive"
            }
        }

        onAttach {
            print(signName(1))
        }
    "#;
    let wasm = splitscript::compile(source).expect("else-if chain should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("else-if lowering should produce valid Wasm");
}

#[test]
fn comparisons_follow_rusts_non_chaining_rule() {
    let source = r#"
        state "game.exe" {}
        fn between(value: i32) -> bool {
            return 0 < value < 10
        }
    "#;
    let diagnostics =
        splitscript::compile(source).expect_err("comparison chains should require parentheses");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("comparison operators cannot be chained")
    }));
}

#[test]
fn methods_have_implicit_self_and_support_nested_receivers() {
    let source = r#"
        state "game.exe" {}

        record Digits {
            minutes: f32,
            seconds: f32
        }

        record TimerInfo {
            digits: Digits,
            stopped: bool
        }

        fn Digits.isZero() -> bool {
            return self.minutes == 0.0 && self.seconds == 0.0
        }

        fn TimerInfo.canStart(expectedStopped: bool) -> bool {
            return self.digits.isZero() && self.stopped == expectedStopped
        }

        whileAttached {
            let timer = TimerInfo {
                digits: Digits { minutes: 0.0, seconds: 0.0 },
                stopped: false
            }
            if (timer.canStart(false)) {
                print("method")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("methods should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("method calls should produce valid Wasm");
}

#[test]
fn generic_gc_arrays_infer_elements_and_support_core_methods() {
    let source = r#"
        state "game.exe" {}

        record ScanBuffer {
            bytes: [u8]
        }

        fn ScanBuffer.prepare() -> bool {
            self.bytes.set(1u32, 0x8bu8)
            return self.bytes.length() == 3u32
                && self.bytes[0u32] == 0x48u8
                && self.bytes[1u32] == 0x8bu8
        }

        onAttach {
            let inferred = [1, 2, 3]
            let empty: [u16] = []
            let buffer = ScanBuffer {
                bytes: [0x48u8, 0u8, 0u8]
            }
            await process.module("GameAssembly.dll")
            if (buffer.prepare()
                && inferred[2u32] == 3
                && empty.length() == 0u32) {
                print("array")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("generic arrays should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("monomorphized GC arrays should validate");
}

#[test]
fn array_indexing_is_first_class_bidirectional_and_array_only() {
    let source = r#"
        state "game.exe" {}

        record Point {
            x: i32,
        }

        whileAttached {
            let points: [Point] = [Point { x: 42 }]
            let answer: i32 = points[0].x
            let matrix = [[1u8, 2u8]]
            let inferred = [0]
            let inferredByte: u8 = inferred[0]
            print(matrix[0][1])
            print(answer)
            print(inferredByte)
        }
    "#;
    let wasm =
        splitscript::compile(source).expect("indexing should infer receiver and result types");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("first-class indexing should produce valid Wasm GC array access");

    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let invalid = 42[0]
            }
        "#,
    )
    .expect_err("non-array values must not be indexable");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("cannot be indexed; expected an array")
    }));

    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let values = [1, 2]
                let invalid = values.get(0)
            }
        "#,
    )
    .expect_err("the retired get method must not remain available");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("no method `get`"))
    );
}
