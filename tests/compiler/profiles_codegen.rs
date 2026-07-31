//! profiles codegen integration tests.

use super::catalogs_types::TypedExpressionCounter;
use super::*;

#[test]
fn compiler_stages_expose_lowered_declarations_without_mutating_syntax() {
    let source = r#"
        state "game.exe" {
            level: u16 at 0x1234
        }

        fn identity(value: u16) -> u16 {
            return value
        }

        whileAttached {
            let inferred = [identity(current.level), 2]
            print(`{inferred.get(0)}`)
        }
    "#;

    let parsed = splitscript::parse(source).unwrap();
    assert!(parsed.syntax().array_types.is_empty());

    let lowered = splitscript::lower(parsed);
    let identity = lowered
        .hir()
        .declarations_named("identity")
        .next()
        .expect("lowering should index functions before type checking");
    assert!(matches!(
        identity.id,
        splitscript::compiler::hir::DeclarationId::Function(_)
    ));
    let identity_id = identity.id;
    assert!(
        lowered
            .hir()
            .declarations_named("whileAttached")
            .any(|declaration| {
                declaration.id
                    == splitscript::compiler::hir::DeclarationId::Action(
                        splitscript::compiler::ast::ActionKind::WhileAttached,
                    )
            })
    );

    let checked = splitscript::check(lowered).unwrap();
    assert!(
        checked.syntax().array_types.is_empty(),
        "type checking must not append inferred layouts to parsed syntax"
    );
    assert!(
        checked
            .semantics()
            .array_element_types()
            .any(|(_, element)| checked.semantics().types().kind(element)
                == &TypeKind::Builtin(BuiltinType::U16))
    );
    assert_eq!(
        checked
            .hir()
            .declarations_named("identity")
            .next()
            .map(|declaration| declaration.id),
        Some(identity_id)
    );
    assert_eq!(
        checked.typed_hir().expressions().count(),
        checked.semantics().expression_types().count()
    );
    assert!(checked.typed_hir().expressions().any(|expression| matches!(
        &expression.resolution,
        Some(splitscript::compiler::hir::ExpressionResolution::Call(_))
    )));
    let action_body = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .expect("typed HIR should own action statement shape");
    let splitscript::compiler::hir::TypedStatementKind::Variable { initializer, .. } =
        &action_body.statements[0].kind
    else {
        panic!("expected the inferred variable in typed HIR");
    };
    assert!(matches!(
        &checked.typed_hir().expression(*initializer).unwrap().kind,
        splitscript::compiler::hir::TypedExpressionKind::Array(_)
    ));
    let interpolation = checked
        .typed_hir()
        .expressions()
        .find_map(|expression| match &expression.kind {
            splitscript::compiler::hir::TypedExpressionKind::InterpolatedString(parts) => {
                Some(parts)
            }
            _ => None,
        })
        .expect("typed HIR should retain the interpolated string");
    assert!(matches!(
        interpolation.as_slice(),
        [splitscript::compiler::hir::TypedInterpolatedPart::Expression {
            conversion: Some(splitscript::compiler::hir::ImplicitConversion::ToString { source }),
            ..
        }] if checked.semantics().types().kind(*source)
            == &TypeKind::Builtin(BuiltinType::U16)
    ));
    let mut counter = TypedExpressionCounter::default();
    splitscript::compiler::hir::TypedVisitor::visit_program(&mut counter, checked.typed_hir());
    assert_eq!(counter.0, checked.typed_hir().expressions().count());

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("checked inferred layouts should remain available to code generation");
}

#[test]
fn compiler_profiles_flow_through_staged_and_one_shot_compilation() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = r#"state "game.exe" {} whileAttached { print("profile") }"#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let mut outputs = Vec::new();
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        let options = CompilerOptions { profile };
        let lowered = splitscript::lower_wasm_with_options(&checked, options);
        assert_eq!(lowered.profile(), profile);
        let staged = splitscript::codegen_with_options(&checked, options);
        let one_shot = splitscript::compile_with_options(source, options).unwrap();
        assert_eq!(staged, one_shot);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&staged)
            .expect("both compiler profiles should produce valid WebAssembly GC");
        outputs.push(staged);
    }

    assert_eq!(
        outputs[0], outputs[1],
        "profiles intentionally remain identical until debug constructs exist"
    );
}

