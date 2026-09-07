use std::{collections::BTreeMap, sync::Arc};

use super::CompilerDatabase;
use crate::{DiagnosticCode, WarningLevel, WarningPolicy, typeck::inference_run_count};

const SOURCE: &str = "// Unicode 🦊\nstate \"game.exe\" { level: u32 at 0x100 }\n\
    fn identity(value) { return value }\n\
    whileAttached { let f = value => value; print(identity(f(current.level))) }\n";

#[test]
fn failed_strict_checks_retain_inference_and_validation_results() {
    for (suffix, has_effects) in [
        ("fn broken() -> bool { return 42 }", false),
        (
            "fn broken() { let value = { 1; }; return missingName }",
            false,
        ),
        (
            "fn readValue() -> u32! { return process.read<u32>(0) }\n\
             onDetach { let value = readValue() }",
            true,
        ),
    ] {
        let source = format!("{SOURCE}{suffix}");
        let mut database = CompilerDatabase::with_source_name("reuse.split", &source);
        let lowered = database.lower().unwrap(); // Initialize the graph outside the count.
        let before = inference_run_count();
        let strict = database.check().unwrap_err();
        assert_eq!(inference_run_count() - before, 1);
        let recovered = database.recovering_check().unwrap();
        assert_eq!(
            inference_run_count() - before,
            1,
            "recovery reran inference"
        );
        assert!(Arc::ptr_eq(&strict, &database.check().unwrap_err()));
        assert_eq!(recovered.source_name(), "reuse.split");
        assert_eq!(
            recovered.effects().is_some(),
            has_effects,
            "{:?}",
            recovered.diagnostics()
        );

        // Compare independently executed public APIs, including diagnostics,
        // visible expression IDs, inferred values, calls, and effect facts.
        assert_eq!(&*strict, crate::check((*lowered).clone()).unwrap_err());
        let reference = crate::check_recovering((*lowered).clone());
        assert_eq!(recovered.diagnostics(), reference.diagnostics());
        assert_eq!(recovered.effects(), reference.effects());
        assert_eq!(
            recovered
                .semantics()
                .expression_types()
                .collect::<BTreeMap<_, _>>(),
            reference
                .semantics()
                .expression_types()
                .collect::<BTreeMap<_, _>>()
        );
        assert_eq!(
            recovered
                .semantics()
                .value_types()
                .collect::<BTreeMap<_, _>>(),
            reference
                .semantics()
                .value_types()
                .collect::<BTreeMap<_, _>>()
        );
        assert_eq!(
            recovered.semantics().calls().collect::<BTreeMap<_, _>>(),
            reference.semantics().calls().collect::<BTreeMap<_, _>>()
        );
    }
}

#[test]
fn failures_before_inference_keep_the_recovery_path() {
    for suffix in ["fn broken(", "fn broken(value: Mystery) {}"] {
        let mut database = CompilerDatabase::new(format!("{SOURCE}{suffix}"));
        database.recovering_parse().unwrap();
        let before = inference_run_count();
        assert!(database.check().is_err());
        assert_eq!(inference_run_count(), before);
        let recovered = database.recovering_check().unwrap();
        assert_eq!(inference_run_count() - before, 1);
        assert!(!recovered.diagnostics().is_empty());
        assert!(database.check().is_err());
        assert_eq!(inference_run_count() - before, 1);
    }
}

