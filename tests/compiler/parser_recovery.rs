//! parser recovery integration tests.

#[test]
fn recovering_parse_reports_multiple_errors_and_keeps_later_declarations() {
    use splitscript::compiler::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        record Broken { value }
        fn retained() {
            let missingAssignment
            return 1
        }
        nonsense
        whileAttached { print("retained action") }
        reset { let = 1 }
        split { return false }
    "#;
    let recovered = splitscript::parse_recovering(source)
        .expect("the lexer should still produce a recoverable document");

    assert_eq!(recovered.source_document().reconstruct(), source);
    assert_eq!(recovered.diagnostics().len(), 4);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "expected `:` after the field name")
    );
    assert!(recovered.diagnostics().iter().any(|diagnostic| {
        diagnostic
            .message
            .starts_with("expected `state`, `tickRate`, `settings`")
    }));
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "expected a variable name")
    );
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "expected `=` in variable declaration" })
    );
    assert_eq!(recovered.syntax().functions.len(), 1);
    assert_eq!(recovered.syntax().actions.len(), 3);
    assert!(recovered.syntax().actions[1].body.statements.is_empty());
    assert!(recovered.recovery_nodes().iter().any(|node| {
        node.kind == RecoveryNodeKind::Missing && node.span.start == node.span.end
    }));
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        4
    );

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), recovered.diagnostics().len());
}

#[test]
fn missing_state_uses_canonical_attachment_syntax() {
    let recovered = splitscript::parse_recovering("fn helper() { return 1 }").unwrap();
    let diagnostic = recovered
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.message.contains("attachment `state`"))
        .expect("missing state has a focused diagnostic");

    assert_eq!(
        diagnostic.message,
        "a SplitScript autosplitter needs one attachment `state` declaration"
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("state \"game.exe\" { ... }") && note.contains("state GBA { ... }")
    }));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("helper module"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .all(|note| !note.contains("state(process"))
    );
    assert_eq!(
        diagnostic.migration_topic.as_deref(),
        Some("asl.state.attachment")
    );
}

#[test]
fn legacy_lifecycle_blocks_get_semantic_migration_guidance() {
    let source = r#"
        state "game.exe" {}
        startup {}
        init {}
        update {}
        exit {}
        shutdown {}
        onStart {}
        onSplit {}
        onReset {}
        whileAttached { print("retained") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 8);
    for expected in [
        "ASL `startup` is not a SplitScript lifecycle block",
        "ASL `init` has no blind one-to-one lifecycle rename",
        "ASL `update` is named `whileAttached` for ordinary per-tick work",
        "ASL `exit` is named `onDetach`",
        "ASL `shutdown` has no SplitScript equivalent yet",
    ] {
        assert!(
            recovered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message == expected),
            "missing diagnostic `{expected}`: {:#?}",
            recovered.diagnostics()
        );
    }
    assert_eq!(
        recovered
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.message == "ASL timer event handlers are not SplitScript decision blocks"
            })
            .count(),
        3
    );
    assert!(
        recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.fixes.is_empty()),
        "lifecycle migration cannot be a blind machine-applicable rename"
    );
    let topics = recovered
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.migration_topic.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert!(topics.contains(&"asl.lifecycle.startup"));
    assert!(topics.contains(&"asl.lifecycle.init"));
    assert!(topics.contains(&"asl.lifecycle.update"));
    assert!(topics.contains(&"asl.lifecycle.exit"));
    assert!(topics.contains(&"asl.lifecycle.shutdown"));
    assert_eq!(
        topics
            .iter()
            .filter(|topic| **topic == "asl.timer.events")
            .count(),
        3
    );
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(recovered.syntax().actions[0].kind.name(), "whileAttached");
}

#[test]
fn recovering_parse_keeps_later_statements_in_the_same_block() {
    use splitscript::{compiler::ast::Stmt, compiler::syntax::RecoveryNodeKind};

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let before = 1
            let = 2
            print("after outer error")
            if true {
                let = 3
                print("after nested error")
            }
            print("last")
        }
        split { return false }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.message == "expected a variable name")
    );
    assert_eq!(recovered.syntax().actions.len(), 2);
    let body = &recovered.syntax().actions[0].body;
    assert_eq!(body.statements.len(), 4);
    let Stmt::If { then_block, .. } = &body.statements[2] else {
        panic!("the recovered outer block should retain its if statement");
    };
    assert_eq!(then_block.statements.len(), 1);
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn standalone_value_blocks_parse_without_recovery() {
    use splitscript::compiler::ast::{ExprKind, Stmt};

    // A block in statement position is an ordinary expression statement. Its
    // internal return still targets the enclosing action.
    let source = r#"
        state "game.exe" {}
        split {
            {
                print("legacy block")
                return true
            }
        }
        reset { return false }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert!(recovered.diagnostics().is_empty());
    assert_eq!(recovered.syntax().actions.len(), 2);
    assert!(matches!(
        recovered.syntax().actions[0].body.statements.as_slice(),
        [Stmt::Expression(expression)] if matches!(expression.kind, ExprKind::Block(_))
    ));
    assert_eq!(recovered.syntax().actions[1].kind.name(), "reset");
    assert_eq!(recovered.source_document().reconstruct(), source);
}