#[test]
fn debug_statements_are_checked_but_erased_from_release_lowering() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = include_str!("../debug_profile.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("supported debug statements should typecheck");
    assert!(
        checked
            .typed_hir()
            .action_bodies()
            .flat_map(|body| &body.body.statements)
            .filter(|statement| statement.debug_only)
            .count()
            >= 5
    );

    let debug_functions = checked
        .syntax()
        .functions
        .iter()
        .filter(|function| function.debug_only)
        .collect::<Vec<_>>();
    assert_eq!(debug_functions.len(), 2);
    let debug_globals = checked
        .syntax()
        .globals
        .iter()
        .filter(|global| global.debug_only)
        .collect::<Vec<_>>();
    assert_eq!(debug_globals.len(), 1);
    let debug_lowering = splitscript::lower_wasm_with_options(
        &checked,
        CompilerOptions {
            profile: BuildProfile::Debug,
        },
    );
    let release_lowering = splitscript::lower_wasm_with_options(
        &checked,
        CompilerOptions {
            profile: BuildProfile::Release,
        },
    );
    for function in debug_functions {
        assert!(
            debug_lowering
                .body(splitscript::compiler::wasm_ir::BodyOwner::Function(
                    function.id
                ))
                .is_some()
        );
        assert!(
            release_lowering
                .body(splitscript::compiler::wasm_ir::BodyOwner::Function(
                    function.id
                ))
                .is_none()
        );
    }
    assert!(debug_lowering.contains_global(debug_globals[0].id));
    assert!(!release_lowering.contains_global(debug_globals[0].id));

    let debug = splitscript::compile_with_options(
        source,
        CompilerOptions {
            profile: BuildProfile::Debug,
        },
    )
    .unwrap();
    let release = splitscript::compile_with_options(
        source,
        CompilerOptions {
            profile: BuildProfile::Release,
        },
    )
    .unwrap();
    for wasm in [&debug, &release] {
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(wasm)
            .expect("profile-erased programs should remain valid WebAssembly GC");
    }
    for debug_only in [
        b"debug conditional".as_slice(),
        b"debug statement".as_slice(),
        b"debug loop".as_slice(),
        b"debug function".as_slice(),
        b"debug method".as_slice(),
        b"debug binding".as_slice(),
        b"debug local".as_slice(),
        b"runtime_print_message".as_slice(),
    ] {
        assert!(
            debug
                .windows(debug_only.len())
                .any(|bytes| bytes == debug_only)
        );
        assert!(
            !release
                .windows(debug_only.len())
                .any(|bytes| bytes == debug_only)
        );
    }
    let count_globals = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(|payload| match payload.unwrap() {
                Payload::GlobalSection(section) => Some(section.count()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(count_globals(&debug), count_globals(&release) + 1);
    assert!(release.len() < debug.len());
}

#[test]
fn debug_bindings_support_suspension_and_are_erased_from_release() {
    use splitscript::{BuildProfile, CompilerOptions};

    for binding in [
        "debug let module = await process.module(\"debug-only.dll\")\n\
         debug print(module.address as String)",
        "debug let marker = retry process.read.i32(0)\n\
         debug print(marker as String)",
    ] {
        let source = format!(r#"state "game.exe" {{}} onAttach {{ {binding} }}"#);
        let debug = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Debug,
            },
        )
        .expect("debug suspension bindings should compile");
        let release = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Release,
            },
        )
        .expect("release should type-check and erase debug suspension bindings");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&debug)
            .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&release)
            .unwrap();
        assert!(release.len() < debug.len());
        assert!(!release.windows(10).any(|bytes| bytes == b"debug-only"));
    }
}

#[test]
fn debug_bindings_are_visible_only_from_debug_code() {
    for source in [
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached { print(hidden as String) }
        "#,
        r#"
            state "game.exe" {}
            whileAttached {
                debug let hidden = 1
                print(hidden as String)
            }
        "#,
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached { hidden = 2 }
        "#,
    ] {
        let errors = splitscript::compile(source)
            .expect_err("retained code must not reference an erased binding");
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("debug-only binding `hidden` can only be used from debug code")
        }));
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached {
                debug let local = hidden + 1
                debug print(local as String)
                debug hidden = local
            }
        "#,
    )
    .expect("debug statements may share debug globals and local bindings");
}

#[test]
fn debug_modifier_rejects_terminators_and_duplicates() {
    for statement in ["debug return", "debug throw \"failure\""] {
        let source = format!(r#"state "game.exe" {{}} onAttach {{ {statement} }}"#);
        let errors = splitscript::compile(&source).expect_err("unsupported debug form must fail");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("`debug` currently supports"))
        );
    }

    let errors = splitscript::compile(
        r#"state "game.exe" {} whileAttached { debug debug print("nested") }"#,
    )
    .expect_err("duplicate debug modifiers must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("more than one `debug` modifier"))
    );

    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            debug fn trace() { print("trace") }
            whileAttached { trace() }
        "#,
    )
    .expect_err("release-visible code must not call a debug-only function");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("debug-only function `trace` can only be called from debug code")
    }));
}

#[test]
fn compiles_a_complete_autosplitter_to_valid_wasm_gc() {
    let wasm = splitscript::compile(EXAMPLE).expect("example should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generated WebAssembly GC should validate");
    assert!(
        wasm.windows("splitscript".len())
            .any(|bytes| bytes == b"splitscript")
    );
}

