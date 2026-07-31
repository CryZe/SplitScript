//! diagnostics migration integration tests.

#[test]
fn repeated_option_and_result_postfixes_have_a_focused_diagnostic() {
    use splitscript::{DiagnosticLabelStyle, FixApplicability};

    for annotation in ["i32??", "i32!!", "i32?!", "i32!?"] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                fn invalid(value: {annotation}) {{}}
            "#
        );
        let errors =
            splitscript::parse(&source).expect_err("repeated postfixes should be rejected");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .message
                .contains("repeated optional/result postfixes"),
            "unexpected diagnostic for {annotation}: {}",
            errors[0].message
        );
        assert_eq!(errors[0].labels.len(), 1);
        assert_eq!(errors[0].labels[0].style, DiagnosticLabelStyle::Primary);
        assert_eq!(
            errors[0].labels[0].message.as_deref(),
            Some("this second wrapper postfix is not allowed")
        );
        assert_eq!(errors[0].notes.len(), 1);
        assert_eq!(errors[0].fixes.len(), 1);
        assert_eq!(
            errors[0].fixes[0].applicability,
            FixApplicability::MachineApplicable
        );
        assert_eq!(errors[0].fixes[0].edits.len(), 1);
        let edit = &errors[0].fixes[0].edits[0];
        assert_eq!(&source[edit.span.start..edit.span.end], &annotation[4..]);
        assert!(edit.replacement.is_empty());
    }
}

#[test]
fn familiar_declaration_keywords_recover_as_let_with_machine_applicable_fixes() {
    use splitscript::{FixApplicability, compiler::ast::Stmt};

    let source = r#"
        state "game.exe" {}
        const baseAddress = 5
        var fallbackAddress = 0

        onAttach {
            const module = await process.module("GameAssembly.dll")
        }

        whileAttached {
            const address = baseAddress
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 4);
    for diagnostic in recovered.diagnostics() {
        let keyword = &source[diagnostic.span.start..diagnostic.span.end];
        assert!(matches!(keyword, "const" | "var"));
        assert_eq!(
            diagnostic.message,
            format!("SplitScript uses `let` instead of `{keyword}` for variable declarations")
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, format!("replace `{keyword}` with `let`"));
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, "let");
    }

    assert_eq!(recovered.syntax().globals.len(), 2);
    assert!(matches!(
        recovered.syntax().actions[0].body.statements[0],
        Stmt::Suspend { .. }
    ));
    assert!(matches!(
        recovered.syntax().actions[1].body.statements[0],
        Stmt::Variable(_)
    ));
    assert!(recovered.recovery_nodes().is_empty());
    splitscript::compile(&source.replace("const", "let").replace("var", "let"))
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn null_recovers_as_none_with_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        fn maybeLevel(selected) -> u32? {
            if selected { return 7 }
            return null
        }

        fn levelName(level: u32?) {
            return match level {
                null => "None",
                Some(value) => value as String
            }
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    for diagnostic in recovered.diagnostics() {
        assert_eq!(
            diagnostic.message,
            "SplitScript uses `None` instead of `null` for absent optional values"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "null");
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, "replace `null` with `None`");
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, "None");
    }

    assert!(recovered.recovery_nodes().is_empty());
    splitscript::compile(&source.replace("null", "None"))
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn familiar_function_and_string_spellings_have_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        func firstLabel() -> string {
            return "First"
        }

        function identity(value: string) -> String {
            return value
        }

        function string.isEmpty() {
            return string.length(self) == 0
        }

        function elapsed() -> TimeSpan {
            return TimeSpan.fromSeconds(1.0)
        }

        debug function trace(message: string) {
            print(message)
        }

        whileAttached {
            print(identity(firstLabel()))
            debug trace("updated")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 12);
    for diagnostic in recovered.diagnostics() {
        let spelling = &source[diagnostic.span.start..diagnostic.span.end];
        let (replacement, message, title) = match spelling {
            "func" | "function" => (
                "fn",
                format!("SplitScript uses `fn` instead of `{spelling}` for functions"),
                format!("replace `{spelling}` with `fn`"),
            ),
            "string" => (
                "String",
                "SplitScript uses `String` instead of `string` for the string type".to_owned(),
                "replace `string` with `String`".to_owned(),
            ),
            "TimeSpan" => (
                "Duration",
                "SplitScript uses `Duration` instead of `TimeSpan` for timer durations".to_owned(),
                "replace `TimeSpan` with `Duration`".to_owned(),
            ),
            _ => panic!("unexpected familiar spelling `{spelling}`"),
        };
        assert_eq!(diagnostic.message, message);
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, title);
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, replacement);
    }

    assert_eq!(recovered.syntax().functions.len(), 5);
    assert!(recovered.syntax().functions[4].debug_only);
    assert!(recovered.recovery_nodes().is_empty());
    let fixed = source
        .replace("function", "fn")
        .replace("func", "fn")
        .replace("string", "String")
        .replace("TimeSpan", "Duration");
    splitscript::compile(&fixed)
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn csharp_numeric_type_names_have_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        record CSharpNumbers {
            signed8: sbyte
            unsigned8: byte
            signed16: short
            unsigned16: ushort
            signed32: int
            unsigned32: uint
            signed64: long
            unsigned64: ulong
            single: float
            doublePrecision: double
        }

        fn int.identity() -> int {
            return self
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    let expected = [
        ("sbyte", "i8"),
        ("byte", "u8"),
        ("short", "i16"),
        ("ushort", "u16"),
        ("int", "i32"),
        ("uint", "u32"),
        ("long", "i64"),
        ("ulong", "u64"),
        ("float", "f32"),
        ("double", "f64"),
        ("int", "i32"),
        ("int", "i32"),
    ];

    assert_eq!(recovered.diagnostics().len(), expected.len());
    for (diagnostic, (csharp_name, splitscript_name)) in
        recovered.diagnostics().iter().zip(expected)
    {
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            csharp_name
        );
        assert_eq!(
            diagnostic.message,
            format!(
                "SplitScript uses `{splitscript_name}` instead of `{csharp_name}` for this numeric type"
            )
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(
            fix.title,
            format!("replace `{csharp_name}` with `{splitscript_name}`")
        );
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, splitscript_name);
    }

    assert!(recovered.recovery_nodes().is_empty());
    let mut fixed = source.to_owned();
    for diagnostic in recovered.diagnostics().iter().rev() {
        let edit = &diagnostic.fixes[0].edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    splitscript::compile(&fixed)
        .expect("applying every suggested numeric type replacement should produce valid source");
}