#[test]
fn recovering_parse_keeps_later_record_fields_and_enum_variants() {
    use splitscript::compiler::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        record RecoveredRecord {
            first: i32,
            missingColon i64,
            after: u32,
        }
        enum RecoveredEnum {
            First,
            Broken(i32,
            After,
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "expected `:` after the field name" })
    );
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "expected `)` after the payload type" })
    );
    assert_eq!(
        recovered.syntax().records[0]
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "after"]
    );
    assert_eq!(
        recovered.syntax().enums[0]
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "After"]
    );
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn recovering_parse_keeps_later_state_fields_in_both_syntaxes() {
    use splitscript::compiler::syntax::RecoveryNodeKind;

    let cases = [
        (
            r#"
                state "game.exe" {
                    first: i32 at 0x10;
                    broken: i32 nope,
                    after: u32 at 0x20;
                }
                whileAttached { print("still parsed") }
            "#,
            "expected `at`",
        ),
        (
            r#"
                state("game.exe", {
                    first: memory.i32(0x10),
                    broken: memory.i32("bad"),
                    after: memory.u32(0x20)
                })
                whileAttached { print("still parsed") }
            "#,
            "expected an address offset",
        ),
    ];

    for (source, expected_error) in cases {
        let recovered = splitscript::parse_recovering(source).unwrap();
        assert_eq!(recovered.diagnostics().len(), 1);
        assert_eq!(recovered.diagnostics()[0].message, expected_error);
        assert_eq!(
            recovered
                .syntax()
                .state
                .as_ref()
                .unwrap()
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "after"]
        );
        assert_eq!(recovered.syntax().actions.len(), 1);
        assert_eq!(
            recovered
                .recovery_nodes()
                .iter()
                .filter(|node| node.kind == RecoveryNodeKind::Error)
                .count(),
            1
        );
        assert_eq!(recovered.source_document().reconstruct(), source);

        let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
        assert_eq!(strict_errors.len(), 1);
    }
}

#[test]
fn recovering_parse_keeps_neighboring_settings_in_the_settings_dsl() {
    use splitscript::compiler::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        settings {
            "Group" {
                "First" => first: true,
                "Broken" -> broken: true,
                /// Retained tooltip.
                "After" => after: false,
            },
            "Outside" => outside: true,
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 1);
    assert_eq!(
        recovered.diagnostics()[0].message,
        "expected `=>` after the setting label"
    );
    assert_eq!(
        recovered
            .syntax()
            .settings
            .iter()
            .map(|setting| setting.name.as_str())
            .collect::<Vec<_>>(),
        ["_heading0", "first", "after", "outside"]
    );
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered
            .syntax()
            .settings
            .iter()
            .find(|setting| setting.name == "after")
            .and_then(|setting| setting.tooltip.as_deref()),
        Some("Retained tooltip.")
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 1);
}

#[test]
fn legacy_settings_syntax_is_rejected_with_a_dsl_rewrite() {
    let compact = r#"
        state "game.exe" {}
        settings {
            area_5_to_2: bool = true, "Wayfarer Offices"
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(compact).unwrap();
    assert!(recovered.syntax().settings.is_empty());
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "legacy `name: bool = default, \"label\"` settings syntax is not supported"
    );
    let fix = diagnostic
        .fixes
        .first()
        .expect("legacy syntax should have a fix");
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(
        fix.edits[0].replacement,
        "\"Wayfarer Offices\" => area_5_to_2: true"
    );

    let parenthesized = r#"
        state "game.exe" {}
        settings({ enabled: Setting.bool(true) })
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(parenthesized).unwrap();
    assert!(recovered.syntax().settings.is_empty());
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(recovered.diagnostics().len(), 1);
    assert_eq!(
        recovered.diagnostics()[0].message,
        "legacy `settings({ ... })` syntax is not supported; use `settings { ... }`"
    );
}

