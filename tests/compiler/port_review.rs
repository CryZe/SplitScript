//! Cross-cutting regression fixtures for reviewing compiler-clean ports.
//!
//! Provider unit tests prove the individual implementations. These minimized
//! cases instead preserve the queries used to notice a port that compiles but
//! selected the wrong attachment model or silently dropped source behavior.

use splitscript::{
    DocumentationReference, compiler::stdlib::StdlibStateProviderId,
    tooling::database::CompilerDatabase,
};

fn checked_without_warnings(source: &str) -> std::sync::Arc<splitscript::CheckedProgram> {
    let mut database = CompilerDatabase::new(source);
    let checked = database
        .check()
        .unwrap_or_else(|diagnostics| panic!("review fixture did not check: {diagnostics:#?}"))
        .clone();
    assert!(
        database.diagnostics().is_empty(),
        "a canonical review fixture should not hide drift behind warnings: {:#?}",
        database.diagnostics(),
    );
    checked
}

fn assert_search_leads_to(query: &str, expected_uri: &str) {
    let results = DocumentationReference::default().search(query);
    assert!(
        results.iter().any(|entry| entry.uri == expected_uri),
        "`{query}` did not lead to `{expected_uri}`: {results:#?}",
    );
}

#[test]
fn native_review_fixture_preserves_identity_build_lifecycle_settings_width_and_failure() {
    let source = r#"
        settings {
            "Enable checkpoint splits" => checkpointSplits: true,
        }

        state ["game.exe", "game-demo.exe"] {
            layout Retail {
                checkpoint: u16 at "engine.dll", 0x1000;
            },
            layout Demo {
                checkpoint: u16 at "engine.dll", 0x2000;
            },
        }

        onAttach {
            let executable = await process.mainModule()
            if executable.size == 10_000 { return StateLayout.Retail }
            if executable.size == 20_000 { return StateLayout.Demo }
            await process.closed()
        }

        split {
            return settings.checkpointSplits
                && old.checkpoint != current.checkpoint
        }
    "#;

    let checked = checked_without_warnings(source);
    let state = checked.syntax().state.as_ref().expect("state declaration");
    assert_eq!(state.processes, ["game.exe", "game-demo.exe"]);
    assert_eq!(state.layouts.len(), 2, "build choice must remain explicit");
    assert!(state.all_fields().all(|field| {
        field
            .annotation
            .is_some_and(|annotation| annotation.to_string() == "u16")
    }));
    assert!(
        checked
            .syntax()
            .actions
            .iter()
            .any(|action| { action.kind == splitscript::ast::ActionKind::OnAttach })
    );
    assert!(
        checked
            .syntax()
            .actions
            .iter()
            .any(|action| { action.kind == splitscript::ast::ActionKind::Split })
    );

    assert_search_leads_to(".exe", "/language/state.md");
    assert_search_leads_to(
        "version labelled states",
        "/migration/asl/state/version-label.md",
    );
    assert_search_leads_to(
        "settings Add loop",
        "/migration/asl/settings/registration.md",
    );
    for query in ["DeepPointer", "ASL native state numeric root"] {
        assert_search_leads_to(query, "/migration/asl/memory/deep-pointer.md");
    }
    let pointer_migration = DocumentationReference::default()
        .page("/migration/asl/memory/deep-pointer.md")
        .expect("the native pointer migration page should exist");
    assert!(pointer_migration.markdown.contains("main-module-relative"));
    assert!(
        pointer_migration
            .markdown
            .contains("`at offset` form is an absolute virtual address")
    );
    assert!(
        pointer_migration
            .markdown
            .contains("asl-numeric-roots-are-module-relative")
    );
}

#[test]
fn unity_review_fixture_uses_the_schema_provider_and_rejects_manual_traversal() {
    let source = r#"
        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance;
                u32 checkpoint;
            }
        }

        state Unity ["game.exe"] {
            checkpoint: u32 = GameManager.instance?.checkpoint?
        }

        split { return old.checkpoint != current.checkpoint }
    "#;

    let checked = checked_without_warnings(source);
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Unity),
    );
    assert_eq!(checked.syntax().managed_images.len(), 1);

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach { let runtime = await Unity.mono(MonoVersion.V2) }
        "#,
    )
    .expect_err("manual Unity traversal must guide users to the schema provider");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.migration_topic() == Some("asl.unity.managed-schema")
            && diagnostic.message.contains("`image` schema")
    }));

    assert_search_leads_to("UnityASL", "/migration/asl/unity/managed-schema.md");
    assert_search_leads_to("mono.Make", "/migration/asl/unity/managed-schema.md");
}

#[test]
fn emulator_review_fixture_keeps_guest_addresses_and_provider_owned_byte_order() {
    let source = r#"
        struct Snapshot {
            checkpoint: u16,
            flags: u32,
        }

        state GCN {
            snapshot: Snapshot at 0x80001000;
        }

        split {
            return old.snapshot.checkpoint != current.snapshot.checkpoint
                || old.snapshot.flags != current.snapshot.flags
        }
    "#;

    let checked = checked_without_warnings(source);
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Gcn),
    );
    assert!(
        checked
            .syntax()
            .state
            .as_ref()
            .unwrap()
            .processes
            .is_empty()
    );

    let diagnostics =
        splitscript::compile("state GCN { value: u32 = process.read(0x80001000) else 0 }")
            .expect_err("native reads must not bypass an emulator provider");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`process` is unavailable under `state GCN`; use `gcn` instead")
    }));

    assert_search_leads_to("Dolphin", "/stdlib/state-providers/GCN.md");
    assert_search_leads_to("DeepPointer", "/stdlib/state-providers/GCN.md");
}

#[test]
fn review_fixture_keeps_reachability_and_failure_diagnostics_visible() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
                settings { "Unused route" => unusedRoute: true }
                state "game.exe" {}
            "#,
        )
        .unwrap(),
    )
    .expect("an unused setting is a warning");
    assert!(
        checked
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.message == "unused setting `unusedRoute`" })
    );

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach { let module: Module = process.mainModule() }
        "#,
    )
    .expect_err("fallible behavior must be handled explicitly");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("async") || diagnostic.message.contains("await")
    }));

    assert_search_leads_to("TryParse", "/migration/string/numeric-parse.md");
}
