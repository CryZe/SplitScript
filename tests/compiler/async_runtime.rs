//! async runtime integration tests.

use super::*;

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
    let splitscript::compiler::ast::Stmt::Suspend {
        binding: Some(unused_module),
        ..
    } = &action.body.statements[8]
    else {
        panic!("expected unusedModule await binding");
    };
    assert_eq!(body.frame_values, [expected.id]);
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
    assert_eq!(live_values, &[expected.id]);
    assert!(matches!(
        continuation.statements.as_slice(),
        [
            splitscript::compiler::wasm_ir::Statement::Store { .. },
            splitscript::compiler::wasm_ir::Statement::Store { .. },
            splitscript::compiler::wasm_ir::Statement::If { .. }
        ]
    ));
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
    assert!(live_values.is_empty());

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
    let [splitscript::compiler::wasm_ir::Statement::If { then_block, .. }] =
        continuation.statements.as_slice()
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
    let splitscript::compiler::ast::Stmt::Suspend {
        mode: splitscript::compiler::ast::SuspensionMode::Retry,
        binding: Some(marker),
        ..
    } = &action.body.statements[0]
    else {
        panic!("expected a retry binding");
    };
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
    assert!(error.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("expects an expression of type `T!`")
    }));
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
            counter: i16 = process.read(0x1000)
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
            tag: u8
            count: u32
            flags: u16
        }

        record Packet {
            version: u16
            header: Header
        }

        state "game.exe" {
            packet: Packet = process.read(0x1000)
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
            id: u16
            flags: u8
        }

        state "game.exe" {
            bytes: [u8; 6] at 0x1000
            entries: [Entry; 2] at 0x2000
        }

        fn firstByte(values: [u8]) {
            return values.get(0)
        }

        whileAttached {
            let first = firstByte(current.bytes)
            let entry = current.entries.get(1)
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
            points: i32 = process.read<i32>(gameManager.offset(pointsOffset))
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
                Empty
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