#[test]
fn recovering_parse_keeps_valid_choice_options_and_file_filters() {
    use splitscript::{
        compiler::ast::{SettingFileFilter, SettingKind},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        enum Mode {
            A,
            B,
            C,
        }
        settings {
            "Mode" => mode: choice {
                "A" => Mode.A default,
                "Broken" -> Mode.B,
                "C" => Mode.C,
            },
            "File" => file: file {
                "Save" => "*.sav",
                "Broken" -> "*.bad",
                mime => "application/octet-stream",
            },
            "After" => after: true,
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.message.contains("expected `=>`"))
    );
    assert_eq!(
        recovered
            .syntax()
            .settings
            .iter()
            .map(|setting| setting.name.as_str())
            .collect::<Vec<_>>(),
        ["mode", "file", "after"]
    );
    let SettingKind::Choice {
        default_variant,
        options,
        ..
    } = &recovered.syntax().settings[0].kind
    else {
        panic!("mode should remain a choice setting");
    };
    assert_eq!(default_variant, "A");
    assert_eq!(
        options
            .iter()
            .map(|option| option.variant.as_str())
            .collect::<Vec<_>>(),
        ["A", "C"]
    );
    let SettingKind::File { filters, .. } = &recovered.syntax().settings[1].kind else {
        panic!("file should remain a file setting");
    };
    assert_eq!(filters.len(), 2);
    assert!(matches!(
        &filters[0],
        SettingFileFilter::Name {
            description: Some(description),
            pattern,
        } if description == "Save" && pattern == "*.sav"
    ));
    assert!(matches!(
        &filters[1],
        SettingFileFilter::Mime { value: mime, .. } if mime == "application/octet-stream"
    ));
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn recovering_parse_keeps_valid_match_arms_and_enclosing_function() {
    use splitscript::{
        compiler::ast::{Expr, ExprKind, MatchPattern, Stmt},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        enum Mode {
            A,
            B,
            C,
        }
        fn label(mode: Mode) {
            return match mode {
                Mode.A => "A",
                Mode.B -> "Broken",
                Mode.C => "C",
            }
        }
        whileAttached { print(label(Mode.A)) }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 1);
    assert_eq!(
        recovered.diagnostics()[0].message,
        "expected `=>` after the pattern"
    );
    assert_eq!(recovered.syntax().functions.len(), 1);
    let Stmt::Expression(Expr {
        kind: ExprKind::Return(Some(value)),
        ..
    }) = &recovered.syntax().functions[0].body.statements[0]
    else {
        panic!("the recovered function should retain its return expression");
    };
    let ExprKind::Match { arms, .. } = &value.kind else {
        panic!("the recovered return value should remain a match");
    };
    assert_eq!(arms.len(), 2);
    assert_eq!(
        arms.iter()
            .map(|arm| match &arm.pattern {
                MatchPattern::Enum { variant, .. } => variant.as_str(),
                _ => panic!("expected enum patterns"),
            })
            .collect::<Vec<_>>(),
        ["A", "C"]
    );
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 1);
}

