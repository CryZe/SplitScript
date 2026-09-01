//! async runtime integration tests.

use super::*;

fn semantic_statements(
    block: &splitscript::compiler::wasm_ir::Block,
) -> impl Iterator<Item = &splitscript::compiler::wasm_ir::Statement> {
    block.statements.iter().filter(|statement| {
        !matches!(
            statement,
            splitscript::compiler::wasm_ir::Statement::DebugLocation(_)
        )
    })
}

#[test]
fn never_completions_join_with_values_and_erase_from_wasm_storage() {
    let source = r#"
        state "game.exe" {
            layout Steam { level: u32 at 0x100; },
            layout GOG { level: u32 at 0x200; },
        }

        fn ignoreUnsupportedBuild(flag: bool) -> async Never {
            let result = if flag {
                await process.closed()
            } else {
                await process.closed()
            }
            return result
        }

        onAttach {
            let path = process.path() else await ignoreUnsupportedBuild(true)
            let conditional = if path == "game.exe" {
                1
            } else {
                await ignoreUnsupportedBuild(true)
            }
            let selected = match conditional {
                0 => await ignoreUnsupportedBuild(true),
                1 => StateLayout.Steam,
                _ => StateLayout.GOG,
            }
            return selected
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("never should inhabit fallback, conditional, and match result types");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("never values should not create invalid Wasm locals or frame fields");
}

#[test]
fn ordinary_values_do_not_flow_into_never() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn returnsNormally() -> Never {
                return 1
            }
        "#,
    )
    .expect_err("a normal integer value is not a never value");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("expected `Never`")),
        "{diagnostics:#?}"
    );
}

#[test]
fn lowercase_never_is_not_a_type_alias() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn oldSpelling() -> never {
                await process.closed()
            }
        "#,
    )
    .expect_err("the bottom type has exactly one source spelling");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type `never`")),
        "{diagnostics:#?}"
    );
}

#[test]
fn never_can_appear_as_an_uninhabited_aggregate_payload() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            fn absentBottom() -> Never? {
                return None
            }

            whileAttached {
                absentBottom()
            }
        "#,
    )
    .expect("an optional bottom type should have a valid aggregate representation");

    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("an uninhabited payload slot should still produce valid Wasm GC");
}

#[derive(Default)]
struct AsyncTestHost {
    process_open: bool,
    timer_state: i32,
    monotonic_nanoseconds: i64,
    messages: Vec<String>,
    memory_regions: Vec<(u64, Vec<u8>)>,
    module_lookups: usize,
    process_reads: Vec<u64>,
    raw_scene: i32,
    raw_entities: i32,
    fail_scene_read: bool,
    fail_entities_read: bool,
}

fn execute_with_mock_host(source: &str) -> (wasmtime::Store<AsyncTestHost>, wasmtime::Instance) {
    use wasmtime::{Config, Engine, ExternType, Linker, Module, Store, Val, ValType};

    let wasm = splitscript::compile(source).expect("runtime fixture should compile");
    let mut config = Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    let engine = Engine::new(&config).expect("GC-enabled Wasmtime engine should initialize");
    let module = Module::new(&engine, wasm).expect("Wasmtime should compile generated Wasm GC");
    let mut linker: Linker<AsyncTestHost> = Linker::new(&engine);
    for import in module.imports() {
        let ExternType::Func(function_type) = import.ty() else {
            panic!("SplitScript host ABI imports only functions")
        };
        let module_name = import.module().to_owned();
        let name = import.name().to_owned();
        let host_name = name.clone();
        let result_types = function_type.results().collect::<Vec<_>>();
        linker
            .func_new(
                &module_name,
                &name,
                function_type,
                move |mut caller, parameters, results| {
                    for (result, ty) in results.iter_mut().zip(&result_types) {
                        *result = match ty {
                            ValType::I32 => Val::I32(0),
                            ValType::I64 => Val::I64(0),
                            ValType::F32 => Val::F32(0),
                            ValType::F64 => Val::F64(0),
                            ty => panic!("mock host does not return `{ty}`"),
                        };
                    }
                    match host_name.as_str() {
                        "process_attach" => results[0] = Val::I64(1),
                        "process_is_open" => {
                            results[0] = Val::I32(i32::from(caller.data().process_open));
                        }
                        "timer_get_state" => {
                            results[0] = Val::I32(caller.data().timer_state);
                        }
                        "timer_start" => caller.data_mut().timer_state = 1,
                        "timer_reset" => caller.data_mut().timer_state = 0,
                        "clock_time_get" => {
                            let pointer = parameters[2].unwrap_i32() as usize;
                            let timestamp = caller.data().monotonic_nanoseconds.to_le_bytes();
                            let memory = caller
                                .get_export("memory")
                                .and_then(wasmtime::Extern::into_memory)
                                .expect("generated modules export memory");
                            memory
                                .write(&mut caller, pointer, &timestamp)
                                .expect("clock output should belong to guest memory");
                        }
                        "process_get_module_address" => {
                            caller.data_mut().module_lookups += 1;
                            results[0] = Val::I64(0x1000);
                        }
                        "process_get_module_size" => results[0] = Val::I64(0x200),
                        "process_read" => {
                            let address = parameters[1].unwrap_i64();
                            caller.data_mut().process_reads.push(address as u64);
                            if address == 0x7fff_0000 && caller.data().fail_scene_read
                                || address == 0x7fff_0004 && caller.data().fail_entities_read
                            {
                                return Ok(());
                            }
                            let pointer = parameters[2].unwrap_i32() as usize;
                            let length = parameters[3].unwrap_i32() as usize;
                            let value = match address {
                                0x7fff_0000 => Some(caller.data().raw_scene),
                                0x7fff_0004 => Some(caller.data().raw_entities),
                                _ => None,
                            };
                            let mapped = value
                                .is_none()
                                .then(|| {
                                    caller
                                        .data()
                                        .memory_regions
                                        .iter()
                                        .find_map(|(base, bytes)| {
                                            let offset =
                                                u64::try_from(address).ok()?.checked_sub(*base)?
                                                    as usize;
                                            let end = offset.checked_add(length)?;
                                            (end <= bytes.len())
                                                .then(|| bytes[offset..end].to_vec())
                                        })
                                })
                                .flatten();
                            let memory = caller
                                .get_export("memory")
                                .and_then(wasmtime::Extern::into_memory)
                                .expect("generated modules export memory");
                            if let Some(value) = value {
                                memory
                                    .write(&mut caller, pointer, &value.to_le_bytes()[..length])
                                    .expect("process-read output should belong to guest memory");
                            } else if let Some(mapped) = mapped {
                                memory
                                    .write(&mut caller, pointer, &mapped)
                                    .expect("process-read output should belong to guest memory");
                            } else {
                                return Ok(());
                            }
                            results[0] = Val::I32(1);
                        }
                        "runtime_print_message" => {
                            let pointer = parameters[0].unwrap_i32() as usize;
                            let length = parameters[1].unwrap_i32() as usize;
                            let memory = caller
                                .get_export("memory")
                                .and_then(wasmtime::Extern::into_memory)
                                .expect("generated modules export memory");
                            let mut bytes = vec![0; length];
                            memory
                                .read(&caller, pointer, &mut bytes)
                                .expect("print range should belong to guest memory");
                            caller
                                .data_mut()
                                .messages
                                .push(String::from_utf8(bytes).expect("printed strings are UTF-8"));
                        }
                        _ => {}
                    }
                    Ok(())
                },
            )
            .expect("mock host import should be unique");
    }
    let mut store = Store::new(
        &engine,
        AsyncTestHost {
            process_open: true,
            timer_state: 0,
            monotonic_nanoseconds: 0,
            messages: Vec::new(),
            memory_regions: Vec::new(),
            module_lookups: 0,
            process_reads: Vec::new(),
            raw_scene: 1,
            raw_entities: 7,
            fail_scene_read: false,
            fail_entities_read: false,
        },
    );
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("generated module should instantiate against the mock ABI");
    instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .unwrap()
        .call(&mut store, ())
        .unwrap();
    (store, instance)
}

#[test]
fn timer_lifecycle_actions_observe_transitions_once_while_detached() {
    let source = r#"
        state "missing.exe" {}

        onStart {
            print("started")
        }

        onReset {
            print("reset")
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    store.data_mut().process_open = false;
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(
        store.data().messages.is_empty(),
        "the first sample is a baseline"
    );

    store.data_mut().timer_state = 1;
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started"]);

    store.data_mut().timer_state = 0;
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started", "reset"]);
}

#[test]
fn attempt_scoped_storage_is_hidden_until_start_and_cleared_after_reset() {
    let source = r#"
        let label
        state "game.exe" {}

        onStart {
            label = "ready"
            print("started")
        }

        onReset {
            print(label)
        }

        gameTime {
            print(label)
            return None
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    // Loading while a timer is already active establishes a baseline rather
    // than inventing an `onStart` event. Attempt-dependent actions must not
    // observe the backend's empty storage in this state.
    store.data_mut().timer_state = 1;
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    // Returning to not-running does not invoke an attempt-dependent onReset
    // for an attempt the script never observed starting.
    store.data_mut().timer_state = 0;
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    // A real transition initializes the attempt before later tick actions.
    store.data_mut().timer_state = 1;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started", "ready"]);

    // onReset sees the still-live attempt value; storage is cleared only
    // after the callback completes.
    store.data_mut().timer_state = 0;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started", "ready", "ready"]);
}

#[test]
fn timer_state_monitor_is_absent_without_lifecycle_or_decision_actions() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                print("attached")
            }
        "#,
    )
    .expect("a script without timer behavior should compile");
    let imports = wasmparser::Parser::new(0)
        .parse_all(&wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            wasmparser::Payload::ImportSection(section) => Some(section),
            _ => None,
        })
        .flat_map(|section| section.into_imports().filter_map(Result::ok))
        .map(|import| import.name.to_owned())
        .collect::<Vec<_>>();

    assert!(!imports.iter().any(|name| name == "timer_get_state"));
}

#[test]
fn script_timer_decisions_are_observed_on_the_following_update() {
    let source = r#"
        state "game.exe" {}

        onStart {
            print("started")
        }

        onReset {
            print("reset")
        }

        start {
            return true
        }

        reset {
            return true
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    // The first update establishes both the timer and state baselines. The
    // second requests a start, but must not invoke `onStart` directly.
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());
    assert_eq!(store.data().timer_state, 1);

    // The next update observes that start exactly once, then requests a reset.
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started"]);
    assert_eq!(store.data().timer_state, 0);

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["started", "reset"]);
}