#[test]
fn unknown_calls_suggest_canonical_names_across_naming_styles() {
    use splitscript::FixApplicability;

    let cases = [
        (
            "Duration.FromSeconds(1.0)",
            "FromSeconds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        (
            "Duration.from_seconds(1.0)",
            "from_seconds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        (
            "Duration.fromSecnds(1.0)",
            "fromSecnds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        ("value.ClAmP(0, 10)", "ClAmP", "clamp", "clamp"),
        (
            "value.increment_by(1)",
            "increment_by",
            "incrementBy",
            "incrementBy",
        ),
        ("add_one(value)", "add_one", "addOne", "addOne"),
    ];

    for (call, misspelled, replacement, suggested_display) in cases {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn addOne(value: u32) -> u32 {{
                    return value + 1
                }}

                fn u32.incrementBy(amount: u32) -> u32 {{
                    return self + amount
                }}

                whileAttached {{
                    let value: u32 = 5
                    {call}
                }}
            "#
        );
        let errors = splitscript::compile(&source).expect_err("the misspelled call must fail");
        assert_eq!(errors.len(), 1, "unexpected diagnostics for `{call}`");
        let diagnostic = &errors[0];
        assert!(
            diagnostic
                .message
                .contains(&format!("did you mean `{suggested_display}`?")),
            "{}",
            diagnostic.message
        );
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            misspelled
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, replacement);

        let mut fixed = source;
        let edit = &fix.edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        splitscript::compile(&fixed)
            .expect("applying the suggested call-name replacement should compile");
    }
}

#[test]
fn unrelated_unknown_methods_do_not_receive_noisy_suggestions() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let value: u32 = 5
            value.completelyUnrelated()
        }
    "#;
    let errors = splitscript::compile(source).expect_err("the unknown method must fail");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "type `u32` has no method `completelyUnrelated`"
    );
    assert!(errors[0].fixes.is_empty());
}
