//! expressions control integration tests.

use super::*;

#[test]
fn discarded_must_use_values_warn_without_failing_compilation() {
    let source = r#"
        state "game.exe" {}

        fn maybeValue() -> i32? {
            return None
        }

        whileAttached {
            "abc".replaceAll("a", "b")
            maybeValue()

            let replaced = "abc".replaceAll("a", "b") else "abc"
            let optional = maybeValue()
            print(replaced)
            print(optional else 0)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("warnings must not reject a valid program");
    assert_eq!(checked.diagnostics().len(), 2);
    assert!(checked.diagnostics().iter().all(|diagnostic| {
        diagnostic.severity == splitscript::DiagnosticSeverity::Warning
            && diagnostic.code == splitscript::DiagnosticCode::MustUse
            && diagnostic.message.starts_with("unused result of")
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("String.replaceAll")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("immutable"))
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("Option")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("inspected"))
    }));

    let (wasm, warnings) = splitscript::compile_with_context_and_options_diagnostics(
        splitscript::CompilerContext::default(),
        source,
        splitscript::CompilerOptions::default(),
    )
    .expect("one-shot compilation should preserve warnings and emit an artifact");
    assert_eq!(warnings, checked.diagnostics());
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("discarded values should lower to valid Wasm");
}

#[test]
fn warning_policy_filters_or_denies_without_changing_semantic_checking() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            "abc".replaceAll("a", "b")
            let unread = 1
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("raw semantic checking should retain non-fatal warnings");
    assert_eq!(checked.diagnostics().len(), 2);

    let mut warnings = splitscript::WarningPolicy::default();
    assert!(warnings.set(
        splitscript::DiagnosticCode::MustUse,
        splitscript::WarningLevel::Allow,
    ));
    assert!(warnings.set(
        splitscript::DiagnosticCode::UnusedBinding,
        splitscript::WarningLevel::Deny,
    ));
    let diagnostics = splitscript::compile_with_options(
        source,
        splitscript::CompilerOptions {
            warnings,
            ..splitscript::CompilerOptions::default()
        },
    )
    .expect_err("a denied warning should reject the configured build");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].code,
        splitscript::DiagnosticCode::UnusedBinding
    );
    assert_eq!(
        diagnostics[0].severity,
        splitscript::DiagnosticSeverity::Error
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("denied by the active warning policy"))
    );
}