#[test]
fn state_snapshot_reuses_shared_module_and_pointer_prefixes() {
    let source = r#"
        state "game.exe" {
            health: u32 at "game.dll", 0x20, 0x8, 0x4;
            flags: u16 at "game.dll", 0x20, 0x8, 0x6;
            mode: u8 at "game.dll", 0x30, 0x8;
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    store.data_mut().memory_regions = vec![
        (0x1020, 0x2000u64.to_le_bytes().to_vec()),
        (0x1030, 0x4000u64.to_le_bytes().to_vec()),
        (0x2008, 0x3000u64.to_le_bytes().to_vec()),
        (0x3004, vec![42, 0, 7, 0]),
        (0x4008, vec![3]),
    ];

    instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap()
        .call(&mut store, ())
        .unwrap();

    assert_eq!(store.data().module_lookups, 1);
    let mut reads = store.data().process_reads.clone();
    reads.sort_unstable();
    assert_eq!(reads, [0x1020, 0x1030, 0x2008, 0x3004, 0x3006, 0x4008]);

    // Locals belong to one update invocation, not to the attachment. A later
    // snapshot must follow replaced pointers while still sharing that tick's
    // common work.
    store.data_mut().module_lookups = 0;
    store.data_mut().process_reads.clear();
    store.data_mut().memory_regions = vec![
        (0x1020, 0x5000u64.to_le_bytes().to_vec()),
        (0x1030, 0x4000u64.to_le_bytes().to_vec()),
        (0x5008, 0x6000u64.to_le_bytes().to_vec()),
        (0x6004, vec![99, 0, 11, 0]),
        (0x4008, vec![4]),
    ];
    instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap()
        .call(&mut store, ())
        .unwrap();

    assert_eq!(store.data().module_lookups, 1);
    let mut reads = store.data().process_reads.clone();
    reads.sort_unstable();
    assert_eq!(reads, [0x1020, 0x1030, 0x4008, 0x5008, 0x6004, 0x6006]);
}

#[test]
fn shared_pointer_prefixes_remain_lazy_for_inactive_layout_fields() {
    let source = r#"
        enum Edition {
            Active,
            Inactive,
        }

        state "game.exe" {
            layout {
                edition: Edition,
            }

            active: u8 at 0x9000;
            if layout.edition == Edition.Inactive {
                dormantA: u8 at "unused.dll", 0x20, 0x4;
                dormantB: u8 at "unused.dll", 0x20, 0x8;
            }
        }

        onAttach {
            return Layout { edition: Edition.Active }
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    store.data_mut().memory_regions = vec![(0x9000, vec![1])];
    instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap()
        .call(&mut store, ())
        .unwrap();

    assert_eq!(store.data().module_lookups, 0);
    assert_eq!(store.data().process_reads, [0x9000]);
}

#[test]
fn float_display_matches_zmij_for_special_boundaries_and_sampled_bits() {
    use std::fmt::Write as _;

    let mut source = String::from("state \"game.exe\" {}\nsetup {\n");
    let mut expected = Vec::new();
    let mut buffer = zmij::Buffer::new();
    let f32_bits = [
        0x0000_0000,
        0x8000_0000,
        0x0000_0001,
        0x007f_ffff,
        0x0080_0000,
        0x3f80_0000,
        0x3fc0_0000,
        0x7f7f_ffff,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_0000,
    ];
    for bits in f32_bits {
        writeln!(source, "print(f32.fromBits(0x{bits:08x}u32))").unwrap();
        expected.push(buffer.format(f32::from_bits(bits)).to_owned());
    }
    let f64_bits = [
        0x0000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
        0x000f_ffff_ffff_ffff,
        0x0010_0000_0000_0000,
        0x3ff0_0000_0000_0000,
        0x3ff8_0000_0000_0000,
        0x7fef_ffff_ffff_ffff,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff8_0000_0000_0000,
    ];
    for bits in f64_bits {
        writeln!(source, "print(f64.fromBits(0x{bits:016x}u64))").unwrap();
        expected.push(buffer.format(f64::from_bits(bits)).to_owned());
    }
    let mut state = 0x4d59_5df4_d0f3_3173u64;
    for _ in 0..32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits32 = state as u32;
        writeln!(source, "print(f32.fromBits(0x{bits32:08x}u32))").unwrap();
        expected.push(buffer.format(f32::from_bits(bits32)).to_owned());
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        writeln!(source, "print(f64.fromBits(0x{state:016x}u64))").unwrap();
        expected.push(buffer.format(f64::from_bits(state)).to_owned());
    }
    source.push_str("}\n");

    let (store, _) = execute_with_mock_host(&source);
    assert_eq!(store.data().messages, expected);
}

#[test]
fn computed_infinity_can_be_converted_to_a_string_explicitly() {
    let source = r#"
        state "game.exe" {}

        setup {
            let inf = 1.0 / 0.0
            let infStr = inf.toString()
            print(infStr)
        }
    "#;

    let (store, _) = execute_with_mock_host(source);
    assert_eq!(store.data().messages, ["inf"]);
}

#[test]
fn closed_pure_global_initializers_execute_before_setup() {
    let source = r#"
        fn increment(value: u32) -> u32 {
            return value + 1
        }

        let answer: u32 = {
            let base = 40
            increment(increment(base))
        }
        let labels = ["ready", "go"]

        state "game.exe" {}

        setup {
            print(answer)
            print(labels)
        }
    "#;

    let (store, _) = execute_with_mock_host(source);
    assert_eq!(
        store.data().messages,
        ["42", "[\n    \"ready\",\n    \"go\",\n]"]
    );
}

#[test]
fn floating_point_constants_round_trip_through_display_and_parse() {
    let source = r#"
        state "game.exe" {}

        setup {
            let inf = f32.positiveInfinity
            let infStr = inf.toString()
            let infAgain: f64 = infStr.parse() else 0.0
            print(infAgain)
        }
    "#;

    let (store, _) = execute_with_mock_host(source);
    assert_eq!(store.data().messages, ["inf"]);
}

#[test]
fn integer_string_parsing_validates_and_respects_inferred_limits() {
    let source = r#"
        state "game.exe" {}

        setup {
            let minimum: i8 = "-128".parse() else 0
            let maximum: u64 = "18446744073709551615".parse() else 0
            let overflow: u8 = "256".parse() else 7
            print(minimum)
            print(maximum)
            print(overflow)
        }
    "#;

    let (store, _) = execute_with_mock_host(source);
    assert_eq!(store.data().messages, ["-128", "18446744073709551615", "7"]);
}

#[test]
fn floating_point_constants_preserve_their_width_and_ieee_values() {
    let source = r#"
        state "game.exe" {}

        setup {
            let narrowNaN = f32.NaN.isNaN()
            let narrowPositive = f32.positiveInfinity.toBits() == 0x7f800000u32
            let narrowNegative = f32.negativeInfinity.toBits() == 0xff800000u32
            let wideNaN = f64.NaN.isNaN()
            let widePositive = f64.positiveInfinity.toBits()
                == 0x7ff0000000000000u64
            let wideNegative = f64.negativeInfinity.toBits()
                == 0xfff0000000000000u64
            print(`{narrowNaN}:{narrowPositive}:{narrowNegative}:{wideNaN}:{widePositive}:{wideNegative}`)
        }
    "#;

    let (store, _) = execute_with_mock_host(source);
    assert_eq!(store.data().messages, ["true:true:true:true:true:true"]);
}

#[test]
fn closures_execute_through_typed_function_references_and_capture_values() {
    let source = r#"
        state "game.exe" {}

        fn apply(value: u32, transform: (u32) -> u32) -> u32 {
            return transform(value)
        }

        whileAttached {
            let offset = 3u32
            let addOffset = value => value + offset
            print(addOffset(4))
            print(apply(5, value => value * 2))
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["7", "10"]);
}

#[test]
fn higher_order_helpers_specialize_inferred_callable_parameters() {
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
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["5", "ready!"]);
}

#[test]
fn closures_declared_inside_generic_helpers_are_specialized_per_call() {
    let source = r#"
        state "game.exe" {}

        fn describe(value) {
            let render = item => `{value}: {item}`
            return render(value)
        }

        fn doubled(value) {
            let apply = () => {
                value += value
                return value
            }
            return apply()
        }

        whileAttached {
            print(describe(4u32))
            print(describe("ready"))
            print(doubled(3u32))
            print(doubled(5u64))
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["4: 4", "ready: ready", "6", "10"]);
}

#[test]
fn async_closures_declared_inside_generic_helpers_have_specialized_frames() {
    let source = r#"
        state "game.exe" {}

        fn afterTick(value) {
            let delayed = () => {
                await nextTick()
                return value
            }
            return await delayed()
        }

        onAttach {
            print(await afterTick(7u32))
            print(await afterTick("ready"))
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..12 {
        update.call(&mut store, ()).unwrap();
    }

    assert_eq!(store.data().messages, ["7", "ready"]);
}

#[test]
fn mutable_closure_captures_share_one_cell_with_the_declaring_scope() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let counter = 1u32
            let increment = () => {
                counter += 1
                return counter
            }
            print(increment())
            counter += 4
            print(increment())
            print(counter)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["2", "7", "7"]);
}

#[test]
fn returned_closures_retain_mutable_function_parameters() {
    let source = r#"
        state "game.exe" {}

        fn counterFrom(start: u32) -> () -> u32 {
            return () => {
                start += 1
                return start
            }
        }

        whileAttached {
            let next = counterFrom(8)
            print(next())
            print(next())
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["9", "10"]);
}

#[test]
fn nested_closures_share_the_cell_created_by_their_enclosing_closure() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let makeCounter = () => {
                let counter = 2u32
                return () => {
                    counter += 1
                    return counter
                }
            }
            let next = makeCounter()
            print(next())
            print(next())
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["3", "4"]);
}

#[test]
fn closures_share_mutable_cells_stored_across_async_suspension() {
    let source = r#"
        state "game.exe" {}

        fn counterAfterTick() {
            let counter = 4u32
            await nextTick()
            return () => {
                counter += 1
                return counter
            }
        }

        onAttach {
            let next = await counterAfterTick()
            print(next())
            print(next())
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..5 {
        update.call(&mut store, ()).unwrap();
    }

    assert_eq!(store.data().messages, ["5", "6"]);
}

#[test]
fn async_closure_bodies_use_typed_continuation_frames() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let offset = 3u32
            let afterTick = (value: u32) => {
                value += offset
                await nextTick()
                value += 1
                return value
            }
            print(await afterTick(4))
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..6 {
        update.call(&mut store, ()).unwrap();
    }

    assert_eq!(store.data().messages, ["8"]);
}