#[test]
fn reused_inference_survives_policy_changes_and_invalidates_on_edits() {
    let mut database = CompilerDatabase::new(format!(
        "{SOURCE}fn readValue() -> u32! {{ return process.read<u32>(0) }}\n\
         onDetach {{ let unread = readValue() }}"
    ));
    database.lower().unwrap();
    let before = inference_run_count();
    assert!(database.check().is_err());
    let recovered = database.recovering_check().unwrap();
    let mut policy = WarningPolicy::default();
    for level in [WarningLevel::Deny, WarningLevel::Allow] {
        policy.set(DiagnosticCode::UnusedBinding, level);
        assert!(database.set_warning_policy(policy));
        let diagnostics = database.diagnostics();
        assert!(!diagnostics.is_empty());
        assert_eq!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::UnusedBinding),
            level == WarningLevel::Deny
        );
        assert!(database.semantic_snapshot().unwrap().checked().is_none());
        assert!(Arc::ptr_eq(
            &recovered,
            &database.recovering_check().unwrap()
        ));
        assert_eq!(inference_run_count() - before, 1);
    }

    assert!(database.set_source(SOURCE));
    assert!(database.check().is_ok());
    assert!(database.semantic_snapshot().unwrap().checked().is_some());
    assert_eq!(inference_run_count() - before, 2);
    assert!(database.set_source(format!("{SOURCE}fn broken() {{ return missingName }}")));
    assert!(database.check().is_err());
    let edited = database.recovering_check().unwrap();
    assert!(!Arc::ptr_eq(&recovered, &edited));
    assert_eq!(inference_run_count() - before, 3);
}

#[test]
fn strict_check_does_not_replace_an_existing_recovery_snapshot() {
    let mut database =
        CompilerDatabase::new(format!("{SOURCE}fn broken() {{ return missingName }}"));
    database.lower().unwrap();
    let before = inference_run_count();
    let recovered = database.recovering_check().unwrap();
    assert!(database.check().is_err());
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_check().unwrap()
    ));
    assert_eq!(inference_run_count() - before, 1);
}

#[test]
fn successful_checks_share_recovery_facts_in_either_query_order() {
    for source in [
        SOURCE.to_owned(),
        format!("{SOURCE}fn warning() {{ return {{ 1; }} }}"),
        "state \"game.exe\" {} whileAttached { let unread = 1 }".to_owned(),
    ] {
        for recovery_first in [false, true] {
            let mut database = CompilerDatabase::with_source_name("shared.split", &source);
            let lowered = database.lower().unwrap();
            let before = inference_run_count();
            if recovery_first {
                database.recovering_check().unwrap();
            }
            let checked = database.check().unwrap();
            let recovered = database.recovering_check().unwrap();
            assert_eq!(inference_run_count() - before, 1);
            assert!(Arc::ptr_eq(
                &lowered.compilation_syntax,
                &checked.compilation_syntax
            ));
            assert!(std::ptr::eq(checked.semantics(), recovered.semantics()));
            assert!(std::ptr::eq(checked.syntax(), recovered.syntax()));
            assert!(std::ptr::eq(
                checked.source_document(),
                recovered.source_document()
            ));
            assert!(std::ptr::eq(checked.hir(), recovered.hir()));
            let cloned = (*recovered).clone();
            assert!(std::ptr::eq(cloned.semantics(), checked.semantics()));

            let independent = crate::check_recovering((*lowered).clone());
            assert_eq!(recovered.diagnostics(), independent.diagnostics());
            assert_eq!(recovered.effects(), independent.effects());
            assert_eq!(recovered.source_name(), independent.source_name());
            assert_eq!(recovered.context(), independent.context());
            assert_eq!(
                recovered
                    .semantics()
                    .expression_types()
                    .collect::<BTreeMap<_, _>>(),
                independent
                    .semantics()
                    .expression_types()
                    .collect::<BTreeMap<_, _>>()
            );
        }
    }
}

