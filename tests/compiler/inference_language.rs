//! inference language integration tests.

use super::*;

#[test]
fn user_function_types_are_inferred_across_bodies_and_call_sites() {
    let source = r#"
        state "game.exe" {}

        record Clock {
            minutes: f32
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
fn global_types_are_inferred_from_uses_and_assignments() {
    let source = r#"
        let base = 0
        let fieldOffset = 0
        let timerState = TimerState.NotRunning
        let largeCounter = 0

        state "game.exe" {
            value: i32 = process.read.i32(base.offset(fieldOffset))
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
fn state_field_types_are_inferred_from_expressions_and_uses() {
    let source = r#"
        state "game.exe" {
            expressionValue = process.read.u16(0)
            usageValue = 0
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
    "#;
    let diagnostics =
        splitscript::compile(ambiguous).expect_err("an unconstrained pointer field is ambiguous");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot infer type variable"))
    );
}

#[test]
fn lifecycle_blocks_use_event_and_polling_names_without_prototype_aliases() {
    use splitscript::compiler::ast::ActionKind;

    let source = r#"
        state "game.exe" {}
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
        diagnostic
            .message
            .contains("can only construct an optional value")
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
            minutes: f32
            seconds: f32
            hundredths: f32
        }

        record TimerInfo {
            digits: Digits
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
                        wasmparser::Operator::GlobalSet { global_index: 6 } => {
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
fn enums_and_their_payloads_use_structural_equality() {
    let source = r#"
        record Position {
            x: i32
            y: i32
        }

        enum Value {
            Position(Position)
            Label(String)
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
            Items(Array<i32>)
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
            Level(i32)
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
            Yes
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
            minutes: f32
            seconds: f32
        }

        record TimerInfo {
            digits: Digits
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
            bytes: Array<u8>
        }

        fn ScanBuffer.prepare() -> bool {
            self.bytes.set(1u32, 0x8bu8)
            return self.bytes.length() == 3u32
                && self.bytes.get(0u32) == 0x48u8
                && self.bytes.get(1u32) == 0x8bu8
        }

        onAttach {
            let inferred = [1, 2, 3]
            let empty: Array<u16> = []
            let buffer = ScanBuffer {
                bytes: [0x48u8, 0u8, 0u8]
            }
            await process.module("GameAssembly.dll")
            if (buffer.prepare()
                && inferred.get(2u32) == 3
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