#[test]
fn unity_context_discovers_and_snapshots_scenes() {
    let source = r#"
        state Unity ["game.exe"] {
            activeScene = unity.scenes.active();
            loadedScenes = unity.scenes.loaded();
            persistentScene = unity.scenes.persistent();
            playerName = unity.scenes.active()?.find("World/Player")?.name();
        }

        whileAttached {
            if !current.loadedScenes.isEmpty() {
                print(`{current.activeScene.index}:{current.activeScene.name}:{current.loadedScenes[0].name}:{current.persistentScene.name}:{current.playerName}`)
            }
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);

    let mut unity_player = vec![0; 0x200];
    unity_player[0..2].copy_from_slice(&0x5a4du16.to_le_bytes());
    unity_player[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    unity_player[0x80..0x84].copy_from_slice(&0x0000_4550u32.to_le_bytes());
    unity_player[0x98..0x9a].copy_from_slice(&0x020bu16.to_le_bytes());
    let signature = [
        0x48, 0x83, 0xec, 0x20, 0x4c, 0x8b, 0x15, 0, 0, 0, 0, 0x33, 0xf6,
    ];
    unity_player[0x100..0x10d].copy_from_slice(&signature);
    let manager_pointer = 0x1800u64;
    let displacement = (manager_pointer as i64 - 0x110bu64 as i64) as i32;
    unity_player[0x107..0x10b].copy_from_slice(&displacement.to_le_bytes());

    let manager_address = 0x2000u64;
    let active_scene = 0x3000u64;
    let loaded_scene_table = 0x4000u64;
    let scene_path = 0x5000u64;
    let persistent_path = 0x5100u64;
    let root_node = 0x5200u64;
    let world_transform = 0x5300u64;
    let world_object = 0x5400u64;
    let world_native_name = 0x5500u64;
    let world_name = 0x5600u64;
    let children = 0x5700u64;
    let player_transform = 0x5800u64;
    let player_object = 0x5900u64;
    let player_native_name = 0x5a00u64;
    let player_name = 0x5b00u64;
    let mut manager = vec![0; 0x120];
    manager[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    manager[0x28..0x30].copy_from_slice(&loaded_scene_table.to_le_bytes());
    manager[0x48..0x50].copy_from_slice(&active_scene.to_le_bytes());
    manager[0x80..0x88].copy_from_slice(&persistent_path.to_le_bytes());
    manager[0x108..0x10c].copy_from_slice(&(-1i32).to_le_bytes());
    let mut scene = vec![0; 0xa0];
    scene[0x10..0x18].copy_from_slice(&scene_path.to_le_bytes());
    scene[0x98..0x9c].copy_from_slice(&(-1i32).to_le_bytes());
    scene.resize(0xb8, 0);
    scene[0xb0..0xb8].copy_from_slice(&root_node.to_le_bytes());
    let mut path = vec![0; 128];
    let path_text = b"Assets/Scenes/Forest.unity";
    path[..path_text.len()].copy_from_slice(path_text);
    let mut persistent_path_bytes = vec![0; 128];
    let persistent_path_text = b"Assets/Scenes/DontDestroyOnLoad.unity";
    persistent_path_bytes[..persistent_path_text.len()].copy_from_slice(persistent_path_text);
    let mut root_node_bytes = vec![0; 24];
    root_node_bytes[..8].copy_from_slice(&root_node.to_le_bytes());
    root_node_bytes[16..24].copy_from_slice(&world_transform.to_le_bytes());
    let mut world_transform_bytes = vec![0; 0x88];
    world_transform_bytes[0x30..0x38].copy_from_slice(&world_object.to_le_bytes());
    world_transform_bytes[0x70..0x78].copy_from_slice(&children.to_le_bytes());
    world_transform_bytes[0x80..0x88].copy_from_slice(&1u64.to_le_bytes());
    let mut world_object_bytes = vec![0; 0x68];
    world_object_bytes[0x60..0x68].copy_from_slice(&world_native_name.to_le_bytes());
    let mut player_transform_bytes = vec![0; 0x88];
    player_transform_bytes[0x30..0x38].copy_from_slice(&player_object.to_le_bytes());
    let mut player_object_bytes = vec![0; 0x68];
    player_object_bytes[0x60..0x68].copy_from_slice(&player_native_name.to_le_bytes());
    let mut world_name_bytes = vec![0; 128];
    world_name_bytes[..5].copy_from_slice(b"World");
    let mut player_name_bytes = vec![0; 128];
    player_name_bytes[..6].copy_from_slice(b"Player");

    store.data_mut().memory_regions = vec![
        (0x1000, unity_player),
        (manager_pointer, manager_address.to_le_bytes().to_vec()),
        (manager_address, manager),
        (active_scene, scene),
        (loaded_scene_table, active_scene.to_le_bytes().to_vec()),
        (scene_path, path),
        (persistent_path, persistent_path_bytes),
        (root_node, root_node_bytes),
        (world_transform, world_transform_bytes),
        (world_object, world_object_bytes),
        (world_native_name, world_name.to_le_bytes().to_vec()),
        (world_name, world_name_bytes),
        (children, player_transform.to_le_bytes().to_vec()),
        (player_transform, player_transform_bytes),
        (player_object, player_object_bytes),
        (player_native_name, player_name.to_le_bytes().to_vec()),
        (player_name, player_name_bytes),
    ];

    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..12 {
        update.call(&mut store, ()).unwrap();
    }

    assert!(
        store
            .data()
            .messages
            .iter()
            .any(|message| message == "-1:Forest:Forest:DontDestroyOnLoad:Player"),
        "scene snapshots should preserve signed indices and derived names: {:?}",
        store.data().messages,
    );
}

#[test]
fn on_attach_preserves_locals_across_awaits() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let beforeOnly: u32 = 3
            if beforeOnly == 3 { print("before") }
            let expected: u32 = 7
            let overwritten: u32 = 1
            await process.module("GameAssembly.dll")
            let afterOnly: u32 = 9
            overwritten = 2
            if expected == 7 && overwritten == 2 && afterOnly == 9 {
                print("ready")
            }
            let unusedModule = await process.module("Unused.dll")
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("onAttach should support ordinary statements");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("Wasm lowering should expose the onAttach body");
    assert_eq!(
        body.cancellation_region,
        Some(splitscript::compiler::wasm_ir::CancellationRegion::ProcessLifetime)
    );
    let action = &checked.syntax().actions[0];
    let splitscript::compiler::ast::Stmt::Variable(before_only) = &action.body.statements[0] else {
        panic!("expected beforeOnly");
    };
    let splitscript::compiler::ast::Stmt::Variable(expected) = &action.body.statements[2] else {
        panic!("expected expected");
    };
    let splitscript::compiler::ast::Stmt::Variable(overwritten) = &action.body.statements[3] else {
        panic!("expected overwritten");
    };
    let splitscript::compiler::ast::Stmt::Variable(after_only) = &action.body.statements[5] else {
        panic!("expected afterOnly");
    };
    let splitscript::compiler::ast::Stmt::Variable(unused_module) = &action.body.statements[8]
    else {
        panic!("expected unusedModule await binding");
    };
    assert!(matches!(
        unused_module.value.as_ref().unwrap().kind,
        splitscript::compiler::ast::ExprKind::Suspend {
            mode: splitscript::compiler::ast::SuspensionMode::Await,
            ..
        }
    ));
    assert!(body.frame_values.contains(&expected.id));
    assert!(!body.frame_values.contains(&before_only.id));
    assert!(!body.frame_values.contains(&overwritten.id));
    assert!(!body.frame_values.contains(&after_only.id));
    assert!(!body.frame_values.contains(&unused_module.id));
    assert!(matches!(
        body.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::Suspend { .. }
    ));
    let splitscript::compiler::wasm_ir::Terminator::Suspend {
        cancellation,
        live_values,
        continuation,
        ..
    } = &body.entry.terminator
    else {
        unreachable!()
    };
    assert_eq!(
        *cancellation,
        Some(splitscript::compiler::wasm_ir::CancellationRegion::ProcessLifetime)
    );
    assert!(live_values.contains(&expected.id));
    assert!(continuation.statements.iter().any(|statement| matches!(
        statement,
        splitscript::compiler::wasm_ir::Statement::If { .. }
    )));
    let splitscript::compiler::wasm_ir::Terminator::Suspend {
        cancellation,
        live_values,
        ..
    } = &continuation.terminator
    else {
        unreachable!()
    };
    assert_eq!(
        *cancellation,
        Some(splitscript::compiler::wasm_ir::CancellationRegion::ProcessLifetime)
    );
    assert!(!live_values.contains(&before_only.id));
    assert!(!live_values.contains(&expected.id));
    assert!(!live_values.contains(&overwritten.id));
    assert!(!live_values.contains(&after_only.id));
    assert!(!live_values.contains(&unused_module.id));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("GC continuation frame should validate");
}

#[test]
fn awaited_intrinsics_plan_only_their_required_scratch_locals() {
    use splitscript::{
        compiler::ast::ActionKind,
        compiler::wasm_ir::{BodyOwner, LocalPurpose},
    };

    let next_tick = splitscript::check(
        splitscript::parse(
            r#"
                state "game.exe" {}
                onAttach { await nextTick() }
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let next_tick_ir = splitscript::lower_wasm(&next_tick);
    let next_tick_body = next_tick_ir
        .body(BodyOwner::Action(ActionKind::OnAttach))
        .unwrap();
    assert!(
        next_tick_body
            .locals
            .iter()
            .all(|local| !matches!(local.purpose, LocalPurpose::IntrinsicScratch { .. }))
    );

    let module = splitscript::check(
        splitscript::parse(
            r#"
                state "game.exe" {}
                onAttach { let module = await process.module("game.exe") }
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let module_ir = splitscript::lower_wasm(&module);
    let module_body = module_ir
        .body(BodyOwner::Action(ActionKind::OnAttach))
        .unwrap();
    let scratch = module_body
        .locals
        .iter()
        .filter(|local| matches!(local.purpose, LocalPurpose::IntrinsicScratch { .. }))
        .collect::<Vec<_>>();
    assert_eq!(scratch.len(), 2);
    assert!(scratch.iter().all(|local| {
        module.semantics().types().kind(local.ty) == &TypeKind::Builtin(BuiltinType::U64)
    }));
}

#[test]
fn on_attach_lowers_awaits_inside_conditional_branches() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let first = await process.module("First.dll")
            if first.address != 0 {
                print("entered")
                let second = await process.module("Second.dll")
                print(`ready {first.address}:{second.address}`)
            } else {
                print("missing")
            }
            print("finished")
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("onAttach should support await inside a conditional branch");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach should have a lowered body");
    assert_eq!(body.async_state_count, 5);

    let splitscript::compiler::wasm_ir::Terminator::Suspend {
        poll_state,
        resume_state,
        continuation,
        ..
    } = &body.entry.terminator
    else {
        panic!("the first await should terminate the entry state");
    };
    assert_eq!(poll_state.index(), 1);
    assert_eq!(resume_state.index(), 2);
    let Some(splitscript::compiler::wasm_ir::Statement::If { then_block, .. }) =
        continuation.statements.iter().find(|statement| {
            matches!(
                statement,
                splitscript::compiler::wasm_ir::Statement::If { .. }
            )
        })
    else {
        panic!("the first continuation should branch");
    };
    let splitscript::compiler::wasm_ir::Terminator::Suspend {
        poll_state,
        resume_state,
        ..
    } = then_block.terminator
    else {
        panic!("the selected branch should suspend");
    };
    assert_eq!(poll_state.index(), 3);
    assert_eq!(resume_state.index(), 4);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("conditional async state machine should validate");
}

#[test]
fn retry_is_first_class_suspending_control_flow_for_result_expressions() {
    let source = r#"
        state "game.exe" {}

        fn readMarker() {
            return process.read<i32>(0x3000)
        }

        onAttach {
            let marker = retry readMarker()
            print(`marker {marker}`)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("retry should unwrap a user function's inferred Result value");
    let action = &checked.syntax().actions[0];
    let splitscript::compiler::ast::Stmt::Variable(marker) = &action.body.statements[0] else {
        panic!("expected a retry binding");
    };
    assert!(matches!(
        marker.value.as_ref().unwrap().kind,
        splitscript::compiler::ast::ExprKind::Suspend {
            mode: splitscript::compiler::ast::SuspensionMode::Retry,
            ..
        }
    ));
    let marker_type = checked
        .semantics()
        .value_type(marker.id)
        .expect("retry bindings have inferred types");
    assert_eq!(
        checked.semantics().types().kind(marker_type),
        &TypeKind::Builtin(BuiltinType::I32)
    );

    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach should have a lowered body");
    assert!(matches!(
        body.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::Retry { .. }
    ));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("generic retry lowering should produce valid Wasm GC");
}

#[test]
fn retry_rejects_non_result_expressions() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let value = retry 42
            print(value as String)
        }
    "#;
    let error = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect_err("retry should require a Result expression");
    assert!(
        error
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("expects a result value (`T!`)") })
    );
}

#[test]
fn retry_accepts_an_ordinary_block_expression_and_catches_propagation() {
    let source = r#"
        state "game.exe" {}

        fn readTotal() {
            return retry {
                let first = process.read<i32>(0x100)?
                let second = process.read<i32>(0x104)?
                first + second
            }
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("a retry boundary should lift the successful block tail and catch `?`");

    let retry_expression = checked
        .syntax()
        .functions
        .first()
        .and_then(|function| function.body.statements.first())
        .and_then(|statement| match statement {
            splitscript::compiler::ast::Stmt::Expression(splitscript::compiler::ast::Expr {
                kind: splitscript::compiler::ast::ExprKind::Return(Some(expression)),
                ..
            }) => Some(expression.as_ref()),
            _ => None,
        })
        .expect("the helper should return its retry expression");
    let splitscript::compiler::ast::ExprKind::Suspend {
        mode: splitscript::compiler::ast::SuspensionMode::Retry,
        value: operand,
        ..
    } = &retry_expression.kind
    else {
        panic!("expected a retry expression");
    };
    assert!(matches!(
        operand.kind,
        splitscript::compiler::ast::ExprKind::Block(_)
    ));

    let targets = checked
        .typed_hir()
        .expressions()
        .filter_map(|expression| match expression.kind {
            splitscript::compiler::hir::TypedExpressionKind::Propagate { target, .. } => {
                Some(target)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 2);
    assert!(targets.iter().all(|target| matches!(
        target,
        splitscript::compiler::hir::FailureTarget::Retry { expression, .. }
            if *expression == retry_expression.id
    )));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("a block retry with local propagation should produce valid Wasm GC");
}

#[test]
fn retry_catches_throw_but_return_and_break_keep_their_lexical_targets() {
    let source = r#"
        state "game.exe" {}

        fn readOrExit(exitEarly: bool) {
            return loop {
                let value = retry {
                    if exitEarly {
                        break 7
                    }
                    let value = process.read<i32>(0x100)?
                    if value < 0 {
                        throw "negative marker"
                    }
                    value
                }
                break value
            }
        }

        fn returnThroughRetry() {
            return retry {
                if true {
                    return 9
                }
                process.read<i32>(0x104)?
            }
        }

        fn retryExplicitError() -> async i32 {
            return retry Err("not ready")
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("retry should catch failures without capturing lexical exits");
    #[derive(Default)]
    struct ThrowCollector(Vec<splitscript::compiler::hir::FailureTarget>);

    impl splitscript::compiler::hir::TypedVisitor for ThrowCollector {
        fn visit_expression(
            &mut self,
            expression: &splitscript::compiler::hir::TypedExpression,
            program: &splitscript::compiler::hir::TypedProgram,
        ) {
            if let splitscript::compiler::hir::TypedExpressionKind::Throw { target, .. } =
                expression.kind
            {
                self.0.push(target);
            }
            splitscript::compiler::hir::walk_typed_expression(self, expression, program);
        }
    }

    let mut throws = ThrowCollector::default();
    splitscript::compiler::hir::TypedVisitor::visit_program(&mut throws, checked.typed_hir());
    assert_eq!(throws.0.len(), 1);
    assert!(throws.0.iter().all(|target| matches!(
        target,
        splitscript::compiler::hir::FailureTarget::Retry { .. }
    )));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("retry with throw, return, and break should produce valid Wasm GC");
}

#[test]
fn retry_operands_must_not_evaluate_await_or_another_retry() {
    for (source, nested) in [
        (
            r#"
                state "game.exe" {}
                onAttach {
                    let value = retry {
                        await nextTick()
                        process.read<i32>(0x100)?
                    }
                }
            "#,
            "`await`",
        ),
        (
            r#"
                state "game.exe" {}
                onAttach {
                    let value = retry {
                        retry process.read<i32>(0x100)
                    }
                }
            "#,
            "`retry`",
        ),
    ] {
        let diagnostics = splitscript::check(splitscript::lower(
            splitscript::parse(source).expect("the nested suspension probe should parse"),
        ))
        .expect_err("retry attempts must remain synchronous within one tick");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(nested)
                && diagnostic
                    .message
                    .contains("cannot be evaluated inside an `await` or `retry` operand")
        }));
    }
}

#[test]
fn retry_may_synchronously_construct_an_async_value_without_awaiting_it() {
    let source = r#"
        state "game.exe" {}

        fn delayed() {
            await nextTick()
            return 1
        }

        onAttach {
            let value = retry {
                let future = delayed()
                process.read<i32>(0x100)?
            }
            print(value)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("constructing an async value is synchronous until it is awaited");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("an unpolled async value inside retry should produce valid Wasm GC");
}

#[test]
fn retry_restarts_the_complete_operand_once_per_attached_update() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let total = retry {
                print("attempt")
                let scene = process.read<i32>(0x7fff_0000)?
                let entities = process.read<i32>(0x7fff_0004)?
                scene + entities
            }
            print(`ready {total}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    store.data_mut().fail_scene_read = true;
    store.data_mut().fail_entities_read = true;
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["attempt", "attempt"]);

    store.data_mut().fail_scene_read = false;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["attempt", "attempt", "attempt"]);

    store.data_mut().fail_entities_read = false;
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["attempt", "attempt", "attempt", "attempt", "ready 8"]
    );
}

#[test]
fn structural_debug_formats_nested_containers_and_text_unambiguously() {
    let source = r#"
        struct Checkpoint {
            name: String,
            values: [u32],
        }

        enum Status {
            Ready(String),
            Idle,
        }

        struct Label {
            name: String,
        }

        fn Label.debugString() -> String {
            return `debug:{self.name}`
        }

        struct Custom {
            name: String,
        }

        struct Measurement {
            value: f32,
        }

        fn Custom.toString() -> String {
            return `display:{self.name}`
        }

        state "game.exe" {}

        whileAttached {
            print(["forest", "castle\nkeep"])
            let fixed: [u8; 2] = [1, 2]
            print(fixed)
            let present: String? = "gate"
            print(present)
            let absent: u32? = None
            print(absent)
            print(None)
            let success: u32! = 7
            print(success)
            let failure: u32! = Err("bad\tvalue")
            print(failure)
            print(1u32..<3u32)
            print(Checkpoint { name: "start", values: [1, 2] })
            print(Status.Ready("go"))
            let visited = Set.new<String>()
            visited.insert("atrium")
            visited.insert("vault")
            print(visited)
            print(Label { name: "label" })
            print(Custom { name: "custom" })
            print([Custom { name: "nested" }])
            let item: IteratorStep<u32> = Item(4)
            print(item)
            let end: IteratorStep<u32> = End
            print(end)
            print([v"1.2.3.4"])
            print([1.5 as f32, 2.0 as f32])
            let measurement: f64? = 1.25
            print(measurement)
            print(Measurement { value: -0.0 as f32 })
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();

    assert_eq!(
        store.data().messages,
        [
            "[\n    \"forest\",\n    \"castle\\nkeep\",\n]",
            "[\n    1,\n    2,\n]",
            "Some(\n    \"gate\",\n)",
            "None",
            "None",
            "Ok(\n    7,\n)",
            "Err(\n    \"bad\\tvalue\",\n)",
            "1..<3",
            "Checkpoint {\n    name: \"start\",\n    values: [\n        1,\n        2,\n    ],\n}",
            "Status.Ready(\n    \"go\",\n)",
            "Set {\n    \"atrium\",\n    \"vault\",\n}",
            "debug:label",
            "display:custom",
            "[\n    Custom {\n        name: \"nested\",\n    },\n]",
            "Item(\n    4,\n)",
            "End",
            "[\n    1.2.3.4,\n]",
            "[\n    1.5,\n    2.0,\n]",
            "Some(\n    1.25,\n)",
            "Measurement {\n    value: -0.0,\n}",
        ]
    );
}

#[test]
fn structural_debug_bounds_recursive_container_graphs() {
    let source = r#"
        struct Node {
            children: [Node],
        }

        state "game.exe" {}

        whileAttached {
            let children: [Node] = []
            let node = Node { children: children }
            children.push(node)
            print(node)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages.len(), 1);
    assert!(
        store.data().messages[0].contains("<cycle>"),
        "{}",
        store.data().messages[0]
    );
}

#[test]
fn process_fallbacks_and_awaited_module_values_are_typed_and_persistent() {
    let source = r#"
        state ["full.exe", "demo.exe"] {}
        onAttach {
            let module: Module = await process.module("GameAssembly.dll")
            if (module.address == 0x140000000 && module.size == 0x200000u64) {
                print("module ready")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("await should bind a Module value");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("module GC values should validate");
    for expected in [b"full.exe".as_slice(), b"demo.exe".as_slice()] {
        assert!(wasm.windows(expected.len()).any(|bytes| bytes == expected));
    }
}

#[test]
fn await_is_an_expression_inside_member_access_arguments_and_arithmetic() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            print(`base {(await process.module("game.exe")).address}`)
            let value = 1 + retry process.read<u32>(0x1000)
            print(value)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("await and retry should compose inside ordinary expressions");
    let body = splitscript::lower_wasm(&checked)
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body")
        .clone();
    assert_eq!(body.async_state_count, 5);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("nested suspension expressions should produce valid Wasm GC");
}

#[test]
fn operands_before_a_nested_await_are_spilled_before_suspension() {
    use splitscript::compiler::wasm_ir::{BodyOwner, LocalPurpose, Statement, Terminator};

    let source = r#"
        state "game.exe" {}

        fn marker() -> u32 {
            print("marker")
            return 4
        }

        onAttach {
            let total = marker() + retry process.read<u32>(0x1000)
            print(total)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("a side-effecting operand may precede a nested suspension");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");

    let Some(Statement::StoreTemporary { target, .. }) = semantic_statements(&body.entry).next()
    else {
        panic!("the earlier call must be evaluated into a compiler temporary")
    };
    assert!(body.frame_temporaries.contains(target));
    assert!(body.locals.iter().any(
        |local| matches!(local.purpose, LocalPurpose::Temporary(candidate) if candidate == *target)
    ));
    assert!(matches!(body.entry.terminator, Terminator::Retry { .. }));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("evaluation-order spills should produce valid typed continuation storage");
}

#[test]
fn expression_branches_keep_suspensions_inside_the_selected_path() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Statement, Terminator};

    let source = r#"
        state "game.exe" {}
        onAttach {
            let selected = if false {
                retry process.read<u32>(0x1000)
            } else {
                7
            }
            let shortCircuited = false
                && retry process.read<u8>(0x1001) == 1
            print(selected)
            if shortCircuited { print("unexpected") }
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("awaits may occur in expression branches and short-circuit operands");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");

    let Some(Statement::If {
        then_block,
        else_block,
        ..
    }) = semantic_statements(&body.entry).next()
    else {
        panic!("the expression if must remain branch-shaped in the continuation graph")
    };
    assert!(matches!(then_block.terminator, Terminator::Retry { .. }));
    assert!(!matches!(else_block.terminator, Terminator::Retry { .. }));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("branch-local suspensions should produce valid Wasm GC");
}

#[test]
fn suspending_while_conditions_are_reentered_on_every_back_edge() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Terminator};

    let source = r#"
        state "game.exe" {}
        onAttach {
            while retry process.read<u8>(0x1000) == 0 {
                await nextTick()
            }
            print("done")
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("while conditions may suspend");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");
    let Terminator::AsyncWhile { header, .. } = &body.entry.terminator else {
        panic!("a suspending condition requires an async loop header")
    };
    let Terminator::Retry { continuation, .. } = &header.terminator else {
        panic!("the loop header must poll its condition before deciding the iteration")
    };
    assert!(matches!(
        continuation.terminator,
        Terminator::AsyncWhileCondition { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("suspending loop conditions should produce valid Wasm GC");
}

#[test]
fn match_arm_suspensions_are_selected_and_payloads_survive_resumption() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Statement, Terminator};

    let source = r#"
        state "game.exe" {}

        enum Input {
            Value(u32),
            Missing
        }

        onAttach {
            let selected = match Input.Value(3) {
                Input.Value(payload) => retry process.read<u32>(0x1000) + payload,
                Input.Missing => 7
            }
            print(selected)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("match arm values may suspend");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");
    let Some(Statement::Match { arms, .. }) = semantic_statements(&body.entry).next() else {
        panic!("a suspending match must remain branch-shaped")
    };
    assert!(matches!(arms[0].block.terminator, Terminator::Retry { .. }));
    assert!(!matches!(
        arms[1].block.terminator,
        Terminator::Retry { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("match payloads and arm-local suspension should use typed frame storage");
}

#[test]
fn suspending_match_guards_resume_into_the_next_arm_when_false() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Statement, Terminator};

    let source = r#"
        state "game.exe" {}
        onAttach {
            let selected = match 1 {
                1 if retry process.read<u8>(0x1000) == 1 => 10,
                1 => 20,
                _ => 30
            }
            print(selected)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("match guards may suspend");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");
    let Some(Statement::StoreTemporary { .. }) = semantic_statements(&body.entry).next() else {
        panic!("the match input must survive a suspending guard")
    };
    let Some(Statement::Match { arms, .. }) = semantic_statements(&body.entry).nth(1) else {
        panic!("the guarded match remains explicit control flow")
    };
    let Terminator::Retry { continuation, .. } = &arms[0].block.terminator else {
        panic!("the first matching arm polls its guard")
    };
    let Some(Statement::If { else_block, .. }) = semantic_statements(continuation).next() else {
        panic!("guard readiness must branch on the guard result")
    };
    assert!(matches!(
        semantic_statements(else_block).next(),
        Some(Statement::Match { .. })
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("suspending guards should preserve match dispatch in typed continuation states");
}

#[test]
fn suspending_value_fallbacks_only_poll_on_the_failure_path() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Statement, Terminator};

    let source = r#"
        state "game.exe" {}
        onAttach {
            let maybe: u32? = None
            let selected = maybe else retry process.read<u32>(0x1000)
            print(selected)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("fallback values may suspend");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");
    let Some(Statement::Fallback {
        fallback_block,
        success_block,
        ..
    }) = semantic_statements(&body.entry).nth(1)
    else {
        panic!("fallback must remain explicit branch control flow")
    };
    assert!(matches!(
        fallback_block.terminator,
        Terminator::Retry { .. }
    ));
    assert!(!matches!(
        success_block.terminator,
        Terminator::Retry { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("fallback success extraction and suspension should use typed temporaries");
}

#[test]
fn awaited_call_operands_are_captured_once_before_polling() {
    use splitscript::compiler::wasm_ir::{BodyOwner, Statement, Terminator};

    let source = r#"
        state "game.exe" {}

        fn scanStart() -> Address {
            print("capture")
            return 0x1000 as Address
        }

        onAttach {
            let found = await process.scan(
                scanStart(),
                0x100u64,
                sig"48 8B ?? 00"
            )
            print(found)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("await operands may contain ordinary expressions");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach has a lowered body");
    assert!(matches!(
        semantic_statements(&body.entry).next(),
        Some(Statement::StoreTemporary { .. })
    ));
    assert!(matches!(body.entry.terminator, Terminator::Suspend { .. }));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("captured await arguments should remain in typed continuation storage");
}

#[test]
fn source_defined_async_functions_use_typed_nested_frames() {
    let source = r#"
        state "game.exe" {}

        fn loadModule(expected: u32) -> async Module {
            let module = await process.module("game.dll")
            if expected == 7 { print("parameter survived") }
            return module
        }

        fn loadIndirectly() {
            return await loadModule(7)
        }

        onAttach {
            let module = await loadIndirectly()
            print(module.address)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("source-defined futures should be executable");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("typed nested continuation frames should produce valid Wasm GC");
}

#[test]
fn source_future_values_can_be_stored_and_awaited_later() {
    let source = r#"
        state "game.exe" {}

        fn afterTick(value: u32) -> async u32 {
            await nextTick()
            return value
        }

        onAttach {
            let pending = afterTick(42)
            print("created")
            let value = await pending
            print(value)
            let again = await pending
            print(again)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("source future values should be ordinary storable values");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("stored and repeatedly awaited future handles should validate");
}

#[test]
fn intrinsic_future_values_can_be_stored_and_awaited_later() {
    let source = r#"
        state "game.exe" {}

        struct PendingModule {
            operation: async Module
        }

        fn consume(operation: async Module) -> async Module {
            return await operation
        }

        onAttach {
            let pending = PendingModule {
                operation: process.module("game.dll")
            }
            print("created")
            let module = await consume(pending.operation)
            print(module.address)
            let sameModule = await pending.operation
            print(sameModule.address)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("intrinsic future values should be ordinary storable values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("stored intrinsic future handles should validate");
}

#[test]
fn future_race_is_lazy_left_biased_and_supports_every_future_producer() {
    let source = r#"
        state "game.exe" {}

        fn delayed(value: u32) -> async u32 {
            print(`start {value}`)
            await nextTick()
            print(`complete {value}`)
            return value
        }

        fn afterTick() -> async None {
            await nextTick()
        }

        onAttach {
            let first = delayed(1)
            let second = delayed(2)
            print("created")
            await nextTick()
            print("before race")
            print(await future.race([first, second]))

            let closure: () -> async None = () -> async None => {
                await nextTick()
            }
            await future.race([afterTick(), closure(), nextTick()])
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["created"],
        "constructing futures and a race must not poll their bodies"
    );

    for _ in 0..5 {
        update.call(&mut store, ()).unwrap();
    }
    assert_eq!(
        store.data().messages,
        [
            "created",
            "before race",
            "start 1",
            "start 2",
            "complete 1",
            "1"
        ],
        "the first ready array item must win without advancing later items again"
    );
}

#[test]
fn future_race_polls_shared_handles_at_most_once_per_update() {
    let source = r#"
        state "game.exe" {}

        fn counted() -> async u32 {
            print("first poll")
            await nextTick()
            print("second poll")
            return 7
        }

        onAttach {
            let operation = counted()
            print(await future.race([operation, operation]))
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["first poll"]);
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["first poll", "second poll", "7"]);
}

#[test]
fn future_timeout_is_lazy_and_gives_ready_operations_deadline_priority() {
    let source = r#"
        state "game.exe" {}

        fn afterTick() -> async u32 {
            print("operation polled")
            await nextTick()
            return 7
        }

        onAttach {
            let operation = future.timeout(
                afterTick(),
                Duration.fromNanoseconds(10),
            )
            print("timeout created")
            print(await operation else 0)
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    store.data_mut().monotonic_nanoseconds = 100;
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["timeout created", "operation polled"],
        "constructing timeout must not poll before the returned future is awaited"
    );

    store.data_mut().monotonic_nanoseconds = 110;
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["timeout created", "operation polled", "7"],
        "an operation ready exactly at the deadline must win"
    );
}

#[test]
fn future_timeout_supports_intrinsic_and_closure_future_producers() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let closure: () -> async u32 = () -> async u32 => {
                await nextTick()
                return 11
            }

            print(await future.timeout(
                closure(),
                Duration.fromSeconds(1),
            ) else 0)
            await future.timeout(
                nextTick(),
                Duration.fromSeconds(1),
            ) else {
                print("unexpected timeout")
            }
            print("done")
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["11"]);
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["11", "done"]);
}

#[test]
fn on_attach_failure_holds_the_process_until_close_without_running_on_detach() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            print("attempt")
            let marker = process.read<u8>(0x1000)?
            if marker != 7 {
                throw "unsupported process build"
            }
            print("ready")
        }

        onDetach {
            print("detached")
        }
    "#;

    // A propagated read failure rejects this process exactly once. The handle
    // remains retained until close, so discovery cannot immediately select it
    // again and an attachment that never completed owns no onDetach event.
    let (mut failed_store, failed_instance) = execute_with_mock_host(source);
    let failed_update = failed_instance
        .get_typed_func::<(), ()>(&mut failed_store, "update")
        .unwrap();
    failed_update.call(&mut failed_store, ()).unwrap();
    failed_update.call(&mut failed_store, ()).unwrap();
    assert_eq!(failed_store.data().messages, ["attempt"]);
    failed_store.data_mut().process_open = false;
    failed_update.call(&mut failed_store, ()).unwrap();
    assert_eq!(failed_store.data().messages, ["attempt"]);

    // An explicit throw has the same boundary semantics.
    let (mut rejected_store, rejected_instance) = execute_with_mock_host(source);
    rejected_store
        .data_mut()
        .memory_regions
        .push((0x1000, vec![8]));
    let rejected_update = rejected_instance
        .get_typed_func::<(), ()>(&mut rejected_store, "update")
        .unwrap();
    rejected_update.call(&mut rejected_store, ()).unwrap();
    rejected_update.call(&mut rejected_store, ()).unwrap();
    assert_eq!(rejected_store.data().messages, ["attempt"]);

    // A successfully initialized process still receives exactly one onDetach.
    let (mut ready_store, ready_instance) = execute_with_mock_host(source);
    ready_store
        .data_mut()
        .memory_regions
        .push((0x1000, vec![7]));
    let ready_update = ready_instance
        .get_typed_func::<(), ()>(&mut ready_store, "update")
        .unwrap();
    ready_update.call(&mut ready_store, ()).unwrap();
    assert_eq!(ready_store.data().messages, ["attempt", "ready"]);
    ready_store.data_mut().process_open = false;
    ready_update.call(&mut ready_store, ()).unwrap();
    assert_eq!(
        ready_store.data().messages,
        ["attempt", "ready", "detached"]
    );

    // Omitting onAttach makes completion implicit once provider preparation
    // reaches the normal attachment path; it still owns an onDetach event.
    let no_initializer = r#"
        state "game.exe" {}
        onDetach { print("detached") }
    "#;
    let (mut implicit_store, implicit_instance) = execute_with_mock_host(no_initializer);
    let implicit_update = implicit_instance
        .get_typed_func::<(), ()>(&mut implicit_store, "update")
        .unwrap();
    implicit_update.call(&mut implicit_store, ()).unwrap();
    implicit_store.data_mut().process_open = false;
    implicit_update.call(&mut implicit_store, ()).unwrap();
    assert_eq!(implicit_store.data().messages, ["detached"]);

    // Process closure while an initializer is still suspended cancels it but
    // does not synthesize an onDetach for an attachment that never completed.
    let pending_initializer = r#"
        state "game.exe" {}
        onAttach {
            print("pending")
            await process.closed()
        }
        onDetach { print("detached") }
    "#;
    let (mut pending_store, pending_instance) = execute_with_mock_host(pending_initializer);
    let pending_update = pending_instance
        .get_typed_func::<(), ()>(&mut pending_store, "update")
        .unwrap();
    pending_update.call(&mut pending_store, ()).unwrap();
    pending_store.data_mut().process_open = false;
    pending_update.call(&mut pending_store, ()).unwrap();
    assert_eq!(pending_store.data().messages, ["pending"]);
}

#[test]
fn future_timeout_expires_pending_operations_and_flattens_existing_failures() {
    let source = r#"
        state "game.exe" {}

        fn pending() -> async u32 {
            await process.closed()
        }

        fn fails() -> async u32! {
            await nextTick()
            return process.read<u32>(0xdead)
        }

        onAttach {
            let value = await future.timeout(
                pending(),
                Duration.fromNanoseconds(10),
            ) else 99
            print(value)

            let failed: u32! = await future.timeout(
                fails(),
                Duration.fromSeconds(1),
            )
            match failed {
                Ok(value) => print(value),
                Err(error) => print(error),
            }
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    store.data_mut().monotonic_nanoseconds = 50;
    update.call(&mut store, ()).unwrap();
    store.data_mut().monotonic_nanoseconds = 59;
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    store.data_mut().monotonic_nanoseconds = 60;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["99"]);
    store.data_mut().monotonic_nanoseconds = 61;
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["99", "process read failed"],
        "timeout and operand failures should use one ordinary result channel"
    );
}

#[test]
fn non_positive_future_timeouts_allow_one_immediate_poll() {
    let source = r#"
        state "game.exe" {}

        fn immediate() -> async u32 {
            if true {
                return 4
            }
            await process.closed()
        }

        fn pending() -> async u32 {
            await process.closed()
        }

        onAttach {
            print(await future.timeout(immediate(), Duration.zero()) else 0)
            print(await future.timeout(
                pending(),
                Duration.fromNanoseconds(-1),
            ) else 8)
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["4", "8"]);
}

#[test]
fn empty_and_self_referential_future_races_remain_pending() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let operations: [async u32] = []
            let pending = future.race(operations)
            operations.push(pending)
            print("waiting")
            print(await pending)
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    for _ in 0..5 {
        update.call(&mut store, ()).unwrap();
    }
    assert_eq!(store.data().messages, ["waiting"]);
}

#[test]
fn literal_empty_future_races_warn_that_they_never_complete() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let pending: async u32 = future.race([])
            print("waiting")
            print(await pending)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("an empty race is valid even though it never completes");
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::EmptyFutureRace)
        .expect("a visible empty array should explain the forever-pending behavior");
    assert_eq!(warning.severity, splitscript::DiagnosticSeverity::Warning);
    assert!(warning.message.contains("never completes"));

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..3 {
        update.call(&mut store, ()).unwrap();
    }
    assert_eq!(store.data().messages, ["waiting"]);
}

#[test]
fn stored_intrinsic_futures_cover_ticks_captured_arguments_and_method_receivers() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let tick = nextTick()
            let scan = process.scan(0x1000, 0x200, sig"48 8B ?? 89")
            let executable = await process.mainModule()
            let moduleScan = executable.scan(sig"48 8B ?? 89")

            await tick
            let rangedAddress = await scan
            let moduleAddress = await moduleScan
            print(rangedAddress)
            print(moduleAddress)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("all intrinsically suspending calls should create storable future values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("intrinsic future captures and standalone poll bodies should validate");
}

#[test]
fn creating_and_dropping_a_future_does_not_suspend_the_creator() {
    let source = r#"
        state "game.exe" {}

        fn deferred() -> async None {
            print("polled")
            await nextTick()
        }

        fn prepare() {
            let _pending = deferred()
            print("prepared")
        }

        onAttach {
            prepare()
            await process.closed()
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("future creation alone should be synchronous");
    let prepare = checked
        .syntax()
        .functions
        .iter()
        .find(|function| function.name == "prepare")
        .expect("the synchronous creator should be present");
    assert_eq!(
        checked.effects().function(prepare.id).suspension,
        splitscript::compiler::stdlib::SuspensionKind::None
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("a synchronous future-producing expression should validate");

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["prepared"],
        "constructing a source-defined future must not execute any part of its body"
    );
}

#[test]
fn stored_discovery_future_executes_and_is_cancelled_with_its_attachment() {
    let source = r#"
        state "game.exe" {}

        fn discover() -> async Module {
            await nextTick()
            return await process.module("game.dll")
        }

        onAttach {
            let pending = discover()
            print("created")
            let module = await pending
            print(module.address)
            let sameModule = await pending
            print(sameModule.address)
            await process.closed()
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["created"]);

    store.data_mut().process_open = false;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["created"]);

    store.data_mut().process_open = true;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["created", "created"]);

    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["created", "created", "4096", "4096"]
    );
}

#[test]
fn catalog_declared_unary_operators_execute_for_globals_and_methods() {
    let source = r#"
        let negative: i32 = -7
        let ready = !false
        let invertedByte: u8 = !1
        let invertedWord: u16 = !1
        let invertedSigned: i8 = !1
        let inferredComplement = !1

        state "game.exe" {}

        whileAttached {
            let reverse = negative.negate()
            let disabled = ready.not()
            let sum: u8 = 250
            sum += 10
            let product: u16 = 40_000
            product *= 2
            let directSum: u8 = 250 + 10
            let directProduct: u16 = 40_000 * 2
            let negativeByte: i8 = -127
            negativeByte -= 1
            let wrappedNegation = -negativeByte
            let shifted: u8 = 128
            shifted <<= 1
            print(`{negative}:{reverse}:{ready}:{disabled}:{invertedByte}:{invertedWord}:{invertedSigned}:{inferredComplement}:{sum}:{product}:{directSum}:{directProduct}:{wrappedNegation}:{shifted}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        ["-7:7:true:false:254:65534:-2:-2:4:14464:4:14464:-128:0"]
    );
}

#[test]
fn numeric_swap_bytes_preserves_width_signedness_and_bit_patterns() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let unsignedByte = 0x12u8.swapBytes()
            let signedByte = (-2i8).swapBytes()
            let unsignedWord = 0x1234u16.swapBytes()
            let signedWord = (-2i16).swapBytes()
            let unsignedDword = 0x12345678u32.swapBytes()
            let signedDword = (-2i32).swapBytes()
            let unsignedQword = 0x0123456789abcdefu64.swapBytes()
            let signedQword = (-2i64).swapBytes()
            let floatBits = f32.fromBits(0x12345678u32).swapBytes().toBits()
            let doubleBits = f64.fromBits(0x0123456789abcdefu64).swapBytes().toBits()
            print(`{unsignedByte}:{signedByte}:{unsignedWord}:{signedWord}:{unsignedDword}:{signedDword}:{unsignedQword}:{signedQword}:{floatBits}:{doubleBits}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(
        store.data().messages,
        [
            "18:-2:13330:-257:2018915346:-16777217:17279655951921914625:-72057594037927937:2018915346:17279655951921914625"
        ]
    );
}

#[test]
fn current_state_assignment_overrides_this_tick_and_becomes_next_ticks_old_state() {
    let source = r#"
        state "game.exe" {
            scene: i32 at 0x7fff_0000
        }

        fn rejectTransientScene() {
            if current.scene == 7 {
                current.scene = old.scene
            }
        }

        whileAttached {
            rejectTransientScene()
            print(`{old.scene}:{current.scene}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    store.data_mut().raw_scene = 7;
    update.call(&mut store, ()).unwrap();
    store.data_mut().raw_scene = 9;
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["1:1", "1:9"]);
}

#[test]
fn floating_point_square_root_preserves_width_and_ieee_edges() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let narrow = (9.0 as f32).sqrt() == (3.0 as f32)
            let wide = (2.25 as f64).sqrt() == 1.5
            let negative = (-1.0 as f64).sqrt().isNaN()
            let negativeZero = (-0.0 as f32).sqrt().toBits() == 0x80000000u32
            let infinity = f64.fromBits(0x7ff0000000000000u64).sqrt().toBits()
                == 0x7ff0000000000000u64
            print(`{narrow}:{wide}:{negative}:{negativeZero}:{infinity}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["true:true:true:true:true"]);
}

#[test]
fn floating_point_truncation_preserves_width_and_ieee_edges() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let narrow = (3.75 as f32).truncate() == (3.0 as f32)
            let wide = (-3.75 as f64).truncate() == -3.0
            let negativeZero = (-0.5 as f32).truncate().toBits() == 0x80000000u32
            let infinity = f64.fromBits(0x7ff0000000000000u64).truncate().toBits()
                == 0x7ff0000000000000u64
            let nan = f64.fromBits(0x7ff8000000000000u64).truncate().isNaN()
            print(`{narrow}:{wide}:{negativeZero}:{infinity}:{nan}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["true:true:true:true:true"]);
}

