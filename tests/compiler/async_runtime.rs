//! async runtime integration tests.

use super::*;

#[derive(Default)]
struct AsyncTestHost {
    process_open: bool,
    messages: Vec<String>,
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
                        "process_get_module_address" => results[0] = Val::I64(0x1000),
                        "process_get_module_size" => results[0] = Val::I64(0x200),
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
            messages: Vec::new(),
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
        unused_module.value.kind,
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
        marker.value.kind,
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
        splitscript::compiler::wasm_ir::Terminator::Suspend {
            mode: splitscript::compiler::ast::SuspensionMode::Retry,
            ..
        }
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

    let Some(Statement::StoreTemporary { target, .. }) = body.entry.statements.first() else {
        panic!("the earlier call must be evaluated into a compiler temporary")
    };
    assert!(body.frame_temporaries.contains(target));
    assert!(body.locals.iter().any(
        |local| matches!(local.purpose, LocalPurpose::Temporary(candidate) if candidate == *target)
    ));
    assert!(matches!(body.entry.terminator, Terminator::Suspend { .. }));

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
    }) = body.entry.statements.first()
    else {
        panic!("the expression if must remain branch-shaped in the continuation graph")
    };
    assert!(matches!(then_block.terminator, Terminator::Suspend { .. }));
    assert!(!matches!(else_block.terminator, Terminator::Suspend { .. }));

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
    let Terminator::Suspend { continuation, .. } = &header.terminator else {
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
    let Some(Statement::Match { arms, .. }) = body.entry.statements.first() else {
        panic!("a suspending match must remain branch-shaped")
    };
    assert!(matches!(
        arms[0].block.terminator,
        Terminator::Suspend { .. }
    ));
    assert!(!matches!(
        arms[1].block.terminator,
        Terminator::Suspend { .. }
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
    let Some(Statement::StoreTemporary { .. }) = body.entry.statements.first() else {
        panic!("the match input must survive a suspending guard")
    };
    let Some(Statement::Match { arms, .. }) = body.entry.statements.get(1) else {
        panic!("the guarded match remains explicit control flow")
    };
    let Terminator::Suspend { continuation, .. } = &arms[0].block.terminator else {
        panic!("the first matching arm polls its guard")
    };
    let Some(Statement::If { else_block, .. }) = continuation.statements.first() else {
        panic!("guard readiness must branch on the guard result")
    };
    assert!(matches!(
        else_block.statements.first(),
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
    }) = body.entry.statements.get(1)
    else {
        panic!("fallback must remain explicit branch control flow")
    };
    assert!(matches!(
        fallback_block.terminator,
        Terminator::Suspend { .. }
    ));
    assert!(!matches!(
        success_block.terminator,
        Terminator::Suspend { .. }
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
        body.entry.statements.first(),
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

        record PendingModule {
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

        fn prepare() {
            let _pending = nextTick()
            print("prepared")
        }

        onAttach {
            prepare()
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("future creation alone should be synchronous");
    let prepare = &checked.syntax().functions[0];
    assert_eq!(
        checked.effects().function(prepare.id).suspension,
        splitscript::compiler::stdlib::SuspensionKind::None
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("a synchronous future-producing expression should validate");
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
    assert_eq!(store.data().messages, ["produced", "consumed"]);
}

#[test]
fn source_futures_flow_through_parameters_and_records() {
    let source = r#"
        state "game.exe" {}

        record PendingValue {
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
        .expect("async values should use ordinary parameter and record storage");
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

        record Counter {
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
        record Holder { operation: async u32 }
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
fn explicit_process_reads_and_pointer_following_work_in_sync_and_async_code() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let table = await process.scan(module.address, module.size, sig"48 8B ?? 00")
            let target = retry process.readRelative32(table + 0x3)
            let object = retry process.follow(module.address, [0x10u64, 0x28u64])
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

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "cannot use fallible `u32!` where `u32` is required; unwrap it with `else`, propagate it with `?`, or use `retry` in `onAttach`"
    }), "{diagnostics:#?}");
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

    let detached = attached.replace("whileAttached", "onDetached");
    let diagnostics = splitscript::compile(&detached)
        .expect_err("the generic helper should retain its attached-process effect");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("requires an attached process") })
    );
}

#[test]
fn memory_readable_records_have_shared_layouts_and_single_read_lowering() {
    use splitscript::compiler::memory::MemoryTypeLayout;

    let source = r#"
        record Header {
            tag: u8,
            count: u32,
            flags: u16
        }

        record Packet {
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
    let header = checked.syntax().records[0].id;
    let packet = checked.syntax().records[1].id;
    let header_layout = checked.memory_layouts().record(header).unwrap();
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
    let packet_layout = checked.memory_layouts().record(packet).unwrap();
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
        Ok(MemoryTypeLayout::Record(_))
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("record reads should deserialize into valid WebAssembly GC records");

    let invalid = r#"
        record BadMemory {
            label: String
        }
        state "game.exe" {
            bad: BadMemory = process.read(0x1000)
        }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid).unwrap())
        .expect_err("records containing managed references are not MemoryReadable");
    assert!(errors.iter().any(|error| {
        error.message.contains("BadMemory.label")
            && error.message.contains("no fixed process-memory layout")
    }));
}

#[test]
fn fixed_arrays_have_exact_memory_layouts_and_use_ordinary_array_methods() {
    use splitscript::compiler::{memory::MemoryTypeLayout, types::TypeKind};

    let source = r#"
        record Entry {
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
fn unity_il2cpp_attachment_is_typed_and_suspension_safe() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let unity: UnityModule = await Unity.il2cpp(2020u32)
            let image: UnityImage = await unity.image("Assembly-CSharp")
            let gameManager: UnityClass = await image.class("GameManager")
            let instanceOffset: u32 = await gameManager.field("Instance")
            let levelField: UnityField = await gameManager.fieldAny(["currentLevel", "_currentScene"])
            let staticTable: address = await gameManager.staticTable()
            let instance = retry process.read<address>(staticTable.offset(instanceOffset))
            let singleton = await gameManager.staticInstance(["Instance", "_instance"])
            if (unity.assemblies != 0
                && unity.typeInfoTable != 0
                && unity.version == 2020u32
                && unity.pointerSize == 8u32
                && image.address != 0
                && gameManager.address != 0
                && instanceOffset == 0x20u32
                && levelField.offset != 0u32
                && levelField.index < 2u32
                && staticTable != 0
                && instance != 0
                && singleton != 0) {
                print("IL2CPP ready")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("Unity IL2CPP attach should compile");
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
            state "game.exe" {}
            onAttach {
                let unity = await Unity.il2cpp(2020)
                print(`Found {unity.version}`)
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
            record Live { value: i32 }
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
            record DeadRecord { value: DeadEnum? }
            record Live { value: i32 }

            state "game.exe" {
                current = Live { value: 1 }
            }

            fn dead(
                record: DeadRecord,
                records: [DeadRecord],
                optional: DeadEnum?,
                result: DeadRecord!
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
        "unreachable records, enums, arrays, Options, and Results should emit no indexed types"
    );
}