#[test]
fn unused_bindings_warn_by_identity_and_support_intentional_underscores() {
    let source = r#"
        state "game.exe" {}

        fn inspect(unusedParameter: i32, usedParameter: i32) {
            let unusedLocal = 1
            let _unusedLocal = 9
            let _intentional = 2
            let writtenOnly = 3
            writtenOnly = 4
            let compound = 1
            compound += 1
            let receiver = "level"
            receiver.byteLength()

            for unusedItem in [1] {
                print("tick")
            }

            let optional: i32? = Some(1)
            print(match optional {
                Some(unusedPayload) => "present",
                None => "absent"
            })
            print(usedParameter)
        }

        onAttach {
            let unusedModule = await process.module("game.exe")
            let _ignoredModule = await process.module("game.exe")
        }

        whileAttached {
            inspect(1, 2)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("unused bindings should be non-fatal warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("unused "))
        .collect::<Vec<_>>();
    assert_eq!(unused.len(), 6, "{unused:#?}");
    assert!(
        unused
            .iter()
            .all(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::UnusedBinding)
    );
    for name in [
        "unusedParameter",
        "unusedLocal",
        "writtenOnly",
        "unusedItem",
        "unusedPayload",
        "unusedModule",
    ] {
        let diagnostic = unused
            .iter()
            .find(|diagnostic| diagnostic.message.contains(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing unused warning for {name}: {unused:#?}"));
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], name);
        assert_eq!(diagnostic.fixes.len(), 1);
        assert_eq!(
            diagnostic.fixes[0].applicability,
            splitscript::FixApplicability::MachineApplicable
        );
    }
    for name in [
        "_intentional",
        "_unusedLocal",
        "_ignoredModule",
        "compound",
        "receiver",
        "usedParameter",
    ] {
        assert!(
            unused
                .iter()
                .all(|diagnostic| !diagnostic.message.ends_with(&format!("`{name}`"))),
            "unexpected unused warning for {name}: {unused:#?}"
        );
    }

    let written = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("`writtenOnly`"))
        .unwrap();
    assert_eq!(written.fixes[0].edits.len(), 2);
    assert_eq!(
        written.fixes[0]
            .edits
            .iter()
            .map(|edit| &source[edit.span.start..edit.span.end])
            .collect::<Vec<_>>(),
        ["writtenOnly", "writtenOnly"]
    );
    let local = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("`unusedLocal`"))
        .unwrap();
    assert_eq!(local.fixes[0].edits[0].replacement, "__unusedLocal");
}

#[test]
fn unused_declarations_follow_reachable_calls_and_global_reads() {
    let source = r#"
        enum LiveKind {
            Active
            Dormant
        }

        record LiveRecord {
            kind: LiveKind
            ignored: i32
            _reserved: i32
        }

        enum DeadKind {
            Inactive
        }

        record DeadRecord {
            kind: DeadKind
        }

        enum _IntentionalEnum {
            Reserved
        }

        record _IntentionalRecord {
            value: i32
        }

        let stateRoot = 1
        let actionRoot = 2
        let deadGlobal = 3
        let _intentionalGlobal = 4

        state "game.exe" {
            value = stateRoot
        }

        fn reachableRoot() {
            reachableLeaf()
            reachableType().kind
        }

        fn reachableLeaf() {
            print(actionRoot)
        }

        fn reachableType() -> LiveRecord {
            return LiveRecord {
                kind: LiveKind.Active,
                ignored: 1,
                _reserved: 2
            }
        }

        fn deadParent() {
            deadLeaf()
        }

        fn deadLeaf() {
            print("dead")
        }

        fn deadRecursive() {
            deadRecursive()
        }

        fn deadTyped(_value: DeadRecord) {}

        fn _intentionalFunction() {
            print("reserved")
        }

        whileAttached {
            reachableRoot()
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("unused declarations should be non-fatal warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused global")
                || diagnostic.message.starts_with("unused function")
                || diagnostic.message.starts_with("unused record")
                || diagnostic.message.starts_with("unused enum")
        })
        .collect::<Vec<_>>();

    assert!(unused.iter().all(|diagnostic| {
        diagnostic.code
            == if diagnostic.message.starts_with("unused record field")
                || diagnostic.message.starts_with("unused enum variant")
            {
                splitscript::DiagnosticCode::UnusedMember
            } else {
                splitscript::DiagnosticCode::UnusedDeclaration
            }
    }));

    for (name, source_name) in [
        ("deadGlobal", "deadGlobal"),
        ("deadParent", "deadParent"),
        ("deadLeaf", "deadLeaf"),
        ("deadRecursive", "deadRecursive"),
        ("deadTyped", "deadTyped"),
        ("DeadRecord", "DeadRecord"),
        ("DeadKind", "DeadKind"),
        ("LiveRecord.ignored", "ignored"),
        ("LiveKind.Dormant", "Dormant"),
    ] {
        let diagnostic = unused
            .iter()
            .find(|diagnostic| diagnostic.message.ends_with(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing unused warning for {name}: {unused:#?}"));
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            source_name
        );
        assert!(diagnostic.fixes.is_empty());
    }
    for name in [
        "stateRoot",
        "actionRoot",
        "reachableRoot",
        "reachableLeaf",
        "reachableType",
        "LiveRecord",
        "LiveKind",
        "LiveRecord.kind",
        "LiveRecord._reserved",
        "LiveKind.Active",
        "_intentionalGlobal",
        "_intentionalFunction",
        "_IntentionalRecord",
        "_IntentionalEnum",
    ] {
        assert!(
            unused
                .iter()
                .all(|diagnostic| !diagnostic.message.ends_with(&format!("`{name}`"))),
            "unexpected unused warning for {name}: {unused:#?}"
        );
    }
}

#[test]
fn structural_equality_observes_complete_record_and_enum_shapes() {
    let source = r#"
        record Pair {
            left: i32
            right: i32
        }

        enum Mode {
            First
            Second
        }

        state "game.exe" {}

        fn recordsEqual(left: Pair, right: Pair) -> bool {
            return left == right
        }

        fn modesEqual(left: Mode, right: Mode) -> bool {
            return left == right
        }

        whileAttached {
            if recordsEqual(
                Pair { left: 1, right: 2 },
                Pair { left: 1, right: 2 }
            ) {}
            if modesEqual(Mode.First, Mode.First) {}
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("structural equality should type check");
    let member_warnings = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused record field")
                || diagnostic.message.starts_with("unused enum variant")
        })
        .collect::<Vec<_>>();
    assert!(member_warnings.is_empty(), "{member_warnings:#?}");
}

#[test]
fn if_expressions_infer_branches_bidirectionally_and_lower_to_wasm() {
    let source = r#"
        enum Selected {
            Number(u16)
            Text(String)
        }

        state "game.exe" {
            selected = if useText {
                Selected.Text("DLC")
            } else {
                Selected.Number(process.read<u16>(0x1234 as address) else 0)
            }
        }

        let useText = false

        fn selectedText(value: Selected) {
            return match value {
                Selected.Number(number) => number as String,
                Selected.Text(text) => text
            }
        }

        whileAttached {
            let inferred: u16 = if useText { 1 } else if !useText { 2 } else { 3 }
            setVariable("Selected", selectedText(current.selected))
            setVariable("Inferred", inferred as String)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("if expressions should compile");
    let lowered = splitscript::lower_wasm(&checked);
    let mut if_expressions = 0;
    for expression in checked.typed_hir().expressions() {
        let splitscript::compiler::hir::TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } = &expression.kind
        else {
            continue;
        };
        let splitscript::compiler::wasm_ir::ExpressionKind::If {
            condition: lowered_condition,
            then_expr: lowered_then,
            else_expr: lowered_else,
        } = &lowered
            .expression(expression.id)
            .expect("if expression should have a Wasm IR plan")
            .kind
        else {
            panic!("expression-valued if must not remain deferred to typed HIR")
        };
        assert_eq!(
            (*lowered_condition, *lowered_then, *lowered_else),
            (*condition, *then_expr, *else_expr)
        );
        if_expressions += 1;
    }
    assert!(
        if_expressions >= 3,
        "nested if expressions should all lower"
    );

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("if expressions should produce valid WebAssembly GC");
}

#[test]
fn if_expressions_require_an_else_and_matching_branch_types() {
    let missing_else = r#"
        state "game.exe" {}
        whileAttached { let value = if true { 1 } }
    "#;
    let errors = splitscript::compile(missing_else).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("needs an `else`"))
    );

    let mismatched = r#"
        state "game.exe" {}
        whileAttached { let value = if true { 1 } else { "one" } }
    "#;
    let errors = splitscript::compile(mismatched).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("types do not match")
                || error.message.contains("constraints"))
    );
}