#[test]
fn numeric_minimum_and_maximum_preserve_types_and_ieee_edges() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let signed: i8 = (-7i8).min(4)
            let unsigned: u16 = 500u16.max(1000)
            let minZero = (0.0 as f32).min(-0.0).toBits() == 0x80000000u32
            let maxZero = (-0.0 as f64).max(0.0).toBits() == 0u64
            let nan = f32.fromBits(0x7fc00000u32)
            let minNan = nan.min(1.0).isNaN()
            let maxNan = (1.0 as f32).max(nan).isNaN()
            print(`{signed}:{unsigned}:{minZero}:{maxZero}:{minNan}:{maxNan}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["-7:1000:true:true:true:true"]);
}

#[test]
fn signed_absolute_value_preserves_width_and_wrapping_semantics() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let byte: i8 = (-7i8).abs()
            let minimumByte: i8 = (-127i8 - 1).abs()
            let word: i16 = (-300i16).abs()
            let negativeZero = (-0.0 as f32).abs().toBits() == 0u32
            let infinity = f64.fromBits(0xfff0000000000000u64).abs().toBits()
                == 0x7ff0000000000000u64
            let nan = f32.fromBits(0xffc00000u32).abs().isNaN()
            print(`{byte}:{minimumByte}:{word}:{negativeZero}:{infinity}:{nan}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["7:-128:300:true:true:true"]);
}