#[test]
fn recovering_parse_keeps_valid_parameters_and_function_bodies() {
    use splitscript::compiler::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        fn recovered(first: i32, broken: , after: u32) {
            return first + after
        }
        fn missingClose(value: i32 {
            return value
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "expected a parameter type" })
    );
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "expected `)` after the parameters" })
    );
    assert_eq!(recovered.syntax().functions.len(), 2);
    assert_eq!(
        recovered.syntax().functions[0]
            .params
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "after"]
    );
    assert_eq!(
        recovered.syntax().functions[1]
            .params
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["value"]
    );
    assert_eq!(recovered.syntax().functions[0].body.statements.len(), 1);
    assert_eq!(recovered.syntax().functions[1].body.statements.len(), 1);
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn recovering_parse_keeps_valid_array_elements_and_call_arguments() {
    use splitscript::{
        compiler::ast::{ExprKind, Stmt},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        fn combine(first, second) { return first + second }
        whileAttached {
            let values = [1, , 3]
            combine(10, , 30)
            print("after")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.message == "expected an expression")
    );
    let statements = &recovered.syntax().actions[0].body.statements;
    assert_eq!(statements.len(), 3);

    let Stmt::Variable(values) = &statements[0] else {
        panic!("the recovered first statement should remain a variable declaration");
    };
    let ExprKind::Array(elements) = &values.value.as_ref().unwrap().kind else {
        panic!("the recovered initializer should remain an array");
    };
    assert_eq!(
        elements
            .iter()
            .map(|element| match element.kind {
                ExprKind::Int { value, .. } => value,
                _ => panic!("expected integer array elements"),
            })
            .collect::<Vec<_>>(),
        [1, 3]
    );

    let Stmt::Expression(call) = &statements[1] else {
        panic!("the recovered second statement should remain a call");
    };
    let ExprKind::Call { args, .. } = &call.kind else {
        panic!("the recovered expression should remain a call");
    };
    assert_eq!(
        args.iter()
            .map(|argument| match argument.kind {
                ExprKind::Int { value, .. } => value,
                _ => panic!("expected integer call arguments"),
            })
            .collect::<Vec<_>>(),
        [10, 30]
    );

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        2
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn recovering_parse_keeps_valid_record_fields_and_template_interpolations() {
    use splitscript::{
        compiler::ast::{ExprKind, InterpolatedPart, Stmt},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        record Point {
            x: i32,
            y: i32,
        }
        state "game.exe" {}
        whileAttached {
            let point = Point { x: 1, broken: , y: 2 }
            print(`point={point.x}, broken={1 + }, after={point.y}`)
            print("still parsed")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    assert!(
        recovered
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.message == "expected an expression")
    );
    let statements = &recovered.syntax().actions[0].body.statements;
    assert_eq!(statements.len(), 3);

    let Stmt::Variable(point) = &statements[0] else {
        panic!("the recovered record literal should remain a variable initializer");
    };
    let ExprKind::Record { fields, .. } = &point.value.as_ref().unwrap().kind else {
        panic!("the recovered initializer should remain a record literal");
    };
    assert_eq!(
        fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["x", "y"]
    );

    let Stmt::Expression(print) = &statements[1] else {
        panic!("the recovered template should remain inside its call");
    };
    let ExprKind::Call { args, .. } = &print.kind else {
        panic!("expected the enclosing print call");
    };
    let ExprKind::InterpolatedString(parts) = &args[0].kind else {
        panic!("the recovered argument should remain an interpolated string");
    };
    let interpolations = parts
        .iter()
        .filter_map(|part| match part {
            InterpolatedPart::Expr(expression) => Some(expression),
            InterpolatedPart::Text(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(interpolations.len(), 3);
    assert!(matches!(
        &interpolations[0].kind,
        ExprKind::Path(path) if path == &["point", "x"]
    ));
    assert!(matches!(
        interpolations[1].kind,
        ExprKind::Binary { ref right, .. } if matches!(right.kind, ExprKind::Error)
    ));
    assert!(matches!(
        &interpolations[2].kind,
        ExprKind::Path(path) if path == &["point", "y"]
    ));

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        2
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 2);
}

#[test]
fn recovering_parse_preserves_missing_operands_and_parenthesized_expressions() {
    use splitscript::{
        compiler::ast::{ExprKind, Stmt},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let missingRight = 1 +
            let afterBinary = 2
            let missingUnary = !
            let afterUnary = 3
            let emptyGroup = ()
            let noisyGroup = (4 unexpected)
            print("still parsed")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 4);
    assert_eq!(recovered.syntax().actions[0].body.statements.len(), 7);
    let statements = &recovered.syntax().actions[0].body.statements;

    let Stmt::Variable(missing_right) = &statements[0] else {
        panic!("expected the binary initializer to be retained");
    };
    assert!(matches!(
        missing_right.value.as_ref().unwrap().kind,
        ExprKind::Binary { ref right, .. } if matches!(right.kind, ExprKind::Error)
    ));

    let Stmt::Variable(missing_unary) = &statements[2] else {
        panic!("expected the unary initializer to be retained");
    };
    assert!(matches!(
        missing_unary.value.as_ref().unwrap().kind,
        ExprKind::Unary { ref expr, .. } if matches!(expr.kind, ExprKind::Error)
    ));

    let Stmt::Variable(empty_group) = &statements[4] else {
        panic!("expected the empty parenthesized initializer to be retained");
    };
    assert!(matches!(
        empty_group.value.as_ref().unwrap().kind,
        ExprKind::Error
    ));

    let Stmt::Variable(noisy_group) = &statements[5] else {
        panic!("expected the parenthesized initializer with trailing syntax to be retained");
    };
    assert!(matches!(
        noisy_group.value.as_ref().unwrap().kind,
        ExprKind::Int { value: 4, .. }
    ));

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        4
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 4);
}

#[test]
fn recovering_parse_preserves_malformed_if_expressions() {
    use splitscript::{
        compiler::ast::{Expr, ExprKind, Stmt},
        compiler::syntax::RecoveryNodeKind,
    };

    fn conditional(statement: &Stmt) -> (&Expr, &Expr, &Expr) {
        let Stmt::Variable(variable) = statement else {
            panic!("expected a retained variable declaration");
        };
        let ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } = &variable.value.as_ref().unwrap().kind
        else {
            panic!("expected a retained if expression");
        };
        (condition, then_expr, else_expr)
    }

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let missingCondition = if { 1 } else { 2 }
            let emptyThen = if true {} else { 3 }
            let emptyElse = if false { 4 } else {}
            let missingElse = if true { 5 }
            let noisyThen = if true { 6 unexpected } else { 7 }
            print("still parsed")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 4);
    let statements = &recovered.syntax().actions[0].body.statements;
    assert_eq!(statements.len(), 6);

    assert!(matches!(
        conditional(&statements[0]).0.kind,
        ExprKind::Fallback { .. }
    ));
    assert!(matches!(
        conditional(&statements[0]).1.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[1]).1.kind,
        ExprKind::Block(_)
    ));
    assert!(matches!(
        conditional(&statements[2]).2.kind,
        ExprKind::Block(_)
    ));
    assert!(matches!(
        conditional(&statements[3]).2.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[4]).1.kind,
        ExprKind::Block(_)
    ));

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        4
    );
    assert!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .all(|node| node.span.start == node.span.end)
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        1
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 4);
}