#[test]
fn while_loops_typecheck_lower_and_validate() {
    let source = include_str!("../while_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("while loops should typecheck");
    let while_attached = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .expect("the fixture has a whileAttached action");
    assert!(while_attached.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            splitscript::compiler::hir::TypedStatementKind::Expression(_)
        )
    }));
    let function = checked
        .typed_hir()
        .function_bodies()
        .next()
        .expect("the fixture has a function");
    assert!(function.body.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            splitscript::compiler::hir::TypedStatementKind::While { .. }
        )
    }));

    let lowered = splitscript::lower_wasm(&checked);
    assert!(lowered.bodies().any(|body| {
        body.entry.statements.iter().any(|statement| {
            matches!(
                statement,
                splitscript::compiler::wasm_ir::Statement::While { .. }
            )
        })
    }));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("while loops should produce valid WebAssembly GC");
}

#[test]
fn while_requires_bool_conditions() {
    let errors = splitscript::compile(r#"state "game.exe" {} whileAttached { while 1 {} }"#)
        .expect_err("while conditions must be bool");
    assert!(errors.iter().any(|error| {
        error.message.contains("types do not match") || error.message.contains("bool")
    }));
}

#[test]
fn on_attach_loops_lower_suspending_back_edges_to_async_states() {
    let source = include_str!("../async_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("await and retry should work inside while loops");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("the fixture has an onAttach body");
    assert!(body.async_state_count >= 15);
    assert!(matches!(
        body.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::AsyncWhile { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("suspending loops should produce valid WebAssembly GC");
}

#[test]
fn for_loops_infer_elements_lower_and_validate() {
    let source = include_str!("../for_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("fixed and general arrays should be iterable");
    let lowered = splitscript::lower_wasm(&checked);

    assert!(lowered.bodies().any(|body| {
        body.entry.statements.iter().any(|statement| {
            matches!(
                statement,
                splitscript::compiler::wasm_ir::Statement::For { .. }
            )
        })
    }));

    let on_attach = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("the fixture has an onAttach body");
    assert!(matches!(
        on_attach.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::AsyncFor { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("sync and suspending for loops should produce valid WebAssembly GC");
}

#[test]
fn for_loops_require_arrays_and_keep_bindings_scoped_and_read_only() {
    let not_array =
        splitscript::compile(r#"state "game.exe" {} whileAttached { for value in 42 {} }"#)
            .expect_err("non-arrays are not iterable");
    assert!(
        not_array
            .iter()
            .any(|error| error.message.contains("expects an array"))
    );

    let assignment = splitscript::compile(
        r#"state "game.exe" {} whileAttached { for value in [1] { value = 2 } }"#,
    )
    .expect_err("loop bindings are read-only");
    assert!(
        assignment
            .iter()
            .any(|error| error.message.contains("cannot assign")
                || error.message.contains("constant"))
    );

    let outside = splitscript::compile(
        r#"state "game.exe" {} whileAttached { for value in [1] {} print(value) }"#,
    )
    .expect_err("loop bindings are lexically scoped");
    assert!(
        outside
            .iter()
            .any(|error| error.message.contains("unknown variable `value`"))
    );
}

#[test]
fn for_loops_infer_array_and_element_types_from_the_body() {
    let source = r#"
        state "game.exe" {}

        fn sum(values) -> u16 {
            let total = 0u16
            for value in values {
                total += value
            }
            return total
        }

        fn emptySum() -> u16 {
            let total = 0u16
            for value in [] {
                total += value
            }
            return total
        }

        fn exactSum(values: [u16; 2]) -> u16 {
            let total = 0u16
            for value in values {
                total += value
            }
            return total
        }

        whileAttached {
            print(sum([1u16, 2u16]) as String)
            print(emptySum() as String)
            print(exactSum([1u16, 2u16]) as String)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("loop-body uses should infer both named and empty-array element types");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("backward-inferred for loops should produce valid Wasm");
}

#[test]
fn break_and_continue_require_loops() {
    for (keyword, expected) in [
        ("break", "`break` is only available inside a loop"),
        ("continue", "`continue` is only available inside a loop"),
    ] {
        let source = format!(r#"state "game.exe" {{}} whileAttached {{ {keyword} }}"#);
        let errors = splitscript::compile(&source).expect_err("loop control needs a loop");
        assert!(errors.iter().any(|error| error.message.contains(expected)));
    }

    for (branch, expected) in [
        ("else break", "`else break` is only available inside a loop"),
        (
            "else continue",
            "`else continue` is only available inside a loop",
        ),
    ] {
        let source = format!(
            r#"state "game.exe" {{}} whileAttached {{ let absent: i32? = None; let value = absent {branch} }}"#
        );
        let errors = splitscript::compile(&source).expect_err("fallback loop control needs a loop");
        assert!(errors.iter().any(|error| error.message.contains(expected)));
    }
}

#[test]
fn compound_assignments_reuse_binary_typing_and_lowering() {
    use splitscript::{
        compiler::ast::{ActionKind, BinaryOp},
        compiler::wasm_ir::{BodyOwner, Statement},
    };

    let source = r#"
        state "game.exe" {}

        let integer = 8u32
        let elapsed = 1.0

        onAttach {
            let attempts = 1u32
            let game = await process.module("game.exe")
            attempts += 1
            print(attempts as String)
        }

        whileAttached {
            integer += 2
            integer -= 1
            integer *= 3
            integer /= 2
            integer %= 7
            integer |= 0x10
            integer &= 0xff
            integer ^= 3
            integer <<= 1
            integer >>= 2

            elapsed += 0.5
            elapsed -= 0.25
            elapsed *= 2.0
            elapsed /= 2.0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let while_attached = lowered
        .body(BodyOwner::Action(ActionKind::WhileAttached))
        .expect("the whileAttached action should have a lowered body");
    assert_eq!(
        while_attached
            .entry
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::Store { op: Some(op), .. } => Some(*op),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::BitOr,
            BinaryOp::BitAnd,
            BinaryOp::BitXor,
            BinaryOp::Shl,
            BinaryOp::Shr,
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
        ]
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("compound assignments should lower to valid numeric Wasm operations");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let enabled = true
            enabled += true
        }
    "#;
    let errors = splitscript::compile(invalid)
        .expect_err("compound arithmetic must reject non-numeric operands");
    assert!(errors.iter().any(|error| {
        error.message.contains("bool") && error.message.contains("does not support this operation")
    }));
}

#[test]
fn numeric_min_max_and_clamp_are_type_directed_and_validate() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let signedByte: i8 = -5
            let unsignedWord: u16 = 500
            let signedWide: i64 = -100
            let unsignedWide: u64 = 100
            let single: f32 = 1.5
            let double: f64 = -2.5

            let a = signedByte.min(2)
            let b = unsignedWord.max(1000)
            let c = signedWide.clamp(-50, 50)
            let d = unsignedWide.clamp(10, 90)
            let eMin = single.min(1.0)
            let e = eMin.max(0.0)
            let f = double.clamp(-1.0, 1.0)
            let inferredInput = 3
            let inferredFromResult: u16 = inferredInput.min(7)

            setVariable("Integers", `{a}:{b}:{c}:{d}:{inferredFromResult}`)
            if e > f as f32 { print("bounded") }
        }
    "#;
    let wasm = splitscript::compile(source).expect("numeric bounds should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("numeric bounds should produce valid WebAssembly");
}

#[test]
fn numeric_bounds_reject_non_numeric_receivers_and_wrong_arity() {
    for source in [
        r#"state "game.exe" {} whileAttached { let value = true; let bounded = value.min(false) }"#,
        r#"state "game.exe" {} whileAttached { let value = "a"; let bounded = value.max("b") }"#,
        r#"state "game.exe" {} whileAttached { let value: u32 = 1; value.clamp(2) }"#,
    ] {
        assert!(splitscript::compile(source).is_err());
    }
}

#[test]
fn runtime_text_outputs_accept_display_values() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            print("tick")
            print(42)
            print(-7i64)
            setVariable("Score", 9u16)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("print and setVariable should display supported values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generic runtime text output should produce valid Wasm");

    for call in ["print(true)", "setVariable(\"Flag\", true)"] {
        let source = format!("state \"game.exe\" {{}} whileAttached {{ {call} }}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("values that cannot be displayed should be rejected");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("does not satisfy the inferred constraints")),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn standard_types_can_supply_source_defined_display_implementations() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let version = v"1.2.3.4"
            print(version)
            setVariable("Version", version)
            let cast = version as String
            let interpolated = `version {version}`
            print(cast)
            print(interpolated)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("every Display entry point should use the catalog implementation");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("source-defined Display calls should produce valid Wasm");
}

#[test]
fn strings_are_gc_values_with_content_equality_length_and_predicates() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let message = "tick"
            if message == "tick"
                && message != "tock"
                && message.byteLength() == 4u32
                && message.contains("ic")
                && message.startsWith("ti")
                && message.endsWith("ck") {
                print(message)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("String values should type check");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("GC String operations should produce valid Wasm");
}

#[test]
fn template_strings_interpolate_strings_castable_values_and_nested_templates() {
    let source = r#"
        state "game.exe" {}

        fn format(name, value: u16, location: address) {
            let count = `{value + 1}`
            return `{name}: {count} @ {location} \{escaped\}`
        }

        onAttach {
            print(format("Score", 41, 0x1234 as address))
        }
    "#;
    let wasm = splitscript::compile(source).expect("template strings should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("template string lowering should produce valid Wasm");
}

#[test]
fn template_strings_reject_values_without_string_casts() {
    let source = r#"
        state "game.exe" {}
        fn format(value: bool) -> String {
            return `value={value}`
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("bool does not implement Display");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not support this operation")
    }));
}

#[test]
fn user_functions_are_typed_and_can_call_forward_declarations() {
    let source = r#"
        state "game.exe" {}

        fn isFinalLevel(level: i32) -> bool {
            return stage(level) == 7
        }

        fn stage(level: i32) -> i32 {
            return (level / 2) + 1
        }

        whileAttached {
            let label = "level"
            if (isFinalLevel(13) && label.byteLength() == 5u32) {
                print(label)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("user functions should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("user function calls should produce valid Wasm");
}

#[test]
fn user_function_and_method_calls_expose_stable_callable_ids() {
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
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let direct_target = checked.syntax().functions[0].id;
    let method_target = checked.syntax().functions[1].id;
    assert_ne!(direct_target, method_target);

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(counter) = &statements[0] else {
        panic!("expected the method receiver binding");
    };
    let splitscript::compiler::ast::Stmt::Variable(direct) = &statements[1] else {
        panic!("expected the direct call binding");
    };
    let splitscript::compiler::ast::Stmt::Variable(method) = &statements[2] else {
        panic!("expected the method call binding");
    };
    assert_eq!(
        checked.semantics().call(direct.value.id),
        Some(&ResolvedCall::UserFunction {
            function: direct_target,
            type_arguments: Vec::new(),
            signature: vec![checked.semantics().function_result(direct_target).unwrap()],
        })
    );
    assert_eq!(
        checked.semantics().call(method.value.id),
        Some(&ResolvedCall::UserMethod {
            function: method_target,
            type_arguments: Vec::new(),
            signature: checked
                .semantics()
                .function_parameter_types(method_target)
                .iter()
                .copied()
                .chain(checked.semantics().function_result(method_target))
                .collect(),
            receiver: ResolvedReceiver::Path {
                root: ResolvedValue::Variable(counter.id),
                members: Vec::new(),
            },
            receiver_type: checked
                .semantics()
                .expression_type(counter.value.id)
                .expect("the method receiver has a semantic type"),
        })
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved user calls should produce valid Wasm");
}

#[test]
fn match_payload_bindings_and_method_receivers_resolve_by_value_id() {
    let source = r#"
        state "game.exe" {}

        record Counter { value: i32 }
        enum MaybeCounter {
            Counter(Counter)
            Empty
        }

        fn Counter.increment() -> i32 {
            return self.value + 1
        }

        fn read(value: MaybeCounter) -> i32 {
            return match value {
                MaybeCounter.Counter(counter) => counter.increment(),
                MaybeCounter.Empty => 0
            }
        }

        whileAttached {
            let result = read(MaybeCounter.Counter(Counter { value: 4 }))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let method_target = checked.syntax().functions[0].id;
    let splitscript::compiler::ast::Stmt::Return {
        value: Some(matched),
        ..
    } = &checked.syntax().functions[1].body.statements[0]
    else {
        panic!("expected the match return expression");
    };
    let splitscript::compiler::ast::ExprKind::Match { arms, .. } = &matched.kind else {
        panic!("expected a match expression");
    };
    let splitscript::compiler::ast::MatchPattern::Enum {
        binding: Some(binding),
        ..
    } = &arms[0].pattern
    else {
        panic!("expected a payload binding");
    };
    assert_eq!(
        checked.semantics().pattern_variant(arms[0].pattern_id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    assert_eq!(
        checked
            .typed_hir()
            .pattern(arms[0].pattern_id)
            .and_then(|pattern| pattern.variant),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    let Some(ResolvedCall::UserMethod {
        function, receiver, ..
    }) = checked.semantics().call(arms[0].value.id)
    else {
        panic!("expected the payload method call to resolve");
    };
    assert_eq!(*function, method_target);
    assert_eq!(
        receiver,
        &ResolvedReceiver::Path {
            root: ResolvedValue::Variable(binding.id),
            members: Vec::new(),
        }
    );

    let lowered = splitscript::lower_wasm(&checked);
    let splitscript::compiler::wasm_ir::ExpressionKind::Match {
        value: lowered_value,
        arms: lowered_arms,
    } = &lowered
        .expression(matched.id)
        .expect("match expression should have a Wasm IR plan")
        .kind
    else {
        panic!("resolved match must not remain deferred to typed HIR")
    };
    let splitscript::compiler::ast::ExprKind::Match {
        value: matched_value,
        ..
    } = &matched.kind
    else {
        unreachable!()
    };
    assert_eq!(*lowered_value, matched_value.id);
    assert_eq!(lowered_arms[0].pattern_id, arms[0].pattern_id);
    let splitscript::compiler::wasm_ir::LoweredPattern::Enum {
        enumeration,
        variant,
        binding: lowered_binding,
    } = lowered_arms[0].pattern
    else {
        panic!("enum patterns should retain their resolved identities")
    };
    assert_eq!(
        enumeration,
        splitscript::compiler::types::EnumTypeId::Source(checked.syntax().enums[0].id)
    );
    assert_eq!(
        variant,
        ResolvedEnumVariantId::Source(checked.syntax().enums[0].variants[0].id)
    );
    assert_eq!(lowered_binding, Some(binding.id));

    let splitscript::compiler::ast::Stmt::Variable(result) =
        &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the result binding");
    };
    let splitscript::compiler::ast::ExprKind::Call { args, .. } = &result.value.kind else {
        panic!("expected the read call");
    };
    assert_eq!(
        checked.semantics().enum_variant(args[0].id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    assert_eq!(
        checked.typed_hir().enum_variant(args[0].id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved pattern receivers should produce valid Wasm");
}

#[test]
fn member_paths_resolve_record_and_standard_fields_to_stable_ids() {
    let source = r#"
        state "game.exe" {}

        record Inner { value: i32 }
        record Outer { inner: Inner }

        fn Inner.increment() -> i32 {
            return self.value + 1
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let outer = Outer { inner: Inner { value: 4 } }
            let nested = outer.inner.value
            let method = outer.inner.increment()
            let address = module.address
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let inner_value = checked.syntax().records[0].fields[0].id;
    let outer_inner = checked.syntax().records[1].fields[0].id;
    assert_ne!(inner_value, outer_inner);

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(outer) = &statements[1] else {
        panic!("expected the outer binding");
    };
    assert_eq!(
        checked.semantics().record_literal_fields(outer.value.id),
        Some([ResolvedRecordFieldId::Source(outer_inner)].as_slice())
    );
    assert_eq!(
        checked.typed_hir().record_literal_fields(outer.value.id),
        Some([ResolvedRecordFieldId::Source(outer_inner)].as_slice())
    );
    let splitscript::compiler::ast::Stmt::Variable(nested) = &statements[2] else {
        panic!("expected the nested field binding");
    };
    assert_eq!(
        checked.semantics().path_members(nested.value.id),
        Some(
            [
                ResolvedMember::RecordField(outer_inner),
                ResolvedMember::RecordField(inner_value),
            ]
            .as_slice()
        )
    );
    let (nested_root, nested_members) = checked
        .typed_hir()
        .value_path(nested.value.id)
        .expect("typed HIR should materialize resolved paths");
    assert_eq!(nested_root, Some(ResolvedValue::Variable(outer.id)));
    assert_eq!(
        nested_members,
        [
            ResolvedMember::RecordField(outer_inner),
            ResolvedMember::RecordField(inner_value),
        ]
    );

    let splitscript::compiler::ast::Stmt::Variable(method) = &statements[3] else {
        panic!("expected the nested receiver binding");
    };
    let Some(ResolvedCall::UserMethod { receiver, .. }) = checked.semantics().call(method.value.id)
    else {
        panic!("expected a resolved nested method receiver");
    };
    assert_eq!(
        receiver,
        &ResolvedReceiver::Path {
            root: ResolvedValue::Variable(outer.id),
            members: vec![ResolvedMember::RecordField(outer_inner)],
        }
    );

    let splitscript::compiler::ast::Stmt::Variable(address) = &statements[4] else {
        panic!("expected the built-in field binding");
    };
    assert_eq!(
        checked.semantics().path_members(address.value.id),
        Some([ResolvedMember::StandardField(StdlibFieldId::ModuleAddress,)].as_slice())
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved member chains should produce valid Wasm");
}

#[test]
fn value_paths_resolve_globals_parameters_and_locals_to_declaration_ids() {
    let source = r#"
        let seed = 7
        state "game.exe" {}

        fn identity(value: i32) -> i32 {
            return value
        }

        whileAttached {
            let copy: i32 = seed
            let result = identity(copy)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let global = checked.syntax().globals[0].id;
    let parameter = checked.syntax().functions[0].params[0].id;
    let splitscript::compiler::ast::Stmt::Return {
        value: Some(parameter_path),
        ..
    } = &checked.syntax().functions[0].body.statements[0]
    else {
        panic!("expected the parameter return");
    };
    assert_eq!(
        checked.semantics().value(parameter_path.id),
        Some(ResolvedValue::Variable(parameter))
    );

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(copy) = &statements[0] else {
        panic!("expected the local copy");
    };
    assert_eq!(
        checked.semantics().value(copy.value.id),
        Some(ResolvedValue::Variable(global))
    );
    let splitscript::compiler::ast::Stmt::Variable(result) = &statements[1] else {
        panic!("expected the result binding");
    };
    let splitscript::compiler::ast::ExprKind::Call { args, .. } = &result.value.kind else {
        panic!("expected the identity call");
    };
    assert_eq!(
        checked.semantics().value(args[0].id),
        Some(ResolvedValue::Variable(copy.id))
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved value reads should produce valid Wasm");
}

#[test]
fn snapshot_paths_resolve_state_and_setting_ids_with_temporal_identity() {
    let source = r#"
        state "game.exe" {
            score: i32 at 0x1000
        }

        settings {
            "Enabled" => enabled: true
        }

        whileAttached {
            let currentScore = current.score
            let oldScore = old.score
            let enabled = settings.enabled
            let wasEnabled = oldSettings.enabled
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let state = checked.syntax().state.as_ref().unwrap().fields[0].id;
    let setting = checked.syntax().settings[0].id;
    let statements = &checked.syntax().actions[0].body.statements;
    let expected = [
        ResolvedValue::CurrentState(state),
        ResolvedValue::OldState(state),
        ResolvedValue::Setting(setting),
        ResolvedValue::OldSetting(setting),
    ];
    for (statement, expected) in statements.iter().zip(expected) {
        let splitscript::compiler::ast::Stmt::Variable(variable) = statement else {
            panic!("expected a snapshot binding");
        };
        assert_eq!(checked.semantics().value(variable.value.id), Some(expected));
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved snapshot reads should produce valid Wasm");
}

#[test]
fn assignments_resolve_local_and_global_targets_by_id() {
    let source = r#"
        let counter = 0
        state "game.exe" {}

        whileAttached {
            let local: i32 = 1
            local = 2
            counter = local
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let global = checked.syntax().globals[0].id;
    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(local) = &statements[0] else {
        panic!("expected the local declaration");
    };
    let splitscript::compiler::ast::Stmt::Assign {
        id: local_write, ..
    } = &statements[1]
    else {
        panic!("expected the local assignment");
    };
    let splitscript::compiler::ast::Stmt::Assign {
        id: global_write, ..
    } = &statements[2]
    else {
        panic!("expected the global assignment");
    };
    assert_eq!(
        checked.semantics().assignment_target(*local_write),
        Some(local.id)
    );
    assert_eq!(
        checked.semantics().assignment_target(*global_write),
        Some(global)
    );
    assert_eq!(
        checked
            .typed_hir()
            .assignment(*local_write)
            .map(|assignment| assignment.target),
        Some(local.id)
    );
    assert_eq!(
        checked
            .typed_hir()
            .assignment(*global_write)
            .map(|assignment| assignment.target),
        Some(global)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved assignments should produce valid Wasm");
}