#[test]
fn numeric_squared_preserves_width_and_ieee_semantics() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let byte: i8 = 12i8.squared()
            let word: u16 = 300u16.squared()
            let narrow = (1.5 as f32).squared() == (2.25 as f32)
            let negativeZero = (-0.0 as f64).squared().toBits() == 0u64
            let infinity = f32.fromBits(0xff800000u32).squared().toBits()
                == 0x7f800000u32
            let nan = f64.fromBits(0x7ff8000000000000u64).squared().isNaN()
            print(`{byte}:{word}:{narrow}:{negativeZero}:{infinity}:{nan}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["-112:24464:true:true:true:true"]);
}

#[test]
fn async_none_completion_is_status_only_but_remains_typed() {
    let source = r#"
        state "game.exe" {}

        fn waitOne() {
            await nextTick()
        }

        onAttach {
            let unit: None = await waitOne()
            if unit == None {
                print("unit ready")
            }
            await process.closed()
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["unit ready"]);
}

#[test]
fn inferred_none_arguments_are_abi_erased_but_still_evaluated() {
    let source = r#"
        state "game.exe" {}

        fn produce() {
            print("produced")
        }

        fn consume(value) {
            if value == value {
                print("consumed")
            }
        }

        whileAttached {
            consume(produce())
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["produced", "consumed"]);
}

#[test]
fn source_futures_flow_through_parameters_and_structs() {
    let source = r#"
        state "game.exe" {}

        struct PendingValue {
            operation: async u32
        }

        fn afterTick(value: u32) -> async u32 {
            await nextTick()
            return value
        }

        fn consume(operation: async u32) -> async u32 {
            return await operation
        }

        onAttach {
            let pending = PendingValue { operation: afterTick(7) }
            let value = await consume(pending.operation)
            print(value)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("async values should use ordinary parameter and struct storage");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("future values passed through aggregate storage should validate");
}

#[test]
fn generic_async_functions_receive_distinct_typed_frames() {
    let source = r#"
        state "game.exe" {}

        fn afterTick(value) {
            await nextTick()
            return value
        }

        onAttach {
            let number = await afterTick(7u32)
            let text = await afterTick("ready")
            print(number)
            print(text)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("generic async functions should specialize their future frames");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("each generic async specialization should have a valid typed result slot");
}

#[test]
fn async_methods_capture_their_receiver_once() {
    let source = r#"
        state "game.exe" {}

        struct Counter {
            value: u32
        }

        fn Counter.afterTick() -> async u32 {
            await nextTick()
            return self.value
        }

        onAttach {
            let pending = Counter { value: 9 }.afterTick()
            let value = await pending
            print(value)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("async methods should retain their implicit receiver");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("the receiver should live in the method's typed future frame");
}

#[test]
fn process_lifetime_futures_cannot_escape_into_globals() {
    let source = r#"
        state "game.exe" {}
        struct Holder { operation: async u32 }
        let pending: Holder? = None
    "#;

    let diagnostics = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect_err("globals must not retain cancelled future frames");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot store a process-lifetime async value")
    }));
}

#[test]
fn recursive_async_functions_have_a_bounded_design_diagnostic() {
    let source = r#"
        state "game.exe" {}

        fn recurse(value: u32) -> async u32 {
            await nextTick()
            if value == 0 { return 0 }
            return await recurse(value - 1)
        }
    "#;

    let diagnostics = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect_err("recursive future allocation needs an explicit language policy");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "async function `recurse` cannot be recursive yet"
    }));
}

#[test]
fn signature_literals_are_typed_validated_and_scannable() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let matchAddress = await module.scan(sig"48 8B ?? B? 00")
            if (matchAddress != 0) {
                print("signature found")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("typed signature scan should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("signature scanner should produce valid Wasm");

    let invalid = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let found = await module.scan(sig"48 8X")
        }
    "#;
    let diagnostics = splitscript::compile(invalid).expect_err("invalid signature must fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("invalid signature character"))
    );
}

#[test]
fn finite_range_scans_find_late_matches_and_complete_with_none_after_exhaustion() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            match await process.scanOnce(0x2000, 600_000u64, sig"AA BB") {
                Some(_) => print("found"),
                None => print("unexpected absence"),
            }
            match await process.scanOnce(0x2000, 600_000u64, sig"CC DD") {
                Some(_) => print("unexpected match"),
                None => print("missing"),
            }
            await process.closed()
        }
    "#;

    let (mut store, instance) = execute_with_mock_host(source);
    let mut bytes = vec![0; 600_000];
    bytes[550_000..550_002].copy_from_slice(&[0xaa, 0xbb]);
    store.data_mut().memory_regions = vec![(0x2000, bytes)];
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    for _ in 0..10 {
        update.call(&mut store, ()).unwrap();
        if store.data().messages.len() == 2 {
            break;
        }
    }

    assert_eq!(store.data().messages, ["found", "missing"]);
    assert!(
        store
            .data()
            .process_reads
            .iter()
            .any(|address| *address > 0x2000 + 512 * 1024),
        "the finite scan must continue beyond its first cooperative window"
    );
    let reads_after_completion = store.data().process_reads.len();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().process_reads.len(), reads_after_completion);
}

#[test]
fn file_version_literals_are_typed_and_checked_at_parse_time() {
    let source = r#"
        state "game.exe" {}
        fn supported(version: FileVersion) {
            return version == v"1.5.0.0"
        }
    "#;
    let wasm = splitscript::compile(source).expect("typed file-version literal should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("file-version equality should produce valid Wasm");

    for (literal, expected) in [
        ("1.2.3", "exactly four decimal components"),
        ("1.two.3.4", "components must be decimal integers"),
        ("1.2.3.65536", "components must fit in `u16`"),
    ] {
        let invalid = format!("state \"game.exe\" {{}}\nfn bad() {{ return v\"{literal}\" }}");
        let diagnostics = splitscript::compile(&invalid).expect_err("invalid version must fail");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "missing `{expected}` diagnostic for {literal}: {diagnostics:#?}"
        );
    }
}

#[test]
fn file_version_literals_are_first_class_match_patterns() {
    let source = r#"
        state "game.exe" {}

        fn versionName(version: FileVersion) -> String {
            return match version {
                v"1.5.0.0" => "supported",
                v"2.0.0.0" if version.major == 2 => "preview",
                _ => "unknown",
            }
        }

        onAttach {
            let module = match v"1.5.0.0" {
                v"1.5.0.0" => await process.mainModule(),
                _ => await process.mainModule(),
            }
            print(module.address)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("file-version patterns should type check");
    assert!(checked.typed_hir().expressions().any(|expression| {
        let splitscript::compiler::hir::TypedExpressionKind::Match { arms, .. } = &expression.kind
        else {
            return false;
        };
        arms.iter().any(|arm| {
            matches!(
                arm.pattern,
                splitscript::compiler::hir::TypedPattern::FileVersion([1, 5, 0, 0])
            )
        })
    }));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("file-version pattern comparisons should produce valid Wasm");
}

#[test]
fn file_version_matches_require_a_wildcard_and_reject_invalid_patterns() {
    let non_exhaustive = r#"
        state "game.exe" {}
        fn supported(version: FileVersion) -> bool {
            return match version { v"1.5.0.0" => true }
        }
    "#;
    let diagnostics =
        splitscript::compile(non_exhaustive).expect_err("file-version matches are open-ended");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("non-exhaustive file-version match")
    }));

    let wrong_type = r#"
        state "game.exe" {}
        fn classify(value: u32) -> bool {
            return match value { v"1.5.0.0" => true, _ => false }
        }
    "#;
    let diagnostics =
        splitscript::compile(wrong_type).expect_err("version patterns require FileVersion");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("FileVersion") && diagnostic.message.contains("u32")
    }));

    let malformed = r#"
        state "game.exe" {}
        fn classify(value: FileVersion) -> bool {
            return match value { v"1.5.0" => true, _ => false }
        }
    "#;
    let diagnostics = splitscript::compile(malformed).expect_err("malformed versions must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("file-version literals require exactly four decimal components")
    }));

    let duplicate = r#"
        state "game.exe" {}
        fn classify(value: FileVersion) -> bool {
            return match value {
                v"1.5.0.0" => true,
                v"1.5.0.0" => false,
                _ => false,
            }
        }
    "#;
    let diagnostics =
        splitscript::compile(duplicate).expect_err("duplicate version arms must fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate match arm"))
    );
}

#[test]
fn string_literals_are_content_based_match_patterns() {
    let source = r#"
        state "game.exe" {}

        fn classify(name: String) -> String {
            return match name {
                "CrazyMachines.exe" => "matched",
                "line\nfeed" => "escaped",
                _ => "missed",
            }
        }

        whileAttached {
            let constructed = `CrazyMachines.{"exe"}`
            print(classify(constructed))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("string patterns should type check");
    assert!(checked.typed_hir().expressions().any(|expression| {
        let splitscript::compiler::hir::TypedExpressionKind::Match { arms, .. } = &expression.kind
        else {
            return false;
        };
        arms.iter().any(|arm| {
            matches!(
                &arm.pattern,
                splitscript::compiler::hir::TypedPattern::String(value)
                    if value == "CrazyMachines.exe"
            )
        })
    }));

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    update.call(&mut store, ()).unwrap();
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["matched"]);
}

#[test]
fn string_matches_require_a_wildcard_and_reject_invalid_or_duplicate_patterns() {
    let non_exhaustive = r#"
        state "game.exe" {}
        fn classify(value: String) -> bool {
            return match value { "yes" => true }
        }
    "#;
    let diagnostics =
        splitscript::compile(non_exhaustive).expect_err("string matches are open-ended");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("non-exhaustive string match") })
    );

    let wrong_type = r#"
        state "game.exe" {}
        fn classify(value: u32) -> bool {
            return match value { "yes" => true, _ => false }
        }
    "#;
    let diagnostics = splitscript::compile(wrong_type).expect_err("string patterns require String");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("String") && diagnostic.message.contains("u32")
    }));

    let duplicate = r#"
        state "game.exe" {}
        fn classify(value: String) -> bool {
            return match value {
                "line\nfeed" => true,
                "line\nfeed" => false,
                _ => false,
            }
        }
    "#;
    let diagnostics = splitscript::compile(duplicate)
        .expect_err("equivalent decoded string patterns must be duplicates");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate match arm"))
    );
}

#[test]
fn explicit_process_reads_and_pointer_following_work_in_sync_and_async_code() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let table = await process.scan(module.address, module.size, sig"48 8B ?? 00")
            let target = retry process.readRelative32(table + 0x3)
            let object = retry process.follow(module.address, [0x10i64, 0x28i64])
            let kind = retry process.read<u32>(object + 0x8)
            if (target != 0 && kind == 7u32 && (process.read<bool>(object + 0xc) else false)) {
                print("object ready")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("typed reads and pointer follow should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("typed process operations should produce valid Wasm");

    let mixed = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let wrong: u64 = module.address
        }
    "#;
    let diagnostics = splitscript::compile(mixed).expect_err("addresses must remain nominal");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("expected `u64`, found `address`")
            || diagnostic.message.contains("does not match expected type")
            || diagnostic.message.contains("types do not match")
    }));
}

#[test]
fn result_mismatches_use_source_types_and_explain_unwrapping() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn someFn() {
                return process.read(0x100)
            }

            whileAttached {
                let value: u32 = someFn()
            }
        "#,
    )
    .expect_err("a fallible function call cannot be assigned directly to u32");

    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("fallible value `u32!` must be handled")
                && diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("else fallback"))
                && diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("match value"))
                && diagnostic
                    .notes
                    .iter()
                    .all(|note| !note.contains("postfix `?`"))
        }),
        "{diagnostics:#?}"
    );
    assert!(diagnostics.iter().all(|diagnostic| {
        !diagnostic.message.contains("Result#")
            && !diagnostic.message.contains("Option#")
            && !diagnostic.message.contains("Array#")
    }));
}

#[test]
fn generic_process_read_infers_memory_types_bidirectionally() {
    let source = r#"
        state "game.exe" {
            counter: i16 = process.read(0x1000);
            inferred = process.read(0x1002)
        }

        onAttach {
            let address: address = 0x2000
            let awaited: u32 = retry process.read(address)
        }

        whileAttached {
            let stateUse: u8 = current.inferred
            let wrapped: i32! = process.read(0x3000)
            let unwrapped: u16 = process.read(0x3004) else 0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let generic_reads = checked
        .typed_hir()
        .expressions()
        .filter_map(|expression| match checked.typed_hir().call(expression.id) {
            Some(ResolvedCall::StandardLibrary {
                item,
                type_arguments,
                ..
            }) if *item == StdlibItemId::ProcessRead => Some(type_arguments[0]),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(generic_reads.len(), 5);
    assert!(generic_reads.into_iter().all(|ty| matches!(
        checked.semantics().types().kind(ty),
        TypeKind::Builtin(
            BuiltinType::I16
                | BuiltinType::U8
                | BuiltinType::U32
                | BuiltinType::I32
                | BuiltinType::U16
        )
    )));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("inferred primitive reads should produce valid WebAssembly GC");

    let ambiguous = r#"
        state "game.exe" {}
        whileAttached { let value = process.read(0x1000) }
    "#;
    let errors = splitscript::check(splitscript::parse(ambiguous).unwrap())
        .expect_err("a context-free generic read must be rejected");
    assert!(errors.iter().any(|error| {
        error.message.contains("cannot infer the memory type")
            && error.message.contains("let value: i32!")
            && error.message.contains("process.read<i32>")
    }));
}

#[test]
fn inferred_generic_process_helpers_preserve_constraints_and_effects() {
    let attached = r#"
        state "game.exe" {}
        fn readAt(location) {
            return process.read(location)
        }
        whileAttached {
            let small: u16! = readAt(0x100 as address)
            let large: u32! = readAt(0x200 as address)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(attached).unwrap())
        .expect("the helper result should be generic over MemoryReadable values");
    let parameter = checked
        .semantics()
        .function_type_parameters(checked.syntax().functions[0].id)[0];
    assert!(
        checked
            .semantics()
            .generic_parameter_constraints(parameter)
            .contains(&splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("each concrete read helper should validate");

    let detached = attached.replace("whileAttached", "onDetach");
    let diagnostics = splitscript::compile(&detached)
        .expect_err("the generic helper should retain its attached-process effect");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("requires an attached process") })
    );
}

#[test]
fn memory_readable_structs_have_shared_layouts_and_single_read_lowering() {
    use splitscript::compiler::memory::MemoryTypeLayout;

    let source = r#"
        struct Header {
            tag: u8,
            count: u32,
            flags: u16
        }

        struct Packet {
            version: u16,
            header: Header
        }

        state "game.exe" {
            packet: Packet = process.read(0x1000);
            packetFromPath: Packet at 0x3000
        }

        onAttach {
            let header: Header = retry process.read(0x2000)
            print(header.count as String)
        }

        whileAttached {
            setVariable("Count", current.packet.header.count as String)
            setVariable("Path Count", current.packetFromPath.header.count as String)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let header = checked.syntax().structs[0].id;
    let packet = checked.syntax().structs[1].id;
    let header_layout = checked.memory_layouts().structure(header).unwrap();
    assert_eq!(header_layout.size, 12);
    assert_eq!(header_layout.alignment, 4);
    assert_eq!(
        header_layout
            .fields
            .iter()
            .map(|field| field.offset)
            .collect::<Vec<_>>(),
        [0, 4, 8]
    );
    let packet_layout = checked.memory_layouts().structure(packet).unwrap();
    assert_eq!(packet_layout.size, 16);
    assert_eq!(packet_layout.alignment, 4);
    assert_eq!(
        packet_layout
            .fields
            .iter()
            .map(|field| field.offset)
            .collect::<Vec<_>>(),
        [0, 4]
    );
    assert!(matches!(
        checked.memory_layouts().layout(
            checked
                .semantics()
                .value_type(checked.syntax().state.as_ref().unwrap().fields[0].id)
                .unwrap(),
            checked.semantics()
        ),
        Ok(MemoryTypeLayout::Struct(_))
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("struct reads should deserialize into valid WebAssembly GC structs");

    let invalid = r#"
        struct BadMemory {
            label: String
        }
        state "game.exe" {
            bad: BadMemory = process.read(0x1000)
        }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid).unwrap())
        .expect_err("structs containing managed references are not MemoryReadable");
    assert!(errors.iter().any(|error| {
        error.message.contains("BadMemory.label")
            && error.message.contains("no fixed process-memory layout")
    }));
}

#[test]
fn fixed_arrays_have_exact_memory_layouts_and_use_ordinary_array_methods() {
    use splitscript::compiler::{memory::MemoryTypeLayout, types::TypeKind};

    let source = r#"
        struct Entry {
            id: u16,
            flags: u8
        }

        state "game.exe" {
            bytes: [u8; 6] at 0x1000;
            entries: [Entry; 2] at 0x2000
        }

        fn firstByte(values: [u8]) {
            return values[0]
        }

        whileAttached {
            let first = firstByte(current.bytes)
            let entry = current.entries[1]
            print(`{current.bytes.length()}:{first}:{entry.id}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let fields = &checked.syntax().state.as_ref().unwrap().fields;
    let bytes = checked.semantics().value_type(fields[0].id).unwrap();
    let entries = checked.semantics().value_type(fields[1].id).unwrap();
    assert!(matches!(
        checked.semantics().types().kind(bytes),
        TypeKind::Array {
            length: Some(6),
            ..
        }
    ));
    assert!(matches!(
        checked.memory_layouts().layout(bytes, checked.semantics()),
        Ok(MemoryTypeLayout::FixedArray(layout)) if layout.size == 6 && layout.stride == 1
    ));
    assert!(matches!(
        checked.memory_layouts().layout(entries, checked.semantics()),
        Ok(MemoryTypeLayout::FixedArray(layout)) if layout.size == 8 && layout.stride == 4
    ));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("fixed-array process reads should produce valid Wasm GC arrays");

    let mismatch = r#"
        state "game.exe" {}
        whileAttached { let bytes: [u8; 3] = [1, 2] }
    "#;
    let errors = splitscript::check(splitscript::parse(mismatch).unwrap()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| { error.message.contains("expected 3 array elements, found 2") })
    );

    let incompatible_lengths = r#"
        state "game.exe" {}
        fn takesThree(values: [u8; 3]) {}
        whileAttached {
            let two: [u8; 2] = [1, 2]
            takesThree(two)
        }
    "#;
    let errors = splitscript::check(splitscript::parse(incompatible_lengths).unwrap()).unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.message.contains("[u8; 2]") && error.message.contains("[u8; 3]")
        })
    );

    for (declaration, expected) in [
        ("bytes: [u8] at 0x1000", "use `[T; N]`"),
        (
            "bytes: [u8; 0] at 0x1000",
            "zero-length array does not represent a process-memory read",
        ),
        (
            "bytes: [u8; 4097] at 0x1000",
            "fixed arrays are limited to 4096 elements",
        ),
        (
            "labels: [String; 2] at 0x1000",
            "String` has no fixed process-memory layout",
        ),
    ] {
        let source = format!("state \"game.exe\" {{ {declaration} }}");
        let errors = splitscript::check(splitscript::parse(&source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected `{expected}` in {errors:#?}"
        );
    }
}

#[test]
fn expression_backed_state_fields_use_discovered_addresses_and_rotate_snapshots() {
    let source = r#"
        state "game.exe" {
            points: i32 = process.read<i32>(gameManager.offset(pointsOffset));
            stopped: bool = process.read<bool>(timerInstance.offset(stoppedOffset))
        }

        let gameManager: address = 0
        let timerInstance: address = 0
        let pointsOffset: u32 = 0u32
        let stoppedOffset: u32 = 0u32

        onAttach {
            let module = await process.module("GameAssembly.dll")
            gameManager = module.address
            timerInstance = module.address.add(0x100u64)
            pointsOffset = 0x20u32
            stoppedOffset = 0x40u32
        }

        split {
            return current.points != old.points
        }

        isLoading {
            return current.stopped
        }
    "#;
    let wasm = splitscript::compile(source).expect("dynamic state sources should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("dynamic state sources should produce valid Wasm GC");

    let recursive = r#"
        state "game.exe" {
            points: i32 = current.points
        }
    "#;
    let diagnostics = splitscript::compile(recursive)
        .expect_err("state expressions must not recursively read their snapshot");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot read from its own `current` or `old`")
    }));
}