#[test]
fn recovering_parse_preserves_declarations_and_statements_with_bad_root_expressions() {
    use splitscript::{
        compiler::ast::{Expr, ExprKind, StateSource, Stmt, SuspensionMode},
        compiler::syntax::RecoveryNodeKind,
    };

    let source = r#"
        let brokenGlobal = +
        let goodGlobal = 1
        state "game.exe" {
            brokenState = +;
            goodState = 2;
        }
        fn recovered() {
            let brokenLocal = +
            target = +
            throw
            await +
            retry +
            +
            while + { print("loop body") }
            let missingMatch = match { _ => 1 }
            return 1
        }
        whileAttached { print("still parsed") }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(
        recovered.diagnostics().len(),
        10,
        "{:#?}",
        recovered.diagnostics()
    );
    assert_eq!(recovered.syntax().globals.len(), 2);
    assert!(matches!(
        recovered.syntax().globals[0].value.as_ref().unwrap().kind,
        ExprKind::Error
    ));
    assert!(matches!(
        recovered.syntax().globals[1].value.as_ref().unwrap().kind,
        ExprKind::Int { value: 1, .. }
    ));

    let state = recovered.syntax().state.as_ref().unwrap();
    assert_eq!(state.fields.len(), 2);
    assert!(matches!(
        state.fields[0].source,
        StateSource::Expression(ref expression) if matches!(expression.kind, ExprKind::Error)
    ));

    let statements = &recovered.syntax().functions[0].body.statements;
    assert_eq!(statements.len(), 8);
    assert!(matches!(
        statements[0],
        Stmt::Variable(ref variable) if matches!(variable.value.as_ref().unwrap().kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[1],
        Stmt::Assign { ref value, .. } if matches!(value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[2],
        Stmt::Expression(Expr {
            kind: ExprKind::Throw(ref error),
            ..
        }) if matches!(error.kind, ExprKind::Suspend {
            mode: SuspensionMode::Await,
            ref value,
            ..
        } if matches!(value.kind, ExprKind::Error))
    ));
    assert!(matches!(
        statements[3],
        Stmt::Expression(Expr { kind: ExprKind::Suspend {
            mode: SuspensionMode::Retry, ref value, ..
        }, .. }) if matches!(value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[4],
        Stmt::Expression(ref expression) if matches!(expression.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[5],
        Stmt::While { ref condition, .. } if matches!(condition.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[6],
        Stmt::Variable(ref variable) if matches!(variable.value.as_ref().unwrap().kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[7],
        Stmt::Expression(Expr {
            kind: ExprKind::Return(_),
            ..
        })
    ));
    assert_eq!(recovered.syntax().actions.len(), 1);

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        10
    );
    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Error)
            .count(),
        10
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 10);
}

#[test]
fn malformed_for_header_recovers_without_losing_the_following_statement() {
    use splitscript::compiler::ast::Stmt;

    let source = r#"state "game.exe" {}
whileAttached {
    for value [1, 2] {
        print(value as String)
    }
    print("after")
}"#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("expected `in`"))
    );
    assert!(
        recovered.syntax().actions[0]
            .body
            .statements
            .iter()
            .any(|statement| matches!(statement, Stmt::Expression(_)))
    );
    assert_eq!(recovered.source_document().reconstruct(), source);
}
