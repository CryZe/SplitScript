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
fn failed_initializers_keep_poisoned_bindings_without_follow_on_errors() {
    use splitscript::{
        compiler::ast::Stmt,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"
        state "game.exe" {}

        let globalBroken = missingGlobal()

        whileAttached {
            let localBroken = [1, 2].missingMember(1)
            let member = localBroken.value
            let indexed = localBroken[0]
            let optional: i32? = localBroken
            let fallible: i32! = localBroken
            localBroken.missingAgain()
            if localBroken == 0 {
                print(localBroken)
                print(globalBroken)
                print(member)
                print(indexed)
                print(optional)
                print(fallible)
            }
        }

        onAttach {
            let attachedBroken = await process.missingAwait()
            print(attachedBroken)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("the three unresolved calls remain primary compilation errors");
    let messages = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        messages.len(),
        3,
        "unexpected diagnostic cascade: {messages:#?}"
    );
    assert!(messages.contains(&"unknown function `missingGlobal`"));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("has no method `missingMember`"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("has no method `missingAwait`"))
    );
    assert!(!messages.iter().any(|message| {
        message.contains("unknown variable")
            || message.contains("<unknown>")
            || message.contains("does not satisfy")
            || message.contains("cannot be indexed")
    }));

    let mut database = CompilerDatabase::new(source);
    database.check().expect_err("the source remains invalid");
    let recovered = database.recovering_check().unwrap();
    let Stmt::Variable(local) = &recovered.syntax().actions[0].body.statements[0] else {
        panic!("expected the failed local declaration");
    };
    let use_site = source.find("localBroken.value").unwrap();
    assert!(matches!(
        database.definition_at(use_site).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(local.id)
    ));
    assert!(database.rename_target_at(use_site).unwrap().is_some());
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
        Stmt::Variable(ref variable)
            if matches!(variable.value.kind, splitscript::compiler::ast::ExprKind::Suspend { .. })
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
            return self.byteLength() == 0
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

    assert_eq!(recovered.diagnostics().len(), 11);
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
            signed8: sbyte,
            unsigned8: byte,
            signed16: short,
            unsigned16: ushort,
            signed32: int,
            unsigned32: uint,
            signed64: long,
            unsigned64: ulong,
            single: float,
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
            "\"MAP\".ToLower()",
            "ToLower",
            "toAsciiLowerCase",
            "toAsciiLowerCase",
        ),
        ("\"a_b\".Split(\"_\")", "Split", "split", "split"),
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

#[test]
fn asl_string_n_fields_offer_an_encoding_aware_rewrite() {
    use splitscript::{FixApplicability, compiler::ast::StateMemoryDecoder};

    let source = r#"
        state "game.exe" {
            string50 map : "game.exe", 0x100, 0x20;
            after: u32 at 0x200
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "ASL `stringN` fields need an explicit SplitScript memory decoder"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "string50"
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("auto-detects UTF-16"))
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    let fix = &diagnostic.fixes[0];
    assert_eq!(fix.applicability, FixApplicability::MaybeIncorrect);
    assert_eq!(fix.edits.len(), 3);

    let state = recovered.syntax().state.as_ref().unwrap();
    assert_eq!(
        state
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["map", "after"]
    );
    assert!(matches!(
        &state.fields[0].source,
        splitscript::compiler::ast::StateSource::Pointer(path)
            if matches!(path.decoder, Some(StateMemoryDecoder::Utf8 { max_bytes: 50, .. }))
    ));

    let mut fixed = source.to_owned();
    for edit in fix.edits.iter().rev() {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(fixed.contains("map at \"game.exe\", 0x100, 0x20 as utf8(50)"));
    splitscript::compile(&fixed).expect("the suggested explicit decoder syntax should compile");
}

#[test]
fn string_n_like_field_names_are_not_treated_as_asl_types() {
    let source = r#"
        state "game.exe" {
            string50: u32 at 0x100
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert!(recovered.diagnostics().is_empty());
    assert_eq!(
        recovered.syntax().state.as_ref().unwrap().fields[0].name,
        "string50"
    );
}

#[test]
fn duplicate_state_blocks_explain_named_version_layouts_without_cascades() {
    let source = r#"
        state "game.exe" {
            first: u32 at 0x100
        }
        state "game.exe" {
            second: u32 at 0x200
        }
        whileAttached { print(current.first) }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "SplitScript uses one `state` declaration with named layouts for game versions"
    );
    assert_eq!(diagnostic.labels.len(), 2);
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("StateLayout"))
    );
    assert!(diagnostic.fixes.is_empty());
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered.syntax().state.as_ref().unwrap().fields[0].name,
        "first"
    );
}