#[test]
fn state_expression_propagation_and_final_results_share_one_failure_boundary() {
    let source = r#"
        let rejectSceneAddress = false

        fn sceneAddress() -> address! {
            if rejectSceneAddress {
                return Err("scene address is temporarily unavailable")
            }
            return 0x7fff0000
        }

        state "game.exe" {
            scene: i32 = {
                let address = sceneAddress()?
                process.read(address)
            };
            entities: i32 = process.read(process.follow(0x7fff0004, [])?)
        }

        whileAttached {
            print(`{current.scene}:{current.entities}`)
            rejectSceneAddress = !rejectSceneAddress
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("internal propagation and a final result should share the field boundary");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("flattened state-field failure boundaries should produce valid Wasm GC");

    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    store.data_mut().raw_scene = 2;
    store.data_mut().raw_entities = 6;
    update.call(&mut store, ()).unwrap();

    store.data_mut().raw_scene = 3;
    store.data_mut().raw_entities = 5;
    update.call(&mut store, ()).unwrap();

    store.data_mut().raw_scene = 4;
    store.data_mut().raw_entities = 4;
    store.data_mut().fail_entities_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_entities_read = false;

    store.data_mut().raw_scene = 5;
    store.data_mut().raw_entities = 3;
    update.call(&mut store, ()).unwrap();

    store.data_mut().raw_scene = 6;
    store.data_mut().raw_entities = 2;
    store.data_mut().fail_scene_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_scene_read = false;

    assert_eq!(store.data().messages, ["2:6", "2:5", "4:5", "4:3", "4:2"]);
}

#[test]
fn state_field_filters_retain_one_field_without_rejecting_the_snapshot() {
    splitscript::compile(
        r#"
            state "game.exe" {
                scene = if true { 1 } else { 2 };
            }
        "#,
    )
    .expect("expression-backed fields use an ordinary right-hand-side if");
    splitscript::compile(
        r#"
            state "game.exe" {
                scene = 1 if value == 1 { Err("transient") } else { value };
            }
        "#,
    )
    .expect_err("a trailing field if is specific to at pointer paths");

    let source = r#"
        state "game.exe" {
            scene: i32 at 0x7fff0000 if value == 7 || value == 8 {
                Err("transient loading scene")
            } else {
                value
            };
            entities: i32 at 0x7fff0004;
        }

        whileAttached {
            print(`{current.scene}:{current.entities}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());
    store.data_mut().raw_scene = 7;
    store.data_mut().raw_entities = 6;
    update.call(&mut store, ()).unwrap();
    store.data_mut().raw_scene = 5;
    store.data_mut().raw_entities = 5;
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["1:6", "5:5"]);

    store.data_mut().raw_scene = 6;
    store.data_mut().raw_entities = 4;
    store.data_mut().fail_scene_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_scene_read = false;
    store.data_mut().raw_entities = 3;
    store.data_mut().fail_entities_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_entities_read = false;

    assert_eq!(store.data().messages, ["1:6", "5:5", "5:4", "6:4"]);

    store.data_mut().process_open = false;
    update.call(&mut store, ()).unwrap();
    store.data_mut().process_open = true;
    store.data_mut().raw_scene = 7;
    store.data_mut().raw_entities = 4;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["1:6", "5:5", "5:4", "6:4"]);

    store.data_mut().raw_scene = 9;
    store.data_mut().raw_entities = 3;
    update.call(&mut store, ()).unwrap();
    store.data_mut().raw_scene = 10;
    store.data_mut().raw_entities = 2;
    update.call(&mut store, ()).unwrap();

    assert_eq!(store.data().messages, ["1:6", "5:5", "5:4", "6:4", "10:2"]);
}

#[test]
fn failed_state_dependencies_skip_dependents_and_retain_their_values() {
    let source = r#"
        state "game.exe" {
            derived: i32 = {
                print(`derive:{source}`)
                source + 1
            };
            source: i32 at 0x7fff0000;
        }

        whileAttached {
            print(`state:{current.source}:{current.derived}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["derive:1"]);

    store.data_mut().raw_scene = 2;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["derive:1", "derive:2", "state:2:3"]);

    store.data_mut().raw_scene = 7;
    store.data_mut().fail_scene_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_scene_read = false;
    assert_eq!(
        store.data().messages,
        ["derive:1", "derive:2", "state:2:3", "state:2:3"]
    );
}

#[test]
fn state_initialization_requires_one_complete_poll_and_seeds_equal_snapshots() {
    let source = r#"
        state "game.exe" {
            scene: i32 at 0x7fff0000;
            entities: i32 at 0x7fff0004;
        }

        whileAttached {
            print(`{old.scene}:{old.entities}->{current.scene}:{current.entities}`)
        }
    "#;
    let (mut store, instance) = execute_with_mock_host(source);
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();

    store.data_mut().fail_scene_read = true;
    update.call(&mut store, ()).unwrap();
    store.data_mut().fail_scene_read = false;
    store.data_mut().fail_entities_read = true;
    store.data_mut().raw_scene = 2;
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    store.data_mut().fail_entities_read = false;
    store.data_mut().raw_scene = 3;
    store.data_mut().raw_entities = 4;
    update.call(&mut store, ()).unwrap();
    assert!(store.data().messages.is_empty());

    store.data_mut().raw_scene = 5;
    store.data_mut().raw_entities = 6;
    update.call(&mut store, ()).unwrap();
    assert_eq!(store.data().messages, ["3:4->5:6"]);
}

#[test]
fn unity_managed_schemas_are_typed_and_suspension_safe() {
    let source = r#"
        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance from ["Instance", "_instance"];
                i32 currentLevel from ["currentLevel", "_currentScene"];
            }
        }

        state Unity.il2cpp(2020) ["game.exe"] {
            currentLevel: i32 = GameManager.instance?.currentLevel?;
        }

        onAttach {
            print("IL2CPP schema ready")
        }
    "#;
    let wasm = splitscript::compile(source).expect("Unity IL2CPP schema should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("Unity module GC values should produce valid Wasm");
}