#[test]
fn shared_successful_recovery_survives_policy_changes_and_source_revisions() {
    let source = "state \"game.exe\" {} whileAttached { let unread = 1 }";
    let mut database = CompilerDatabase::new(source);
    database.lower().unwrap();
    let before = inference_run_count();
    let recovered = database.recovering_check().unwrap();
    let checked = database.check().unwrap();
    let mut policy = WarningPolicy::default();
    for level in [WarningLevel::Deny, WarningLevel::Allow] {
        policy.set(DiagnosticCode::UnusedBinding, level);
        database.set_warning_policy(policy);
        assert_eq!(
            database.diagnostics().is_empty(),
            level == WarningLevel::Allow
        );
        assert!(Arc::ptr_eq(&checked, &database.check().unwrap()));
        assert!(Arc::ptr_eq(
            &recovered,
            &database.recovering_check().unwrap()
        ));
        assert_eq!(inference_run_count() - before, 1);
    }

    database.set_source("state \"game.exe\" {} whileAttached { let broken: bool = 42 }");
    let failed = database.recovering_check().unwrap();
    assert!(database.check().is_err());
    assert_eq!(inference_run_count() - before, 2);
    assert!(!std::ptr::eq(failed.semantics(), recovered.semantics()));
    assert_eq!(recovered.source_document().source(), source);
    assert!(std::ptr::eq(recovered.semantics(), checked.semantics()));
}

#[test]
fn recovery_first_validation_failure_runs_inference_once() {
    let mut database = CompilerDatabase::new(format!(
        "{SOURCE}fn readValue() -> u32! {{ return process.read<u32>(0) }}\n\
         onDetach {{ let value = readValue() }}"
    ));
    database.lower().unwrap();
    let before = inference_run_count();
    let recovered = database.recovering_check().unwrap();
    assert!(recovered.effects().is_some());
    assert!(database.check().is_err());
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_check().unwrap()
    ));
    assert_eq!(inference_run_count() - before, 1);
}

#[test]
fn cloned_lowered_programs_share_augmented_syntax_with_and_without_library_bodies() {
    for include_bodies in [false, true] {
        let context = if include_bodies {
            crate::CompilerContext::default()
        } else {
            crate::CompilerContext::default().without_standard_library_bodies()
        };
        let lowered = crate::lower(crate::parse_with_context(context, SOURCE).unwrap());
        assert_eq!(
            lowered.compilation_syntax.functions.len() > lowered.syntax().functions.len(),
            include_bodies
        );
        let cloned = lowered.clone();
        assert!(Arc::ptr_eq(
            &lowered.compilation_syntax,
            &cloned.compilation_syntax
        ));
        let recovered = crate::check_recovering(cloned.clone());
        if include_bodies {
            let checked = crate::check(cloned).unwrap();
            assert!(Arc::ptr_eq(
                &lowered.compilation_syntax,
                &checked.compilation_syntax
            ));
            assert_eq!(checked.diagnostics(), recovered.diagnostics());
            assert_eq!(checked.source_document().source(), SOURCE);
        } else {
            // Signature-only documentation contexts omit required bodies and
            // must still fail strict body validation rather than becoming
            // compilable merely because the syntax storage is shared.
            let errors = crate::check(cloned).unwrap_err();
            assert_eq!(errors.as_slice(), recovered.diagnostics());
        }
    }
}

#[test]
fn augmented_syntax_is_revision_scoped_and_old_checked_programs_remain_compilable() {
    let mut database = CompilerDatabase::new(SOURCE);
    let original = database.check().unwrap();
    let options = crate::CompilerOptions {
        profile: crate::BuildProfile::Release,
        ..Default::default()
    };
    let original_wasm = crate::codegen_with_options(&original, options);
    // Keep the earlier program alive while another revision gets new spans
    // and library identities. Neither checking nor code generation may mutate
    // the shared tree belonging to the old revision.
    database.set_source(format!("// Another revision 🦊\n{SOURCE}"));
    let edited = database.check().unwrap();
    assert!(!Arc::ptr_eq(
        &original.compilation_syntax,
        &edited.compilation_syntax
    ));
    assert!(Arc::ptr_eq(
        &edited.compilation_syntax,
        &database.lower().unwrap().compilation_syntax
    ));
    assert_eq!(original.source_document().source(), SOURCE);
    assert_eq!(
        crate::codegen_with_options(&original, options),
        original_wasm
    );
    assert_eq!(crate::codegen_with_options(&edited, options), original_wasm);
}