#[test]
fn linear_memory_grows_beyond_runtime_scratch_for_large_static_data() {
    let source = format!(
        "state \"game.exe\" {{}}\nwhileAttached {{ print(\"{}\") }}",
        "x".repeat(70_000)
    );
    let wasm = splitscript::compile(&source).expect("large static strings should compile");
    let minimum_pages = Parser::new(0)
        .parse_all(&wasm)
        .find_map(
            |payload| match payload.expect("generated module should parse") {
                Payload::MemorySection(memories) => Some(
                    memories
                        .into_iter()
                        .next()
                        .expect("generated module should declare memory")
                        .expect("generated memory should parse")
                        .initial,
                ),
                _ => None,
            },
        )
        .expect("generated module should contain a memory section");

    assert_eq!(minimum_pages, 3);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("large static-data WebAssembly GC should validate");
}

#[test]
fn linear_memory_moves_static_data_after_large_read_scratch() {
    let chunk_fields = (0..32)
        .map(|index| format!("field{index}: u64"))
        .collect::<Vec<_>>()
        .join("\n");
    let large_fields = (0..260)
        .map(|index| format!("chunk{index}: Chunk"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        r#"
            record Chunk {{
                {chunk_fields}
            }}
            record Large {{
                {large_fields}
            }}
            state "game.exe" {{}}
            whileAttached {{
                let value: Large! = process.read(0x100)
            }}
        "#
    );
    let wasm =
        splitscript::compile(&source).expect("large readable records should size scratch storage");
    let minimum_pages = Parser::new(0)
        .parse_all(&wasm)
        .find_map(
            |payload| match payload.expect("generated module should parse") {
                Payload::MemorySection(memories) => Some(
                    memories
                        .into_iter()
                        .next()
                        .expect("generated module should declare memory")
                        .expect("generated memory should parse")
                        .initial,
                ),
                _ => None,
            },
        )
        .expect("generated module should contain a memory section");

    assert!(minimum_pages >= 2);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("large-record WebAssembly GC should validate");
}

#[test]
fn generated_module_requires_gc() {
    let wasm = splitscript::compile(EXAMPLE).expect("example should compile");
    let features = WasmFeatures::all() - WasmFeatures::GC;
    assert!(
        Validator::new_with_features(features)
            .validate_all(&wasm)
            .is_err()
    );
}

#[test]
fn compiles_attach_await_and_print_hello_world() {
    let wasm = splitscript::compile(HELLO).expect("hello world should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("hello world WebAssembly GC should validate");
    for expected in [
        b"Lunistice-Demo.exe".as_slice(),
        b"GameAssembly.dll".as_slice(),
        b"Hello, world from SplitScript!".as_slice(),
    ] {
        assert!(wasm.windows(expected.len()).any(|bytes| bytes == expected));
    }
}

#[test]
fn compiles_the_complete_settings_showcase() {
    let checked = splitscript::check(splitscript::parse(SETTINGS_EXAMPLE).unwrap())
        .expect("settings example should type-check");
    let choice = checked
        .syntax()
        .settings
        .iter()
        .find(|setting| {
            matches!(
                setting.kind,
                splitscript::compiler::ast::SettingKind::Choice { .. }
            )
        })
        .expect("settings example has a choice");
    let splitscript::compiler::ast::SettingKind::Choice {
        enumeration,
        default_variant,
        options,
    } = &choice.kind
    else {
        unreachable!();
    };
    let declaration = checked
        .syntax()
        .enums
        .iter()
        .find(|item| Some(item.id) == enumeration.source())
        .unwrap();
    let expected_default = declaration
        .variants
        .iter()
        .find(|variant| variant.name == *default_variant)
        .unwrap()
        .id;
    assert_eq!(
        checked.semantics().setting_choice_default(choice.id),
        Some(expected_default)
    );
    assert_eq!(
        checked.typed_hir().setting_choice_default(choice.id),
        Some(expected_default)
    );
    for option in options {
        let expected = declaration
            .variants
            .iter()
            .find(|variant| variant.name == option.variant)
            .unwrap()
            .id;
        assert_eq!(
            checked.semantics().setting_choice_option(option.id),
            Some(expected)
        );
        assert_eq!(
            checked.typed_hir().setting_choice_option(option.id),
            Some(expected)
        );
    }

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("settings example WebAssembly GC should validate");
    for expected in [
        b"Enable Auto Splitting".as_slice(),
        b"Capture Source".as_slice(),
        b"Layout File".as_slice(),
        b"image/*".as_slice(),
    ] {
        assert!(wasm.windows(expected.len()).any(|bytes| bytes == expected));
    }
}