#[test]
fn unity_static_data_is_emitted_only_when_required() {
    const METADATA_SIGNATURE: &[u8] = b"global-metadata.dat\0";
    let contains_metadata = |wasm: &[u8]| {
        wasm.windows(METADATA_SIGNATURE.len())
            .any(|window| window == METADATA_SIGNATURE)
    };
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

    let minimal =
        splitscript::compile(r#"state "game.exe" {}"#).expect("minimal script should compile");
    assert!(!contains_metadata(&minimal));
    assert_eq!(defined_functions(&minimal), 2);

    let unity = splitscript::compile(
        r#"
            image "Assembly-CSharp" {
                class GameManager {
                    static GameManager instance;
                    i32 state;
                }
            }
            state Unity.il2cpp(2020) ["game.exe"] {
                state: i32 = GameManager.instance?.state?;
            }
        "#,
    )
    .expect("Unity script should compile");
    assert!(contains_metadata(&unity));
    assert!(defined_functions(&unity) > defined_functions(&minimal));
}

#[test]
fn unreachable_user_functions_and_their_dependencies_are_omitted() {
    const DEAD_STRING: &[u8] = b"DEAD_FUNCTION_SENTINEL";
    let source = r#"
        state "game.exe" {}

        fn liveLeaf() {
            return 7
        }

        fn liveRoot() {
            return liveLeaf()
        }

        fn dead() {
            print("DEAD_FUNCTION_SENTINEL")
        }

        whileAttached {
            let value = liveRoot()
        }
    "#;
    let wasm = splitscript::compile(source).expect("reachable function chain should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("pruned function indices should remain valid");

    assert!(
        !wasm
            .windows(DEAD_STRING.len())
            .any(|window| window == DEAD_STRING)
    );
    let imports = Parser::new(0)
        .parse_all(&wasm)
        .filter_map(|payload| match payload.unwrap() {
            Payload::ImportSection(section) => Some(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(!imports.iter().any(|name| name == "runtime_print_message"));
}

#[test]
fn unreachable_gc_layouts_are_pruned_and_live_layouts_are_remapped() {
    let indexed_type_count = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(
                |payload| match payload.expect("generated Wasm should parse") {
                    Payload::TypeSection(section) => Some(
                        section
                            .into_iter()
                            .map(|group| {
                                group
                                    .expect("generated recursive type groups should parse")
                                    .types()
                                    .len() as u32
                            })
                            .sum::<u32>(),
                    ),
                    _ => None,
                },
            )
            .expect("generated Wasm has a type section")
    };

    let live_only = splitscript::compile(
        r#"
            struct Live { value: i32 }
            state "game.exe" {
                current = Live { value: 1 }
            }
        "#,
    )
    .expect("live aggregate should compile");
    let with_dead_layouts = splitscript::compile(
        r#"
            enum DeadEnum {
                Empty,
                Value(i32)
            }
            struct DeadStruct { value: DeadEnum? }
            struct Live { value: i32 }

            state "game.exe" {
                current = Live { value: 1 }
            }

            fn dead(
                structure: DeadStruct,
                structs: [DeadStruct],
                optional: DeadEnum?,
                result: DeadStruct!
            ) {}
        "#,
    )
    .expect("dead aggregate declarations should not affect live lowering");

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&with_dead_layouts)
        .expect("compacted live GC indices should remain valid");
    assert_eq!(
        indexed_type_count(&with_dead_layouts),
        indexed_type_count(&live_only),
        "unreachable structs, enums, arrays, Options, and Results should emit no indexed types"
    );
}
#[test]
fn managed_instances_compile_as_a_cooperative_typed_future() {
    let source = r#"
image "Assembly-CSharp" {
    class Enemy {
        i32 health;
    }
}

state Unity ["game.exe"] {}

onAttach {
    let enemies: [Enemy.Ref] = await Enemy.instances()
    print(enemies.length())
}
"#;

    let wasm = splitscript::compile(source).expect("managed instance discovery should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("managed instance discovery should produce valid WebAssembly");
}

#[test]
fn managed_instances_require_a_unity_provider_and_zero_arguments() {
    let native = r#"
image "Assembly-CSharp" {
    class Enemy {}
}
state "game.exe" {}
onAttach {
    let enemies = await Enemy.instances()
}
"#;
    let errors = splitscript::compile(native)
        .expect_err("managed instance discovery needs a prepared Unity runtime");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("managed instance discovery requires a Unity state provider")
    }));

    let argument = r#"
image "Assembly-CSharp" {
    class Enemy {}
}
state Unity ["game.exe"] {}
onAttach {
    let enemies = await Enemy.instances(1)
}
"#;
    let errors = splitscript::compile(argument)
        .expect_err("managed instance discovery has no runtime arguments");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("`instances` expects 0 arguments"))
    );
}
