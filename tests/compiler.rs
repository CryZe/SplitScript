use wasmparser::{Parser, Payload, TypeRef, Validator, WasmFeatures};

use splitscript::{
    abi::{AbiCatalog, AbiEffect, AbiImportId, AbiOwnership},
    language::{LanguageCatalog, LanguageItemId, LanguageItemKind},
    semantic::{ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue},
    stdlib::{
        Availability, CancellationKind, Effect, StandardLibrary, StdlibFieldId, StdlibItemId,
        StdlibTypeId, StdlibVariantId, SuspensionKind,
    },
    types::{BuiltinType, TypeKind},
};

const EXAMPLE: &str = include_str!("../examples/lunistice.split");
const HELLO: &str = include_str!("../examples/hello_lunistice.split");
const SETTINGS_EXAMPLE: &str = include_str!("../examples/lso_desktop_settings.split");

#[test]
fn vscode_manifest_tracks_the_lsp_semantic_token_legend() {
    use std::collections::BTreeSet;

    use serde_json::Value;
    use splitscript::highlight::{SEMANTIC_TOKEN_MODIFIERS, SemanticTokenKind};

    let manifest: Value = serde_json::from_str(include_str!("../editors/vscode/package.json"))
        .expect("VS Code manifest should be valid JSON");
    serde_json::from_str::<Value>(include_str!(
        "../editors/vscode/language-configuration.json"
    ))
    .expect("language configuration should be valid JSON");
    let grammar: Value = serde_json::from_str(include_str!(
        "../editors/vscode/syntaxes/splitscript.tmLanguage.json"
    ))
    .expect("fallback grammar should be valid JSON");

    assert_eq!(
        manifest["contributes"]["languages"][0]["extensions"][0],
        ".split"
    );
    assert_eq!(grammar["scopeName"], "source.splitscript");
    assert_eq!(
        manifest["contributes"]["configurationDefaults"]["[splitscript]"]["editor.semanticHighlighting.enabled"],
        true
    );
    let commands = manifest["contributes"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["command"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for command in [
        "splitscript.buildRelease",
        "splitscript.startDebugWatch",
        "splitscript.stopDebugWatch",
    ] {
        assert!(commands.contains(command));
    }
    let editor_actions = manifest["contributes"]["menus"]["editor/title"]
        .as_array()
        .unwrap();
    for command in [
        "splitscript.buildRelease",
        "splitscript.startDebugWatch",
        "splitscript.stopDebugWatch",
    ] {
        assert!(editor_actions.iter().any(|menu| menu["command"] == command));
    }
    assert!(
        manifest["contributes"]["configuration"]["properties"]["splitScript.compiler.profile"]
            .is_null()
    );

    let standard_types = BTreeSet::from([
        "namespace",
        "type",
        "class",
        "enum",
        "interface",
        "struct",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "enumMember",
        "event",
        "function",
        "method",
        "macro",
        "label",
        "comment",
        "string",
        "keyword",
        "number",
        "regexp",
        "operator",
        "decorator",
    ]);
    let expected_types = SemanticTokenKind::ALL
        .iter()
        .map(|kind| kind.name())
        .filter(|name| !standard_types.contains(name))
        .collect::<BTreeSet<_>>();
    let contributed_types = manifest["contributes"]["semanticTokenTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(contributed_types, expected_types);

    let standard_modifiers = BTreeSet::from([
        "declaration",
        "definition",
        "readonly",
        "static",
        "deprecated",
        "abstract",
        "async",
        "modification",
        "documentation",
        "defaultLibrary",
    ]);
    let expected_modifiers = SEMANTIC_TOKEN_MODIFIERS
        .iter()
        .copied()
        .filter(|modifier| !standard_modifiers.contains(modifier))
        .collect::<BTreeSet<_>>();
    let contributed_modifiers = manifest["contributes"]["semanticTokenModifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(contributed_modifiers, expected_modifiers);
}

#[test]
fn compiler_stages_retain_the_lossless_source_document() {
    let source = "/* lead */\r\nstate \"game.exe\" { /* body */ } // tail\r\n";
    let parsed = splitscript::parse(source).expect("commented source should parse");
    assert_eq!(parsed.source_document().reconstruct(), source);
    assert_eq!(parsed.source_document().source(), source);

    let lowered = splitscript::lower(parsed);
    assert_eq!(lowered.source_document().reconstruct(), source);

    let checked = splitscript::check(lowered).expect("commented source should check");
    assert_eq!(checked.source_document().reconstruct(), source);
    assert_eq!(checked.source_document().trivia().count(), 10);
}

#[test]
fn compiler_database_caches_queries_and_invalidates_on_source_changes() {
    use std::sync::Arc;

    use splitscript::{DiagnosticCode, database::CompilerDatabase};

    let valid = r#"
        state "game.exe" {}
        whileAttached { print("cached") }
    "#;
    let mut database = CompilerDatabase::new(valid);
    assert_eq!(database.revision().index(), 0);

    let recovered = database.recovering_parse().unwrap();
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_parse().unwrap()
    ));
    let parsed = database.parse().unwrap();
    assert!(Arc::ptr_eq(&parsed, &database.parse().unwrap()));
    let lowered = database.lower().unwrap();
    assert!(Arc::ptr_eq(&lowered, &database.lower().unwrap()));
    let checked = database.check().unwrap();
    assert!(Arc::ptr_eq(&checked, &database.check().unwrap()));
    let no_diagnostics = database.diagnostics();
    assert!(no_diagnostics.is_empty());
    assert!(Arc::ptr_eq(&no_diagnostics, &database.diagnostics()));

    assert!(!database.set_source(valid));
    assert_eq!(database.revision().index(), 0);
    assert!(Arc::ptr_eq(&checked, &database.check().unwrap()));

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let broken = +
            print("partial syntax remains available")
        }
    "#;
    assert!(database.set_source(invalid));
    assert_eq!(database.revision().index(), 1);
    let recovered_error = database.recovering_parse().unwrap();
    assert_eq!(recovered_error.diagnostics().len(), 1);
    assert_eq!(recovered_error.syntax().actions[0].body.statements.len(), 2);
    let parse_errors = database.parse().unwrap_err();
    assert_eq!(parse_errors[0].code, DiagnosticCode::Syntax);
    assert!(Arc::ptr_eq(&parse_errors, &database.diagnostics()));

    assert!(database.set_source(valid));
    assert_eq!(database.revision().index(), 2);
    let rechecked = database.check().unwrap();
    assert!(!Arc::ptr_eq(&checked, &rechecked));
}

#[test]
fn compiler_database_caches_formatting_without_type_checking() {
    use std::sync::Arc;

    use splitscript::database::CompilerDatabase;

    let source = "state \"game.exe\"{}\nwhileAttached{let broken:bool=42}";
    let mut database = CompilerDatabase::new(source);
    assert!(database.check().is_err());

    let formatted = database.format().unwrap();
    assert!(Arc::ptr_eq(&formatted, &database.format().unwrap()));
    assert_eq!(
        &*formatted,
        "state \"game.exe\" {}\nwhileAttached {\n    let broken: bool = 42\n}\n"
    );

    assert!(database.set_source("state \"game.exe\" {"));
    assert!(database.format().is_err());
}

#[test]
fn compiler_database_exposes_types_resolutions_and_references() {
    use std::sync::Arc;

    use splitscript::{
        ast::{ExprKind, StateSource, Stmt},
        database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId, ValueReferenceKind},
        hir::{DeclarationId, ExpressionResolution},
        semantic::{ResolvedCall, ResolvedValue},
        stdlib::StdlibItemId,
        types::{BuiltinType, TypeKind},
    };

    let source = r#"
        let global = 1
        state "game.exe" {
            level: i32 at 0x1000
        }
        fn bump(value) {
            let local = value + global.min(10)
            global += local
            return local
        }
        whileAttached {
            let result = bump(current.level)
            print(result as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let syntax = checked.syntax();
    let global = syntax.globals[0].id;
    let state = syntax.state.as_ref().unwrap().fields[0].id;
    assert!(matches!(
        syntax.state.as_ref().unwrap().fields[0].source,
        StateSource::Pointer(_)
    ));
    let function = &syntax.functions[0];
    let parameter = function.params[0].id;
    let Stmt::Variable(local) = &function.body.statements[0] else {
        panic!("expected the local declaration");
    };
    let local_id = local.id;
    let ExprKind::Binary { right, .. } = &local.value.kind else {
        panic!("expected the local binary initializer");
    };
    let min_call = right.id;
    let Stmt::Assign { id: assignment, .. } = function.body.statements[1] else {
        panic!("expected the global assignment");
    };
    let Stmt::Variable(result) = &syntax.actions[0].body.statements[0] else {
        panic!("expected the result declaration");
    };
    let bump_call = result.value.id;
    let ExprKind::Call { args, .. } = &result.value.kind else {
        panic!("expected the user-function call");
    };
    let state_path = args[0].id;

    let declarations = database.declarations_named("bump").unwrap();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].id, DeclarationId::Function(function.id));

    let local_type = database.value_type(local_id).unwrap().unwrap();
    assert_eq!(
        database.type_kind(local_type).unwrap(),
        Some(TypeKind::Builtin(BuiltinType::I32))
    );
    assert_eq!(
        database.expression_type(local.value.id).unwrap(),
        Some(local_type)
    );
    assert_eq!(database.value_type(parameter).unwrap(), Some(local_type));
    assert_eq!(
        database.function_result_type(function.id).unwrap(),
        Some(local_type)
    );

    assert!(matches!(
        database.resolved_call(bump_call).unwrap(),
        Some(ResolvedCall::UserFunction { function: target }) if target == function.id
    ));
    assert!(matches!(
        database.resolved_call(min_call).unwrap(),
        Some(ResolvedCall::StandardLibrary {
            receiver: Some(ResolvedValue::Variable(target)),
            ..
        }) if target == global
    ));
    assert!(matches!(
        database.resolved_path(state_path).unwrap(),
        Some(path) if path.root == Some(ResolvedValue::CurrentState(state))
    ));
    assert_eq!(
        database.assignment_target(assignment).unwrap(),
        Some(global)
    );

    let references = database.reference_index().unwrap();
    assert!(Arc::ptr_eq(
        &references,
        &database.reference_index().unwrap()
    ));
    assert_eq!(
        references
            .references_to(global)
            .iter()
            .map(|reference| reference.kind)
            .collect::<Vec<_>>(),
        [ValueReferenceKind::Read, ValueReferenceKind::Write]
    );
    assert_eq!(references.references_to(parameter).len(), 1);
    assert_eq!(references.references_to(local_id).len(), 2);
    assert_eq!(references.references_to(state).len(), 1);
    assert_eq!(references.references_to(result.id).len(), 1);

    let min_position = source.find("global.min").unwrap() + 1;
    let min_analysis = database.analysis_at(min_position).unwrap().unwrap();
    assert_eq!(min_analysis.expression, min_call);
    assert_eq!(min_analysis.type_kind, TypeKind::Builtin(BuiltinType::I32));
    assert_eq!(
        min_analysis
            .segments
            .iter()
            .map(|segment| (
                &segment.name[..],
                &source[segment.span.start..segment.span.end]
            ))
            .collect::<Vec<_>>(),
        [("global", "global"), ("min", "min")]
    );
    let min_method_position = source.find("global.min").unwrap() + "global.".len();
    assert_eq!(
        min_analysis.segment_at(min_method_position).unwrap().name,
        "min"
    );
    assert!(matches!(
        database.definition_at(min_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(global)
    ));
    assert_eq!(
        database.definition_at(min_method_position).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::NumericMin))
    );
    assert!(matches!(
        min_analysis.resolution,
        Some(ExpressionResolution::Call(ResolvedCall::StandardLibrary {
            receiver: Some(ResolvedValue::Variable(target)),
            ..
        })) if target == global
    ));

    let state_position = source.find("current.level").unwrap() + 2;
    let state_analysis = database.analysis_at(state_position).unwrap().unwrap();
    assert_eq!(state_analysis.expression, state_path);
    assert_eq!(
        state_analysis
            .segments
            .iter()
            .map(|segment| segment.name.as_str())
            .collect::<Vec<_>>(),
        ["current", "level"]
    );
    assert!(matches!(
        state_analysis.resolution,
        Some(ExpressionResolution::ValuePath {
            root: Some(ResolvedValue::CurrentState(target)),
            ..
        }) if target == state
    ));
    assert_eq!(
        database.definition_at(state_position).unwrap(),
        Some(DefinitionTarget::Language(
            splitscript::language::LanguageItemId::CurrentSnapshot
        ))
    );
    let state_field_position = source.find("current.level").unwrap() + "current.".len();
    assert!(matches!(
        database.definition_at(state_field_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(state)
    ));

    let literal_position = source.find("min(10)").unwrap() + "min(".len();
    let literal_analysis = database.analysis_at(literal_position).unwrap().unwrap();
    assert_eq!(
        literal_analysis.type_kind,
        TypeKind::Builtin(BuiltinType::I32)
    );
    assert!(literal_analysis.resolution.is_none());
    assert_eq!(
        &source[literal_analysis.span.start..literal_analysis.span.end],
        "10"
    );
    assert!(database.analysis_at(source.len() + 1).unwrap().is_none());

    let global_token = database.token_at(min_position).unwrap().unwrap();
    assert_eq!(
        &source[global_token.span.start..global_token.span.end],
        "global"
    );
    let trivia_position = source.find("global.min").unwrap() - 1;
    assert!(database.token_at(trivia_position).unwrap().is_none());
}

#[test]
fn source_reference_queries_cover_all_declaration_kinds() {
    use splitscript::database::CompilerDatabase;

    fn spellings(database: &mut CompilerDatabase, source: &str, offset: usize) -> Vec<String> {
        database
            .references_at(offset, true)
            .unwrap()
            .into_iter()
            .map(|span| source[span.start..span.end].to_owned())
            .collect()
    }

    let source = r#"
        record Point { x: i32 }
        enum Mode { Active }
        let total = 0
        state "game.exe" {
            point: Point = process.read(0)
        }
        settings { "General" { "Enabled" => enabled: true } }
        fn inspect(point: Point, mode: Mode) {
            total += point.x
            if mode == Mode.Active && settings.enabled {
                print(total as String)
            }
        }
        whileAttached {
            let point = Point { x: 1 }
            inspect(point, Mode.Active)
            if current.point.x == 1 {}
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database.check().expect("navigation fixture should check");

    for (needle, expected, count) in [
        ("record Point", "Point", 4),
        ("x: i32", "x", 4),
        ("enum Mode", "Mode", 4),
        ("Active }", "Active", 3),
        ("let total", "total", 3),
        ("=> enabled", "enabled", 2),
        ("fn inspect", "inspect", 2),
        ("current.point", "point", 2),
    ] {
        let offset = source.find(needle).unwrap() + needle.rfind(expected).unwrap();
        let references = spellings(&mut database, source, offset);
        assert_eq!(
            references,
            vec![expected; count],
            "references for {expected}"
        );
    }

    let parameter = source.find("point.x").unwrap();
    assert_eq!(
        spellings(&mut database, source, parameter),
        ["point", "point"]
    );
    let local = source.find("inspect(point").unwrap() + "inspect(".len();
    assert_eq!(spellings(&mut database, source, local), ["point", "point"]);

    let call = source.rfind("inspect").unwrap();
    assert_eq!(
        database.references_at(call, false).unwrap().len(),
        1,
        "excluding the declaration leaves only the call"
    );
}

#[test]
fn rename_queries_validate_identifiers_reservations_and_binding_identity() {
    use splitscript::database::{CompilerDatabase, RenameError};

    let source = r#"
        let total = 1
        state "game.exe" {}
        fn show(value: i32) {
            print(total as String)
            print(value as String)
        }
        whileAttached { show(total) }
    "#;
    let mut database = CompilerDatabase::new(source);

    let parameter = source.find("value: i32").unwrap();
    let edits = database.rename_at(parameter, "amount").unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|span| &source[span.start..span.end] == "value")
    );
    let target = database.rename_target_at(parameter).unwrap().unwrap();
    assert_eq!(target.name, "value");
    assert_eq!(&source[target.span.start..target.span.end], "value");

    assert!(matches!(
        database.rename_at(parameter, "2amount"),
        Err(RenameError::InvalidIdentifier)
    ));
    assert!(matches!(
        database.rename_at(parameter, "if"),
        Err(RenameError::ReservedIdentifier)
    ));
    assert!(matches!(
        database.rename_at(parameter, "total"),
        Err(RenameError::ConflictingBinding)
    ));

    let print = source.find("print").unwrap();
    assert!(database.rename_target_at(print).unwrap().is_none());
    assert!(matches!(
        database.rename_at(print, "write"),
        Err(RenameError::NotRenameable)
    ));
}

#[test]
fn document_symbols_preserve_source_order_and_domain_hierarchy() {
    use std::sync::Arc;

    use splitscript::{database::CompilerDatabase, symbols::DocumentSymbolKind};

    let source = r#"
        record Point { x: i32 }
        let global = 1
        state "game.exe" { level = process.read.i32(0) }
        settings {
            "General" {
                "Timing" {
                    "Enabled" => enabled: true
                }
            }
        }
        enum Mode { Active }
        fn Point.isOrigin() { return self.x == 0 }
        whileAttached {}
    "#;
    let mut database = CompilerDatabase::new(source);
    let symbols = database.document_symbols().unwrap();
    assert!(Arc::ptr_eq(&symbols, &database.document_symbols().unwrap()));
    assert_eq!(
        symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Point",
            "global",
            "state",
            "settings",
            "Mode",
            "isOrigin",
            "whileAttached"
        ]
    );

    let record = &symbols[0];
    assert_eq!(record.kind, DocumentSymbolKind::Struct);
    assert_eq!(record.children[0].name, "x");
    assert_eq!(record.children[0].kind, DocumentSymbolKind::Field);

    let state = &symbols[2];
    assert_eq!(state.kind, DocumentSymbolKind::Namespace);
    assert_eq!(state.children[0].name, "level");

    let settings = &symbols[3];
    assert_eq!(settings.children[0].name, "General");
    assert_eq!(settings.children[0].children[0].name, "Timing");
    assert_eq!(settings.children[0].children[0].children[0].name, "enabled");
    assert_eq!(
        settings.children[0].children[0].children[0].kind,
        DocumentSymbolKind::Property
    );

    assert_eq!(symbols[5].kind, DocumentSymbolKind::Method);
    assert_eq!(symbols[6].kind, DocumentSymbolKind::Event);
    for symbol in symbols.iter() {
        assert!(symbol.range.start <= symbol.selection_range.start);
        assert!(symbol.selection_range.end <= symbol.range.end);
    }
}

#[test]
fn compiler_database_preserves_semantics_around_type_errors() {
    use std::sync::Arc;

    use splitscript::{
        DiagnosticCode,
        ast::Stmt,
        database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
        hir::ExpressionResolution,
        semantic::ResolvedCall,
        types::{BuiltinType, TypeKind},
    };

    let source = r#"
        record Counter { value: i32 }
        state "game.exe" {}

        fn readCounter(counter: Counter) -> i32 {
            return counter.value
        }

        whileAttached {
            let counter = Counter { value: 7 }
            let answer = readCounter(counter)
            let broken: bool = 42
            print(answer as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);

    let strict_errors = database.check().unwrap_err();
    assert_eq!(strict_errors.len(), 1);
    assert_eq!(strict_errors[0].code, DiagnosticCode::Type);

    let recovered = database.recovering_check().unwrap();
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_check().unwrap()
    ));
    assert_eq!(recovered.diagnostics().len(), 1);

    let function = recovered.syntax().functions[0].id;
    let Stmt::Variable(answer) = &recovered.syntax().actions[0].body.statements[1] else {
        panic!("expected the unaffected answer declaration");
    };
    let answer_type = database.value_type(answer.id).unwrap().unwrap();
    assert_eq!(
        database.type_kind(answer_type).unwrap(),
        Some(TypeKind::Builtin(BuiltinType::I32))
    );
    assert!(matches!(
        database.resolved_call(answer.value.id).unwrap(),
        Some(ResolvedCall::UserFunction { function: target }) if target == function
    ));

    let call_position = source.rfind("readCounter").unwrap() + 1;
    let analysis = database.analysis_at(call_position).unwrap().unwrap();
    assert_eq!(analysis.type_kind, TypeKind::Builtin(BuiltinType::I32));
    assert!(matches!(
        analysis.resolution,
        Some(ExpressionResolution::Call(ResolvedCall::UserFunction {
            function: target,
        })) if target == function
    ));
    assert!(matches!(
        database.definition_at(call_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Function(function)
    ));

    let argument_position = source.rfind("counter)").unwrap() + 1;
    let Stmt::Variable(counter) = &recovered.syntax().actions[0].body.statements[0] else {
        panic!("expected the counter declaration");
    };
    assert!(matches!(
        database.definition_at(argument_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(counter.id)
    ));
}

#[test]
fn compiler_database_lowers_recovered_declarations_after_syntax_errors() {
    use std::sync::Arc;

    use splitscript::{database::CompilerDatabase, hir::DeclarationId};

    let source = r#"
        let broken = +
        state "game.exe" {}
        fn retained() { return 1 }
        whileAttached { print("retained") }
    "#;
    let mut database = CompilerDatabase::new(source);

    let lowered = database.recovering_lower().unwrap();
    assert!(Arc::ptr_eq(&lowered, &database.recovering_lower().unwrap()));
    let declarations = lowered.hir().declarations().collect::<Vec<_>>();
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "broken" && matches!(declaration.id, DeclarationId::Global(_))
    }));
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "retained" && matches!(declaration.id, DeclarationId::Function(_))
    }));
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "whileAttached" && matches!(declaration.id, DeclarationId::Action(_))
    }));

    let strict_errors = database.lower().unwrap_err();
    assert_eq!(strict_errors.len(), 1);
    assert_eq!(lowered.syntax().globals.len(), 1);
    assert_eq!(lowered.syntax().functions.len(), 1);
    assert_eq!(lowered.syntax().actions.len(), 1);
}

#[test]
fn compiler_database_resolves_expression_segments_to_definitions() {
    use std::sync::Arc;

    use splitscript::{
        database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
        stdlib::StdlibItemId,
    };

    let source = r#"
        record Counter { value: i32 }
        enum Mode {
            Idle
            Active
        }
        let global = 1
        state "game.exe" {}

        fn bump(value) {
            return value.value + global
        }

        whileAttached {
            let counter = Counter { value: 1 }
            let mode = Mode.Active
            let result = bump(counter)
            print(result as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let global = checked.syntax().globals[0].id;
    let function = checked.syntax().functions[0].id;
    let parameter = checked.syntax().functions[0].params[0].id;
    let field = checked.syntax().records[0].fields[0].id;
    let enumeration = checked.syntax().enums[0].id;
    let active = checked.syntax().enums[0].variants[1].id;

    let definitions = database.definition_index().unwrap();
    assert!(Arc::ptr_eq(
        &definitions,
        &database.definition_index().unwrap()
    ));
    assert_eq!(
        definitions
            .get(SourceDefinitionId::Value(global))
            .map(|definition| &source[definition.span.start..definition.span.end]),
        Some("global")
    );

    let value_path = source.find("value.value").unwrap();
    assert!(matches!(
        database.definition_at(value_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(parameter)
    ));
    assert!(matches!(
        database.definition_at(value_path + "value.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::RecordField(field)
    ));

    let global_path = source.find("+ global").unwrap() + 2;
    assert!(matches!(
        database.definition_at(global_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(global)
    ));

    let enum_path = source.find("Mode.Active").unwrap();
    assert!(matches!(
        database.definition_at(enum_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(enumeration)
    ));
    assert!(matches!(
        database.definition_at(enum_path + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(active)
    ));

    let bump_call = source.find("bump(counter)").unwrap();
    assert!(matches!(
        database.definition_at(bump_call + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Function(function)
    ));
    let print_call = source.find("print(result").unwrap();
    assert_eq!(
        database.definition_at(print_call + 1).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::Print))
    );
    assert_eq!(
        database
            .definition_at(source.find("whileAttached").unwrap())
            .unwrap(),
        Some(DefinitionTarget::Language(
            splitscript::language::LanguageItemId::WhileAttached
        ))
    );
}

#[test]
fn compiler_database_resolves_type_record_and_pattern_syntax() {
    use splitscript::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        record Counter { value: i32 }
        record Wrapper { counter: Counter }
        enum Mode {
            Idle
            Active
        }
        enum Event {
            Counter(Counter)
        }
        state "game.exe" {}

        fn identity(value: Counter) -> Counter {
            return value
        }

        fn modeText(mode: Mode) -> String {
            return match mode {
                Mode.Idle => "idle",
                Mode.Active => "active"
            }
        }

        fn eventValue(event: Event) -> i32 {
            return match event {
                Event.Counter(counter) => counter.value
            }
        }

        whileAttached {
            let counter: Counter = Counter { value: 1 }
            print(modeText(Mode.Active))
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let counter = checked.syntax().records[0].id;
    let value_field = checked.syntax().records[0].fields[0].id;
    let mode = checked.syntax().enums[0].id;
    let idle = checked.syntax().enums[0].variants[0].id;

    let wrapper_type = source.find("counter: Counter").unwrap() + "counter: ".len();
    assert!(matches!(
        database.definition_at(wrapper_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Record(counter)
    ));

    let parameter_type = source.find("value: Counter").unwrap() + "value: ".len();
    assert!(matches!(
        database.definition_at(parameter_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Record(counter)
    ));
    let return_type = source.find("-> Counter").unwrap() + "-> ".len();
    assert!(matches!(
        database.definition_at(return_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Record(counter)
    ));
    let payload_type = source.find("Counter(Counter)").unwrap() + "Counter(".len();
    assert!(matches!(
        database.definition_at(payload_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Record(counter)
    ));

    let pattern = source.find("Mode.Idle =>").unwrap();
    assert!(matches!(
        database.definition_at(pattern + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(mode)
    ));
    assert!(matches!(
        database.definition_at(pattern + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(idle)
    ));

    let binding = source.find("Event.Counter(counter)").unwrap() + "Event.Counter(".len();
    let binding_definition = database.definition_at(binding).unwrap().unwrap();
    let binding_use = source.find("counter.value").unwrap();
    assert_eq!(
        database.definition_at(binding_use).unwrap(),
        Some(binding_definition)
    );

    let literal = source.find("Counter { value: 1 }").unwrap();
    assert!(matches!(
        database.definition_at(literal + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Record(counter)
    ));
    assert!(matches!(
        database
            .definition_at(literal + "Counter { ".len())
            .unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::RecordField(value_field)
    ));
}

#[test]
fn recovering_parse_reports_multiple_errors_and_keeps_later_declarations() {
    use splitscript::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        record Broken { value }
        let missingAssignment
        fn retained() { return 1 }
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
            .starts_with("expected `state`, `settings`")
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
fn recovering_parse_keeps_later_statements_in_the_same_block() {
    use splitscript::{ast::Stmt, syntax::RecoveryNodeKind};

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
fn recovering_parse_keeps_later_record_fields_and_enum_variants() {
    use splitscript::syntax::RecoveryNodeKind;

    let source = r#"
        state "game.exe" {}
        record RecoveredRecord {
            first: i32
            missingColon i64
            after: u32
        }
        enum RecoveredEnum {
            First
            Broken(i32
            After
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
    use splitscript::syntax::RecoveryNodeKind;

    let cases = [
        (
            r#"
                state "game.exe" {
                    first: i32 at 0x10
                    broken: i32 nope
                    after: u32 at 0x20
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
fn recovering_parse_keeps_neighboring_settings_in_all_outer_syntaxes() {
    use splitscript::syntax::RecoveryNodeKind;

    let cases = [
        (
            r#"
                state "game.exe" {}
                settings {
                    first: bool = true
                    broken bool = false
                    after: bool = false
                }
                whileAttached { print("still parsed") }
            "#,
            "expected `:` after the setting name",
            vec!["first", "after"],
        ),
        (
            r#"
                state "game.exe" {}
                settings({
                    first: Setting.bool(true),
                    broken Setting.bool(false),
                    after: Setting.bool(false)
                })
                whileAttached { print("still parsed") }
            "#,
            "expected `:` after the setting name",
            vec!["first", "after"],
        ),
        (
            r#"
                state "game.exe" {}
                settings {
                    "Group" {
                        "First" => first: true
                        "Broken" -> broken: true
                        /// Retained tooltip.
                        "After" => after: false
                    }
                    "Outside" => outside: true
                }
                whileAttached { print("still parsed") }
            "#,
            "expected `=>` after the setting description",
            vec!["_heading0", "first", "after", "outside"],
        ),
    ];

    for (source, expected_error, expected_names) in cases {
        let recovered = splitscript::parse_recovering(source).unwrap();
        assert_eq!(recovered.diagnostics().len(), 1);
        assert_eq!(recovered.diagnostics()[0].message, expected_error);
        assert_eq!(
            recovered
                .syntax()
                .settings
                .iter()
                .map(|setting| setting.name.as_str())
                .collect::<Vec<_>>(),
            expected_names
        );
        assert_eq!(recovered.syntax().actions.len(), 1);
        if expected_names.len() == 4 {
            assert_eq!(
                recovered
                    .syntax()
                    .settings
                    .iter()
                    .find(|setting| setting.name == "after")
                    .and_then(|setting| setting.tooltip.as_deref()),
                Some("Retained tooltip.")
            );
        }
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
fn recovering_parse_keeps_valid_choice_options_and_file_filters() {
    use splitscript::{
        ast::{SettingFileFilter, SettingKind},
        syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        enum Mode {
            A
            B
            C
        }
        settings {
            "Mode" => mode: choice {
                "A" => Mode.A default
                "Broken" -> Mode.B
                "C" => Mode.C
            }
            "File" => file: file {
                "Save" => "*.sav"
                "Broken" -> "*.bad"
                mime => "application/octet-stream"
            }
            "After" => after: true
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
    let SettingKind::File { filters } = &recovered.syntax().settings[1].kind else {
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
        SettingFileFilter::Mime(mime) if mime == "application/octet-stream"
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
        ast::{ExprKind, MatchPattern, Stmt},
        syntax::RecoveryNodeKind,
    };

    let source = r#"
        state "game.exe" {}
        enum Mode {
            A
            B
            C
        }
        fn label(mode: Mode) {
            return match mode {
                Mode.A => "A",
                Mode.B -> "Broken",
                Mode.C => "C"
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
    let Stmt::Return {
        value: Some(value), ..
    } = &recovered.syntax().functions[0].body.statements[0]
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
    use splitscript::syntax::RecoveryNodeKind;

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
        ast::{ExprKind, Stmt},
        syntax::RecoveryNodeKind,
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
    let ExprKind::Array(elements) = &values.value.kind else {
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
        ast::{ExprKind, InterpolatedPart, Stmt},
        syntax::RecoveryNodeKind,
    };

    let source = r#"
        record Point {
            x: i32
            y: i32
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
    let ExprKind::Record { fields, .. } = &point.value.kind else {
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
        ast::{ExprKind, Stmt},
        syntax::RecoveryNodeKind,
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
        missing_right.value.kind,
        ExprKind::Binary { ref right, .. } if matches!(right.kind, ExprKind::Error)
    ));

    let Stmt::Variable(missing_unary) = &statements[2] else {
        panic!("expected the unary initializer to be retained");
    };
    assert!(matches!(
        missing_unary.value.kind,
        ExprKind::Unary { ref expr, .. } if matches!(expr.kind, ExprKind::Error)
    ));

    let Stmt::Variable(empty_group) = &statements[4] else {
        panic!("expected the empty parenthesized initializer to be retained");
    };
    assert!(matches!(empty_group.value.kind, ExprKind::Error));

    let Stmt::Variable(noisy_group) = &statements[5] else {
        panic!("expected the parenthesized initializer with trailing syntax to be retained");
    };
    assert!(matches!(
        noisy_group.value.kind,
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
        ast::{Expr, ExprKind, Stmt},
        syntax::RecoveryNodeKind,
    };

    fn conditional(statement: &Stmt) -> (&Expr, &Expr, &Expr) {
        let Stmt::Variable(variable) = statement else {
            panic!("expected a retained variable declaration");
        };
        let ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } = &variable.value.kind
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

    assert_eq!(recovered.diagnostics().len(), 5);
    let statements = &recovered.syntax().actions[0].body.statements;
    assert_eq!(statements.len(), 6);

    assert!(matches!(
        conditional(&statements[0]).0.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[1]).1.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[2]).2.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[3]).2.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        conditional(&statements[4]).1.kind,
        ExprKind::Int { value: 6, .. }
    ));

    assert_eq!(
        recovered
            .recovery_nodes()
            .iter()
            .filter(|node| node.kind == RecoveryNodeKind::Missing)
            .count(),
        5
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
    assert_eq!(strict_errors.len(), 5);
}

#[test]
fn recovering_parse_preserves_declarations_and_statements_with_bad_root_expressions() {
    use splitscript::{
        ast::{ExprKind, StateSource, Stmt, SuspensionMode},
        syntax::RecoveryNodeKind,
    };

    let source = r#"
        let brokenGlobal = +
        let goodGlobal = 1
        state "game.exe" {
            brokenState = +
            goodState = 2
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

    assert_eq!(recovered.diagnostics().len(), 10);
    assert_eq!(recovered.syntax().globals.len(), 2);
    assert!(matches!(
        recovered.syntax().globals[0].value.kind,
        ExprKind::Error
    ));
    assert!(matches!(
        recovered.syntax().globals[1].value.kind,
        ExprKind::Int { value: 1, .. }
    ));

    let state = recovered.syntax().state.as_ref().unwrap();
    assert_eq!(state.fields.len(), 2);
    assert!(matches!(
        state.fields[0].source,
        StateSource::Expression(ref expression) if matches!(expression.kind, ExprKind::Error)
    ));

    let statements = &recovered.syntax().functions[0].body.statements;
    assert_eq!(statements.len(), 9);
    assert!(matches!(
        statements[0],
        Stmt::Variable(ref variable) if matches!(variable.value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[1],
        Stmt::Assign { ref value, .. } if matches!(value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[2],
        Stmt::Throw { ref error, .. } if matches!(error.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[3],
        Stmt::Suspend { mode: SuspensionMode::Await, ref value, .. }
            if matches!(value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[4],
        Stmt::Suspend { mode: SuspensionMode::Retry, ref value, .. }
            if matches!(value.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[5],
        Stmt::Expression(ref expression) if matches!(expression.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[6],
        Stmt::While { ref condition, .. } if matches!(condition.kind, ExprKind::Error)
    ));
    assert!(matches!(
        statements[7],
        Stmt::Variable(ref variable)
            if matches!(variable.value.kind, ExprKind::Match { ref value, .. }
                if matches!(value.kind, ExprKind::Error))
    ));
    assert!(matches!(statements[8], Stmt::Return { .. }));
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
        8
    );
    assert_eq!(recovered.source_document().reconstruct(), source);

    let strict_errors = splitscript::parse(source).expect_err("batch parsing remains strict");
    assert_eq!(strict_errors.len(), 10);
}

#[test]
fn abi_catalog_drives_wasm_imports_and_the_internal_reference() {
    let catalog = AbiCatalog::new();
    assert_eq!(catalog.validate(), Vec::<String>::new());
    assert_eq!(
        catalog.render_signature(AbiImportId::ProcessRead),
        "(i64, i64, i32, i32) -> i32"
    );
    let process_read = catalog.import(AbiImportId::ProcessRead);
    assert_eq!(
        process_read.parameters[0].ownership,
        AbiOwnership::BorrowedHandle
    );
    assert_eq!(
        process_read.parameters[2].ownership,
        AbiOwnership::OutputMemory
    );
    assert!(process_read.effects.contains(&AbiEffect::ReadsProcess));
    assert_eq!(
        catalog.import(AbiImportId::ProcessAttach).results[0].ownership,
        AbiOwnership::OwnedHandle
    );

    let wasm = splitscript::compile("state \"game.exe\" {}").unwrap();
    let mut emitted = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            for import in section.into_imports() {
                let import = import.unwrap();
                assert!(matches!(import.ty, TypeRef::Func(_)));
                emitted.push((import.module.to_owned(), import.name.to_owned()));
            }
        }
    }
    let required = [
        AbiImportId::TimerGetState,
        AbiImportId::ProcessAttach,
        AbiImportId::ProcessDetach,
        AbiImportId::ProcessIsOpen,
    ];
    let declared = catalog
        .imports()
        .filter(|import| required.contains(&import.id))
        .map(|import| (import.module.to_owned(), import.name.to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(emitted, declared);

    let abi_document = include_str!("../docs/ABI.md");
    let table_start = abi_document
        .find("| Import | WebAssembly type |")
        .expect("ABI reference should contain its generated import table");
    let table_end = abi_document[table_start..]
        .find("\n\n")
        .map_or(abi_document.len(), |offset| table_start + offset);
    assert_eq!(
        abi_document[table_start..table_end].trim_end(),
        catalog.render_import_table().trim_end(),
        "docs/ABI.md import table must remain a verified catalog view"
    );
}

#[test]
fn settings_host_imports_are_filtered_by_setting_kind() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            settings {
                enabled: bool = true
            }
        "#,
    )
    .expect("Boolean settings should compile");
    let mut imports = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            imports.extend(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned()),
            );
        }
    }

    assert!(imports.iter().any(|name| name == "user_settings_add_bool"));
    assert!(imports.iter().any(|name| name == "setting_value_get_bool"));
    assert!(
        !imports
            .iter()
            .any(|name| name == "setting_value_get_string")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_choice")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_file_select")
    );
}

#[derive(Default)]
struct TypedExpressionCounter(usize);

impl splitscript::hir::TypedVisitor for TypedExpressionCounter {
    fn visit_expression(
        &mut self,
        expression: &splitscript::hir::TypedExpression,
        program: &splitscript::hir::TypedProgram,
    ) {
        self.0 += 1;
        splitscript::hir::walk_typed_expression(self, expression, program);
    }
}

#[test]
fn standard_library_catalog_is_valid_documented_and_compilable() {
    let library = StandardLibrary::new();
    assert_eq!(library.validate(), Vec::<String>::new());
    assert!(library.core_type_has_capability(
        splitscript::stdlib::CoreTypeId::I32,
        splitscript::stdlib::StdlibCapabilityId::MemoryReadable,
    ));
    assert!(library.core_type_has_capability(
        splitscript::stdlib::CoreTypeId::F64,
        splitscript::stdlib::StdlibCapabilityId::Float,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::stdlib::CoreTypeId::F64,
        splitscript::stdlib::StdlibCapabilityId::StringCast,
    ));
    assert!(library.type_has_capability(
        splitscript::stdlib::StdlibTypeId::String,
        splitscript::stdlib::StdlibCapabilityId::Interpolatable,
    ));
    assert_eq!(
        library.item_by_name("Numeric.clamp").map(|item| item.id),
        Some(StdlibItemId::NumericClamp)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::NumericClamp),
        "T.clamp(minimum: T, maximum: T) -> T where T: Numeric"
    );
    assert_eq!(
        library.item_by_name("setVariable").map(|item| item.id),
        Some(StdlibItemId::SetVariable)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SetTickRate),
        "setTickRate(hz: f64) -> void"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromSeconds),
        "Duration.fromSeconds(seconds: f32) -> Duration"
    );
    for removed in [
        "timer.setVariable",
        "runtime.setTickRate",
        "Duration.saturatingSecondsF32",
    ] {
        assert!(
            library.item_by_name(removed).is_none(),
            "prototype alias `{removed}` must not remain in the catalog"
        );
    }

    for item in library.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            splitscript::compile(example.validation_source()).unwrap_or_else(|errors| {
                panic!(
                    "standard-library example `{}: {}` failed: {errors:#?}",
                    item.qualified_name, example.title
                )
            });
        }
    }
}

#[test]
fn language_catalog_is_valid_documented_and_compilable() {
    let language = LanguageCatalog::new();
    assert_eq!(language.validate(), Vec::<String>::new());
    let retry = language
        .item_by_name("retry")
        .expect("retry should be a catalog-backed language construct");
    assert_eq!(retry.id, LanguageItemId::Retry);
    assert_eq!(retry.kind, LanguageItemKind::Keyword);
    assert_eq!(retry.form, "let value = retry resultExpression");
    assert!(retry.documentation.details.contains("T!"));
    assert_eq!(
        language
            .builtin_type(splitscript::types::BuiltinType::I32)
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::types::BuiltinType::I32
        ))
    );
    assert_eq!(
        language
            .item_for_source_token("Address")
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::types::BuiltinType::Address
        ))
    );
    assert_eq!(
        language.item_for_source_token("choice").map(|item| item.id),
        Some(LanguageItemId::ChoiceSetting)
    );
    assert_eq!(
        StandardLibrary::new()
            .field(StdlibFieldId::ModuleAddress)
            .name,
        "address"
    );
    assert_eq!(
        StandardLibrary::new()
            .variant(StdlibVariantId::TimerStateRunning)
            .name,
        "Running"
    );

    for action in [
        splitscript::ast::ActionKind::OnDetached,
        splitscript::ast::ActionKind::OnAttach,
        splitscript::ast::ActionKind::WhileAttached,
        splitscript::ast::ActionKind::Start,
        splitscript::ast::ActionKind::Split,
        splitscript::ast::ActionKind::Reset,
        splitscript::ast::ActionKind::IsLoading,
        splitscript::ast::ActionKind::GameTime,
    ] {
        let item = language.action(action);
        assert_eq!(item.name, action.name());
        assert_eq!(item.kind, LanguageItemKind::Action(action));
    }

    for item in language.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            splitscript::compile(example.validation_source()).unwrap_or_else(|errors| {
                panic!(
                    "language example `{}: {}` failed: {errors:#?}",
                    item.name, example.title
                )
            });
        }
    }
}

#[test]
fn compiler_database_resolves_language_catalog_syntax() {
    use splitscript::{
        database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
        language::LanguageItemId,
        types::BuiltinType,
    };

    let source = r#"
        enum Mode {
            A
            B
        }

        settings {
            /// Select the active mode.
            "Mode" => selected: choice {
                "First" => Mode.A default
                "Second" => Mode.B
            }
            /// Select an input file.
            "Input" => input: file {
                mime => "application/octet-stream"
            }
        }

        state "game.exe" {
            level: i32 at 0x1000
        }

        fn maybe(value: i32) -> i32? {
            return Some(value)
        }

        fn fallible() -> i32! {
            return Err("unavailable")
        }

        fn propagated() -> i32! {
            return fallible()?
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            print(module.address as String)
        }

        whileAttached {
            let timerState = TimerState.Running
            let levelChanged = current.level != old.level
            let settingChanged = settings.selected != oldSettings.selected
            let value = match maybe(1) {
                Some(value) => value,
                None => 0
            }
            print(`value {value}`)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let mode = checked.syntax().enums[0].id;
    let variant = checked.syntax().enums[0].variants[0].id;

    let i32_type = source.find("value: i32").unwrap() + "value: ".len();
    assert_eq!(
        database.definition_at(i32_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::BuiltinType(
            BuiltinType::I32
        )))
    );
    let option_type = source.find("i32?").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(option_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::OptionType))
    );
    let result_type = source.find("i32!").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(result_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ResultType))
    );
    let propagation = source.find("fallible()?").unwrap() + "fallible()".len();
    assert_eq!(
        database.definition_at(propagation).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Propagate))
    );

    let module_field = source.find("module.address").unwrap() + "module.".len();
    assert_eq!(
        database.definition_at(module_field).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::stdlib::StdlibSymbolId::Field(StdlibFieldId::ModuleAddress)
        ))
    );
    let timer_state = source.find("TimerState.Running").unwrap();
    assert_eq!(
        database.definition_at(timer_state).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::stdlib::StdlibSymbolId::Type(StdlibTypeId::TimerState)
        ))
    );
    assert_eq!(
        database
            .definition_at(timer_state + "TimerState.".len())
            .unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::stdlib::StdlibSymbolId::Variant(StdlibVariantId::TimerStateRunning)
        ))
    );

    for (root, expected) in [
        ("current.level", LanguageItemId::CurrentSnapshot),
        ("old.level", LanguageItemId::OldSnapshot),
        ("settings.selected", LanguageItemId::Settings),
        ("oldSettings.selected", LanguageItemId::OldSettingsSnapshot),
    ] {
        let offset = source.find(root).unwrap();
        assert_eq!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Language(expected)),
            "wrong snapshot catalog target for `{root}`"
        );
    }

    for (spelling, expected) in [
        ("Some(value)", LanguageItemId::SomeConstructor),
        ("Err(\"unavailable\")", LanguageItemId::ErrorConstructor),
        ("None =>", LanguageItemId::NoneConstructor),
        ("choice {", LanguageItemId::ChoiceSetting),
        ("default", LanguageItemId::ChoiceSetting),
        ("file {", LanguageItemId::FileSetting),
        ("mime =>", LanguageItemId::FileSetting),
        ("whileAttached", LanguageItemId::WhileAttached),
    ] {
        let offset = source.find(spelling).unwrap();
        assert_eq!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Language(expected)),
            "wrong catalog target for `{spelling}`"
        );
    }

    let doc_comment = source.find("/// Select").unwrap();
    assert_eq!(
        database.definition_at(doc_comment).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::SettingDocumentation
        ))
    );
    let template = source.find('`').unwrap();
    assert_eq!(
        database.definition_at(template).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::TemplateString))
    );

    let choice = source.find("Mode.A default").unwrap();
    assert!(matches!(
        database.definition_at(choice).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(mode)
    ));
    assert!(matches!(
        database.definition_at(choice + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(variant)
    ));
}

#[test]
fn checked_program_exposes_resolved_standard_library_calls_without_codegen() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let value: i32 = 10
            let bounded = value.clamp(0, 7)
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let calls = checked.semantics().calls().collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let ResolvedCall::StandardLibrary {
        item,
        type_arguments,
        ..
    } = calls[0].1
    else {
        panic!("the call should resolve to the standard library");
    };
    assert_eq!(*item, StdlibItemId::NumericClamp);
    assert_eq!(type_arguments.len(), 1);
    assert_eq!(
        checked.semantics().types().kind(type_arguments[0]),
        &TypeKind::Builtin(BuiltinType::I32)
    );

    let action = checked
        .syntax()
        .actions
        .first()
        .expect("whileAttached action");
    let splitscript::ast::Stmt::Variable(bounded) = &action.body.statements[1] else {
        panic!("the second statement should declare the bounded value");
    };
    assert_eq!(calls[0].0, bounded.value.id);
    let result_type = checked
        .semantics()
        .expression_type(bounded.value.id)
        .expect("every checked expression has a semantic type");
    assert_eq!(
        checked.semantics().types().kind(result_type),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn expression_ids_distinguish_nested_nodes_and_expose_their_types() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let sum: i32 = 1 + 2
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let splitscript::ast::Stmt::Variable(sum) = &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the sum variable");
    };
    let splitscript::ast::ExprKind::Binary { left, right, .. } = &sum.value.kind else {
        panic!("expected a binary expression");
    };
    assert_ne!(sum.value.id, left.id);
    assert_ne!(sum.value.id, right.id);
    assert_ne!(left.id, right.id);
    for expression in [&sum.value, left.as_ref(), right.as_ref()] {
        let ty = checked
            .semantics()
            .expression_type(expression.id)
            .expect("the expression should have a semantic type");
        assert_eq!(
            checked.semantics().types().kind(ty),
            &TypeKind::Builtin(BuiltinType::I32)
        );
    }
}

#[test]
fn inferred_declaration_types_are_semantic_and_syntax_annotations_stay_optional() {
    let source = r#"
        let seed = 7

        state "game.exe" {
            score = seed
        }

        fn identity(value) {
            return value
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let copy = identity(seed)
            let address = module.address
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();

    let assert_builtin = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Builtin(expected));
    };
    let assert_standard = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Standard(expected));
    };

    let global = &syntax.globals[0];
    assert_eq!(global.annotation, None);
    assert_builtin(global.id, BuiltinType::I32);

    let state_field = &syntax.state.as_ref().unwrap().fields[0];
    assert_eq!(state_field.annotation, None);
    assert_builtin(state_field.id, BuiltinType::I32);

    let function = &syntax.functions[0];
    assert_eq!(function.params[0].annotation, None);
    assert_eq!(function.return_annotation, None);
    assert_builtin(function.params[0].id, BuiltinType::I32);
    let result = semantics
        .function_result(function.id)
        .expect("every function should have a semantic result type");
    assert_eq!(
        semantics.types().kind(result),
        &TypeKind::Builtin(BuiltinType::I32)
    );

    let statements = &syntax.actions[0].body.statements;
    let splitscript::ast::Stmt::Suspend {
        binding: Some(module),
        ..
    } = &statements[0]
    else {
        panic!("expected an awaited module binding");
    };
    assert_eq!(module.annotation, None);
    assert_standard(module.id, StdlibTypeId::Module);

    let splitscript::ast::Stmt::Variable(copy) = &statements[1] else {
        panic!("expected the inferred function-call binding");
    };
    assert_eq!(copy.annotation, None);
    assert_builtin(copy.id, BuiltinType::I32);

    let splitscript::ast::Stmt::Variable(address) = &statements[2] else {
        panic!("expected the inferred member-path binding");
    };
    assert_eq!(address.annotation, None);
    assert_builtin(address.id, BuiltinType::Address);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration types should drive valid Wasm lowering");
}

#[test]
fn parsed_type_references_are_inference_free_syntax() {
    let source = r#"
        state "game.exe" {
            level: u16 at "game.exe", 0x10
        }

        record Buffer {
            values: Array<u32>
        }

        fn widen(value: i32) -> u64 {
            return value as u64
        }

        whileAttached {
            let count: i64 = 1i64
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    {
        use splitscript::ast::TypeRef;

        let syntax = parsed.syntax();
        assert_eq!(
            syntax.state.as_ref().unwrap().fields[0].annotation,
            Some(TypeRef::U16)
        );

        let values = &syntax.records[0].fields[0];
        let TypeRef::Array(array) = values.ty else {
            panic!("the record field should retain its parsed array reference");
        };
        assert_eq!(
            syntax
                .array_types
                .iter()
                .find(|declaration| declaration.id == array)
                .unwrap()
                .element,
            TypeRef::U32
        );

        let function = &syntax.functions[0];
        assert_eq!(function.params[0].annotation, Some(TypeRef::I32));
        assert_eq!(function.return_annotation, Some(TypeRef::U64));
        let splitscript::ast::Stmt::Return {
            value: Some(cast), ..
        } = &function.body.statements[0]
        else {
            panic!("expected the cast return expression");
        };
        let splitscript::ast::ExprKind::Cast { target, .. } = &cast.kind else {
            panic!("expected a parsed cast");
        };
        assert_eq!(*target, TypeRef::U64);

        let splitscript::ast::Stmt::Variable(count) = &syntax.actions[0].body.statements[0] else {
            panic!("expected the annotated local");
        };
        assert_eq!(count.annotation, Some(TypeRef::I64));
        let splitscript::ast::ExprKind::Int { suffix, .. } = &count.value.kind else {
            panic!("expected the suffixed integer literal");
        };
        assert_eq!(*suffix, Some(TypeRef::I64));
    }

    let checked = splitscript::check(parsed).unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("syntax type references should adapt to semantic types and valid Wasm");
}

#[test]
fn source_standard_type_names_resolve_after_parsing() {
    use splitscript::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            fn base(module: Module) -> address { return module.address }
        "#,
    )
    .unwrap();
    let function = &parsed.syntax().functions[0];
    let Some(TypeRef::Named(name)) = function.params[0].annotation else {
        panic!("source nominal annotations should retain a name identity");
    };
    assert_eq!(parsed.syntax().type_name(name), "Module");

    let checked = splitscript::check(parsed).unwrap();
    let parameter_type = checked
        .semantics()
        .value_type(checked.syntax().functions[0].params[0].id)
        .unwrap();
    assert_eq!(
        parameter_type,
        checked
            .semantics()
            .types()
            .id_for_standard(StdlibTypeId::Module),
        "name resolution, inference, and published semantics must preserve one standard TypeId",
    );
    assert_eq!(
        checked.semantics().types().kind(parameter_type),
        &TypeKind::Standard(StdlibTypeId::Module)
    );
}

#[test]
fn source_record_and_enum_annotations_resolve_after_parsing() {
    use splitscript::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            record Point {
                x: i32
            }
            enum Location {
                Known(Point)
                Unknown
            }
            fn identity(point: Point) -> Point { return point }
            fn keep(location: Location) -> Location { return location }
        "#,
    )
    .unwrap();
    for (function, expected_name) in parsed.syntax().functions.iter().zip(["Point", "Location"]) {
        let Some(TypeRef::Named(parameter_name)) = function.params[0].annotation else {
            panic!("source nominal parameters should retain name identities");
        };
        let Some(TypeRef::Named(result_name)) = function.return_annotation else {
            panic!("source nominal results should retain name identities");
        };
        assert_eq!(parameter_name, result_name);
        assert_eq!(parsed.syntax().type_name(parameter_name), expected_name);
    }

    let checked = splitscript::check(parsed).unwrap();
    let point_parameter = checked.syntax().functions[0].params[0].id;
    let location_parameter = checked.syntax().functions[1].params[0].id;
    assert_eq!(
        checked.semantics().value_type(point_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_record(checked.syntax().records[0].id),
    );
    assert_eq!(
        checked.semantics().value_type(location_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_enum(checked.syntax().enums[0].id),
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(point_parameter).unwrap()),
        &TypeKind::Record(checked.syntax().records[0].id)
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(location_parameter).unwrap()),
        &TypeKind::Enum(checked.syntax().enums[0].id)
    );
}

#[test]
fn semantic_capabilities_query_declared_and_derived_types_by_type_id() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
            state "game.exe" {}
            record Pair {
                left: i32
                right: i32
            }
            enum MaybePair {
                Pair(Pair)
                Empty
            }
            fn keep(value: MaybePair) -> MaybePair { return value }
        "#,
        )
        .expect("the capability fixture should parse"),
    )
    .expect("the capability fixture should type-check");
    let types = checked.semantics().types();
    let pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Record(record) if *record == checked.syntax().records[0].id)
                .then_some(id)
        })
        .expect("the source record should have a semantic type");
    let maybe_pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Enum(enumeration) if *enumeration == checked.syntax().enums[0].id)
                .then_some(id)
        })
        .expect("the source enum should have a semantic type");
    let string = types.id_for_standard(StdlibTypeId::String);
    let capabilities = checked.capabilities();

    assert!(capabilities.has(
        pair,
        splitscript::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        pair,
        splitscript::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        maybe_pair,
        splitscript::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(!capabilities.has(
        maybe_pair,
        splitscript::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        string,
        splitscript::stdlib::StdlibCapabilityId::Interpolatable,
        checked.semantics(),
    ));
}

#[test]
fn semantic_type_ids_intern_constructed_generic_arguments() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let matrix = [[1i32], [2i32]]
            let row = matrix.get(0)
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let (_, call) = checked
        .semantics()
        .calls()
        .find(|(_, call)| {
            matches!(
                call,
                ResolvedCall::StandardLibrary {
                    item: StdlibItemId::ArrayGet,
                    ..
                }
            )
        })
        .expect("the array get should be resolved");
    let ResolvedCall::StandardLibrary { type_arguments, .. } = call else {
        panic!("array.get should resolve to the standard library");
    };
    let TypeKind::Array { element, .. } = checked.semantics().types().kind(type_arguments[0])
    else {
        panic!("the generic argument should be an interned array type");
    };
    assert_eq!(
        checked.semantics().types().kind(*element),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn option_and_result_annotations_are_distinct_interned_semantic_types() {
    let source = r#"
        state "game.exe" {}

        record Wrappers {
            maybe: i32?
            attempt: String!
        }

        fn inspect(maybe: i32?, attempt: String!) {}
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let function = &syntax.functions[0];

    let maybe = semantics.value_type(function.params[0].id).unwrap();
    let TypeKind::Option {
        layout: maybe_layout,
        value: maybe_value,
    } = semantics.types().kind(maybe)
    else {
        panic!("`i32?` should remain a first-class option type");
    };
    assert_eq!(
        semantics.types().kind(*maybe_value),
        &TypeKind::Builtin(BuiltinType::I32)
    );
    let splitscript::ast::TypeRef::Option(parsed_maybe_layout) =
        function.params[0].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its option layout");
    };
    assert_eq!(*maybe_layout, parsed_maybe_layout);

    let attempt = semantics.value_type(function.params[1].id).unwrap();
    let TypeKind::Result {
        layout: attempt_layout,
        value: attempt_value,
    } = semantics.types().kind(attempt)
    else {
        panic!("`String!` should remain a first-class result type");
    };
    assert_eq!(
        semantics.types().kind(*attempt_value),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    let splitscript::ast::TypeRef::Result(parsed_attempt_layout) =
        function.params[1].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its result layout");
    };
    assert_eq!(*attempt_layout, parsed_attempt_layout);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option and result annotations should have valid Wasm GC layouts");
}

#[test]
fn option_and_result_equality_is_structural_and_payload_checked() {
    let source = include_str!("wrapper_equality.split");
    let wasm = splitscript::compile(source).expect("wrapper equality should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("Option and Result equality helpers should produce valid Wasm GC");

    let invalid = r#"
        state "game.exe" {}

        fn same(left: Array<i32>?, right: Array<i32>?) -> bool {
            return left == right
        }

        whileAttached {
            let value = same(None, None)
        }
    "#;
    let diagnostics = splitscript::compile(invalid)
        .expect_err("wrappers should require equality on their contained values");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("optional value does not support equality")
    }));
}

#[test]
fn option_and_result_matches_bind_values_and_require_exhaustiveness() {
    let source = include_str!("wrapper_match.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("wrapper matches should type-check");
    assert!(
        checked
            .typed_hir()
            .patterns()
            .any(|pattern| pattern.wrapper.is_some())
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("wrapper match lowering should produce valid Wasm GC");

    for (source, missing) in [
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32?) -> i32 {
                    return match value { Some(present) => present }
                }
            "#,
            "missing `None`",
        ),
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32!) -> i32 {
                    return match value { Ok(success) => success }
                }
            "#,
            "missing `Err(error)`",
        ),
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("wrapper matches without both states must be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(missing)),
            "expected `{missing}` in {diagnostics:?}"
        );
    }

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            fn unwrap(value: i32?) -> i32 {
                return match value {
                    present => present,
                    None => 0
                }
            }
        "#,
    )
    .expect_err("bare wrapper bindings should be rejected as misleading");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("use `Some(present)` or `Ok(present)`")
    }));
}

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
    use splitscript::{FixApplicability, ast::Stmt};

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

#[test]
fn option_and_result_values_use_explicit_typed_hir_conversions() {
    use splitscript::semantic::{ResolvedCall, ValueConversionKind};

    let source = r#"
        state "game.exe" {}

        fn maybe(flag: bool) -> i32? {
            if flag { return 7 }
            return None
        }

        fn attempt(flag: bool) -> i32! {
            if flag { return 9 }
            return Err("attempt failed")
        }

        whileAttached {
            let optional: i32? = 5
            let empty: i32? = None
            let successful: i32! = 11
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();

    let mut saw_option_lift = false;
    let mut saw_result_lift = false;
    let mut saw_optional_null = false;
    let mut error_constructors = 0;
    for expression in checked.typed_hir().expressions() {
        if let Some(conversion) = expression.conversion {
            match conversion.kind {
                ValueConversionKind::LiftOption => saw_option_lift = true,
                ValueConversionKind::LiftResult => saw_result_lift = true,
            }
            assert_ne!(conversion.source, conversion.target);
        }
        if matches!(expression.kind, splitscript::hir::TypedExpressionKind::None)
            && matches!(
                checked.semantics().types().kind(expression.ty),
                TypeKind::Option { .. }
            )
        {
            saw_optional_null = true;
        }
        if matches!(
            checked.typed_hir().call(expression.id),
            Some(ResolvedCall::ResultError { .. })
        ) {
            error_constructors += 1;
        }
    }
    assert!(saw_option_lift);
    assert!(saw_result_lift);
    assert!(saw_optional_null);
    assert_eq!(error_constructors, 2);

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        assert_eq!(
            lowered
                .expression(expression.id)
                .expect("every typed expression should have a Wasm IR plan")
                .conversion,
            expression.conversion,
            "wrapper conversion edges must be copied into Wasm IR"
        );
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option/result constructors and lifts should produce valid Wasm GC");
}

#[test]
fn wasm_ir_owns_scalar_expression_operations_and_resolved_paths() {
    use splitscript::wasm_ir::ExpressionKind;

    let source = r#"
        state "game.exe" {}

        fn calculate(input: i32) {
            let negated = -(input + 2)
            let text = negated as String
            if !false && negated != 0 {
                print(text)
            }
        }

        whileAttached {
            calculate(4)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw_path = false;
    let mut saw_unary = false;
    let mut saw_binary = false;
    let mut saw_cast = false;
    for expression in lowered.expressions() {
        match &expression.kind {
            ExpressionKind::Path { root, .. } => {
                assert!(root.is_some());
                saw_path = true;
            }
            ExpressionKind::Unary { .. } => saw_unary = true,
            ExpressionKind::Binary { .. } => saw_binary = true,
            ExpressionKind::Cast { .. } => saw_cast = true,
            _ => {}
        }
    }
    assert!(saw_path && saw_unary && saw_binary && saw_cast);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR scalar expression lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_gc_constructors_interpolation_and_signatures() {
    use splitscript::wasm_ir::{ExpressionKind, InterpolatedPart};

    let source = r#"
        state "game.exe" {}

        record Pair {
            left: i32
            right: i32
        }

        enum Event {
            Empty
            Value(i32)
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let marker = await module.scan(sig"48 8B ?? B?")
            print(marker as String)
        }

        whileAttached {
            let values = [1, 2, 3]
            let pair = Pair { right: values.get(1), left: values.get(0) }
            let event = Event.Value(pair.left)
            print(`pair {pair.left}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 6];
    for expression in lowered.expressions() {
        match &expression.kind {
            ExpressionKind::String(_) => saw[0] = true,
            ExpressionKind::InterpolatedString(parts) => {
                assert!(parts.iter().any(|part| matches!(
                    part,
                    InterpolatedPart::Expression {
                        string_conversion_source: Some(_),
                        ..
                    }
                )));
                saw[1] = true;
            }
            ExpressionKind::Signature(_) => saw[2] = true,
            ExpressionKind::Array(elements) => {
                assert_eq!(elements.len(), 3);
                saw[3] = true;
            }
            ExpressionKind::Record { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert_ne!(fields[0].0, fields[1].0);
                saw[4] = true;
            }
            ExpressionKind::Enum { payload, .. } if payload.is_some() => saw[5] = true,
            _ => {}
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR GC constructor lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_resolved_call_targets_and_arguments() {
    use splitscript::{hir::TypedExpressionKind, wasm_ir::ExpressionKind};

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
            let bounded = direct.min(method)
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 4];

    for expression in checked.typed_hir().expressions() {
        let Some(expected_target) = checked.typed_hir().call(expression.id) else {
            continue;
        };
        let ExpressionKind::Call { target, arguments } = &lowered
            .expression(expression.id)
            .expect("every checked call should have a Wasm IR plan")
            .kind
        else {
            panic!("resolved calls must not remain deferred to typed HIR")
        };
        assert_eq!(target, expected_target);
        let TypedExpressionKind::Call {
            arguments: expected_arguments,
            ..
        } = &expression.kind
        else {
            unreachable!()
        };
        assert_eq!(arguments, expected_arguments);
        match target {
            ResolvedCall::UserFunction { .. } => saw[0] = true,
            ResolvedCall::UserMethod { .. } => saw[1] = true,
            ResolvedCall::StandardLibrary { .. } => saw[2] = true,
            ResolvedCall::ResultError { .. } => saw[3] = true,
            ResolvedCall::OptionSome { .. } | ResolvedCall::ResultSuccess { .. } => {}
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR call lowering should preserve valid codegen");
}

#[test]
fn context_free_null_and_err_request_wrapper_annotations() {
    for (initializer, expected_message) in [
        ("None", "add a `T?` annotation"),
        ("Err(\"failed\")", "add a `T!` annotation"),
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                whileAttached {{ let value = {initializer} }}
            "#
        );
        let errors = splitscript::check(splitscript::parse(&source).unwrap())
            .expect_err("a wrapper constructor needs its contained type from context");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains(expected_message)),
            "missing focused diagnostic in {errors:#?}"
        );
    }
}

#[test]
fn else_unwraps_options_and_results_with_value_or_return_fallbacks() {
    use splitscript::{
        hir::{TypedExpressionKind, TypedFallbackBranch},
        wasm_ir::{ExpressionKind, FallbackBranch, LocalPurpose},
    };

    let source = r#"
        state "game.exe" {}

        fn choose(value: i32?) -> i32 {
            return value else 41
        }

        fn propagate(value: i32!) -> i32! {
            let unwrapped = value else return Err("propagated")
            return unwrapped + 1
        }

        fn nested(optional: i32?, result: i32!) -> i32 {
            return optional else result else 7
        }

        fn observe(value: i32?) {
            let unwrapped = value else return
            print(unwrapped as String)
        }

        whileAttached {
            let empty = choose(None)
            let present = choose(3)
            let failed = propagate(Err("failed"))
            let successful = propagate(5)
            observe(None)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let fallback_count = checked
        .typed_hir()
        .expressions()
        .filter(|expression| matches!(expression.kind, TypedExpressionKind::Fallback { .. }))
        .count();
    assert_eq!(fallback_count, 5);

    let lowered = splitscript::lower_wasm(&checked);
    let mut branches = [false; 3];
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Fallback { value, fallback } = &expression.kind else {
            continue;
        };
        let ExpressionKind::Fallback {
            value: lowered_value,
            fallback: lowered_fallback,
        } = &lowered
            .expression(expression.id)
            .expect("fallback expression should have a Wasm IR plan")
            .kind
        else {
            panic!("fallback expressions must not remain deferred to typed HIR")
        };
        assert_eq!(lowered_value, value);
        match (fallback, lowered_fallback) {
            (TypedFallbackBranch::Value(expected), FallbackBranch::Value(actual)) => {
                assert_eq!(actual, expected);
                branches[0] = true;
            }
            (TypedFallbackBranch::Return(Some(expected)), FallbackBranch::Return(Some(actual))) => {
                assert_eq!(actual, expected);
                branches[1] = true;
            }
            (TypedFallbackBranch::Return(None), FallbackBranch::Return(None)) => {
                branches[2] = true;
            }
            _ => panic!("Wasm IR must preserve the resolved fallback branch"),
        }
    }
    assert!(branches.into_iter().all(|branch| branch));
    let planned_fallbacks = lowered
        .bodies()
        .flat_map(|body| &body.locals)
        .filter(|local| matches!(local.purpose, LocalPurpose::FallbackValue(_)))
        .count();
    assert_eq!(planned_fallbacks, fallback_count);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("value and returning fallbacks should produce valid Wasm control flow");
}

#[test]
fn question_mark_propagates_to_function_and_state_field_boundaries() {
    use splitscript::hir::TypedExpressionKind;

    let source = r#"
        state "game.exe" {
            selected = if readMemory {
                process.read.u16(0x1000)?
            } else {
                7
            }
        }

        let readMemory = true

        fn increment(value: i32!) -> i32! {
            return value? + 1
        }

        fn rejectNegative(value: i32) -> i32! {
            if value < 0 {
                throw "negative values are not supported"
            }
            return value
        }

        whileAttached {
            let incremented = increment(3) else 0
            let rejected = rejectNegative(-1) else 0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let propagation = checked
        .typed_hir()
        .expressions()
        .filter_map(|expression| match expression.kind {
            TypedExpressionKind::Propagate { target, .. } => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(propagation.len(), 2);
    assert!(propagation.into_iter().all(|target| matches!(
        checked.semantics().types().kind(target),
        TypeKind::Result { .. }
    )));

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Propagate { value, target } = &expression.kind else {
            continue;
        };
        let splitscript::wasm_ir::ExpressionKind::Propagate {
            value: lowered_value,
            target: lowered_target,
        } = &lowered
            .expression(expression.id)
            .expect("propagation expression should have a Wasm IR plan")
            .kind
        else {
            panic!("postfix propagation must not remain deferred to typed HIR")
        };
        assert_eq!((lowered_value, lowered_target), (value, target));
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("question-mark propagation should produce valid Wasm GC control flow");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let failed: i32! = Err("failed")
            let value = failed?
        }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid).unwrap())
        .expect_err("actions are not implicit result boundaries");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("state-field boundary or a function returning `T!`")
    }));

    let invalid_throw = r#"
        state "game.exe" {}
        whileAttached { throw "actions do not return results" }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid_throw).unwrap())
        .expect_err("throw requires an enclosing failure boundary");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("function returning `T!` or an explicit catch boundary")
    }));
}

#[test]
fn else_rejects_values_that_are_not_option_or_result() {
    let source = r#"
        state "game.exe" {}
        whileAttached { let value = 1 else 2 }
    "#;
    let errors = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("plain values cannot be unwrapped");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("`else` can only unwrap `T?` or `T!`")
    }));
}

#[test]
fn declared_record_enum_and_array_layouts_are_semantic_facts() {
    let source = r#"
        state "game.exe" {}

        record Inventory {
            names: Array<String>
            code: u16
        }

        enum Lookup {
            Missing
            Found(Inventory)
        }

        whileAttached {
            let lookup = Lookup.Found(Inventory {
                names: ["Moon"],
                code: 7
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let inventory = &syntax.records[0];

    let names_type = semantics
        .record_field_type(inventory.fields[0].id)
        .expect("record field layouts should expose semantic types");
    let TypeKind::Array {
        layout,
        element: names_element,
    } = semantics.types().kind(names_type)
    else {
        panic!("the names field should have a constructed array type");
    };
    assert_eq!(
        semantics.types().kind(*names_element),
        &TypeKind::Standard(StdlibTypeId::String)
    );

    let splitscript::ast::TypeRef::Array(names_array) = inventory.fields[0].ty else {
        panic!("the source annotation should reference its array layout");
    };
    assert_eq!(*layout, names_array);
    assert_eq!(
        semantics.array_element_type(names_array),
        Some(*names_element)
    );

    let code_type = semantics.record_field_type(inventory.fields[1].id).unwrap();
    assert_eq!(
        semantics.types().kind(code_type),
        &TypeKind::Builtin(BuiltinType::U16)
    );

    let enumeration = &syntax.enums[0];
    assert!(
        semantics
            .enum_variant_payloads()
            .any(|(variant, payload)| variant == enumeration.variants[0].id && payload.is_none())
    );
    let found_payload = semantics
        .enum_variant_payload(enumeration.variants[1].id)
        .expect("payload variants should expose their semantic payload type");
    assert_eq!(
        semantics.types().kind(found_payload),
        &TypeKind::Record(inventory.id)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration layouts should drive valid Wasm GC types");
}

#[test]
fn catalog_queries_expose_typed_paths_effects_and_docs_for_editor_tooling() {
    let library = StandardLibrary::new();
    let process_namespace = library
        .namespace_by_name("process")
        .expect("process should be an explicit namespace declaration");
    assert_eq!(
        process_namespace.id,
        splitscript::stdlib::StdlibNamespaceId::Process
    );
    assert!(
        process_namespace
            .documentation
            .summary
            .contains("attached game process")
    );
    let unity_image = library
        .type_by_name("UnityImage")
        .expect("UnityImage should be a nominal library declaration");
    assert_eq!(
        unity_image.id,
        splitscript::stdlib::StdlibTypeId::UnityImage
    );
    assert_eq!(
        library
            .public_field(unity_image.id, "address")
            .expect("UnityImage.address should be declared")
            .ty,
        splitscript::stdlib::DeclaredTypeRef::Core(splitscript::stdlib::CoreTypeId::Address)
    );
    assert!(
        library.public_field(unity_image.id, "module").is_none(),
        "runtime ownership slots must not leak into the public member surface"
    );
    let read_path = ["process", "read", "u16"].map(str::to_owned);
    let read = library
        .resolve_path(&read_path)
        .expect("typed process reads should resolve through the catalog");
    assert_eq!(read.item.id, StdlibItemId::ProcessRead);
    assert_eq!(read.type_arguments, [("T", BuiltinType::U16)]);
    let get = library.method_candidates("get");
    assert_eq!(get.len(), 1);
    assert_eq!(get[0].item.id, StdlibItemId::ArrayGet);
    assert_eq!(
        get[0].receiver(),
        Some(splitscript::stdlib::TypeRef::Array(
            &splitscript::stdlib::TypeRef::Variable("T")
        ))
    );
    let min = library.method_candidates("min");
    assert_eq!(min.len(), 1);
    assert_eq!(min[0].item.id, StdlibItemId::NumericMin);
    assert_eq!(min[0].item.signature.type_parameters[0].name, "T");
    assert_eq!(
        min[0].item.signature.type_parameters[0].constraints,
        [splitscript::stdlib::TypeConstraint::Numeric]
    );
    assert!(library.method_candidates("missing").is_empty());
    assert_eq!(
        library.render_signature(read.item.id),
        "process.read(address: address) -> T! where T: MemoryReadable"
    );
    let managed_string_path = ["process", "read", "managedString"].map(str::to_owned);
    let managed_string = library
        .resolve_path(&managed_string_path)
        .expect("specialized readers should share the process.read namespace");
    assert_eq!(
        managed_string.item.id,
        StdlibItemId::ProcessReadManagedString
    );
    assert!(
        library
            .resolve_path(&["process", "readManagedString"].map(str::to_owned))
            .is_none(),
        "the inconsistent legacy path should not remain in the catalog"
    );
    assert_eq!(
        library.render_signature(managed_string.item.id),
        "process.read.managedString(address: address, maxUtf16Units: u32) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerState),
        "timer.state() -> TimerState"
    );
    let next_tick = library
        .item_by_name("nextTick")
        .expect("nextTick should be catalog-backed");
    assert_eq!(library.render_signature(next_tick.id), "nextTick() -> void");
    assert_eq!(
        next_tick.operation_semantics().suspension,
        SuspensionKind::Suspends
    );
    assert_eq!(
        next_tick.operation_semantics().cancellation,
        CancellationKind::ProcessClose
    );

    let field_any = library
        .item_by_name("UnityClass.fieldAny")
        .expect("UnityClass.fieldAny should have a documented catalog entry");
    assert_eq!(field_any.availability, Availability::OnAttach);
    assert!(field_any.effects.contains(&Effect::Suspends));
    assert!(field_any.effects.contains(&Effect::RequiresAttachedProcess));
    assert!(field_any.effects.contains(&Effect::CancelsOnProcessClose));
    let operation = field_any.operation_semantics();
    assert_eq!(operation.availability, Availability::OnAttach);
    assert_eq!(operation.suspension, SuspensionKind::Suspends);
    assert!(operation.requires_attached_process);
    assert_eq!(operation.cancellation, CancellationKind::ProcessClose);
    assert_eq!(
        library.render_operation_semantics(field_any.id),
        "available in onAttach; suspends; requires an attached process; cancels when the process closes"
    );
    assert_eq!(
        read.item.operation_semantics().suspension,
        SuspensionKind::None
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessFollow),
        "process.follow(base: address, offsets: [u64]) -> address!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessReadRelative32),
        "process.readRelative32(address: address) -> address!"
    );
    assert!(!field_any.documentation.summary.is_empty());
    assert_eq!(
        library.render_signature(field_any.id),
        "UnityClass.fieldAny(names: [String]) -> UnityField"
    );
}

#[test]
fn process_operations_reject_detached_lifecycle_use() {
    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetached {
                let value = process.read.i32(0x1000) else 0
                print(value as String)
            }
        "#,
    )
    .expect_err("process access should not be available before attachment");
    assert!(errors.iter().any(|error| {
        error.message
            == "`process.read` requires an attached process and is unavailable in `onDetached`"
    }));
}

#[test]
fn call_result_fields_parse_before_detached_effects_are_checked() {
    let source = r#"
        state "game.exe" {}

        record LevelTimeParts {
            minutes: f32
            seconds: f32
            hundredths: f32
        }

        fn baz() {
            return process.read(0x200) else process.read(0x100) else LevelTimeParts {
                minutes: 0.0,
                seconds: 0.0,
                hundredths: 0.0
            }
        }

        onDetached {
            let minutes = baz().minutes
        }
    "#;

    splitscript::parse(source).expect("a field on a call result should parse");
    let attached = source.replace("onDetached", "whileAttached");
    let wasm = splitscript::compile(&attached)
        .expect("a call-result field should type-check and lower while attached");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("a call-result field should produce valid Wasm");
    let diagnostics = splitscript::compile(source)
        .expect_err("the process-dependent helper should still be rejected while detached");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`baz` requires an attached process and is unavailable in `onDetached`"
        }),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != splitscript::DiagnosticCode::Syntax)
    );
}

#[test]
fn immediate_process_failures_are_results_and_not_awaitable_intrinsics() {
    let source = include_str!("fallible_process_operations.split");
    let wasm = splitscript::compile(source).expect("fallible process operations should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("process failure sentinels should lower to valid Result values");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let value = await process.read.i32(0x1000)
            }
        "#,
    )
    .expect_err("immediate Result operations should use retry rather than await");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "this operation is not awaitable")
    );
}

#[test]
fn attached_process_requirements_propagate_through_function_call_graphs() {
    let safe_source = r#"
        state "game.exe" {}

        record Reader {
            address: address
        }

        fn Reader.readValue() {
            return process.read.i32(self.address) else 0
        }

        fn relay(reader: Reader) {
            return reader.readValue()
        }

        fn recursiveRelay(reader: Reader, recurse) {
            if recurse {
                return recursiveRelay(reader, false)
            }
            return relay(reader)
        }

        whileAttached {
            let reader = Reader { address: 0x1000 }
            print(recursiveRelay(reader, true) as String)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(safe_source).unwrap()))
        .expect("process-dependent helpers should be callable while attached");
    for name in ["readValue", "relay", "recursiveRelay"] {
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("test helper should exist");
        assert!(
            checked
                .effects()
                .function(function.id)
                .requires_attached_process,
            "{name} should inherit its process requirement"
        );
        let effects = checked.effects().function(function.id).effects;
        assert!(
            effects.contains(&Effect::ReadsProcess),
            "{name} should inherit its process-read effect"
        );
        assert!(effects.contains(&Effect::RequiresAttachedProcess));
    }

    let detached_source = safe_source.replace("whileAttached", "onDetached");
    let errors = splitscript::compile(&detached_source)
        .expect_err("a transitive process dependency should be rejected while detached");
    assert!(errors.iter().any(|error| {
        error.message
            == "`recursiveRelay` requires an attached process and is unavailable in `onDetached`"
    }));
}

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
        splitscript::hir::DeclarationId::Function(_)
    ));
    let identity_id = identity.id;
    assert!(
        lowered
            .hir()
            .declarations_named("whileAttached")
            .any(|declaration| {
                declaration.id
                    == splitscript::hir::DeclarationId::Action(
                        splitscript::ast::ActionKind::WhileAttached,
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
        Some(splitscript::hir::ExpressionResolution::Call(_))
    )));
    let action_body = checked
        .typed_hir()
        .action_body(splitscript::ast::ActionKind::WhileAttached)
        .expect("typed HIR should own action statement shape");
    let splitscript::hir::TypedStatementKind::Variable { initializer, .. } =
        &action_body.statements[0].kind
    else {
        panic!("expected the inferred variable in typed HIR");
    };
    assert!(matches!(
        &checked.typed_hir().expression(*initializer).unwrap().kind,
        splitscript::hir::TypedExpressionKind::Array(_)
    ));
    let interpolation = checked
        .typed_hir()
        .expressions()
        .find_map(|expression| match &expression.kind {
            splitscript::hir::TypedExpressionKind::InterpolatedString(parts) => Some(parts),
            _ => None,
        })
        .expect("typed HIR should retain the interpolated string");
    assert!(matches!(
        interpolation.as_slice(),
        [splitscript::hir::TypedInterpolatedPart::Expression {
            conversion: Some(splitscript::hir::ImplicitConversion::ToString { source }),
            ..
        }] if checked.semantics().types().kind(*source)
            == &TypeKind::Builtin(BuiltinType::U16)
    ));
    let mut counter = TypedExpressionCounter::default();
    splitscript::hir::TypedVisitor::visit_program(&mut counter, checked.typed_hir());
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

    let source = include_str!("debug_profile.split");
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
                .body(splitscript::wasm_ir::BodyOwner::Function(function.id))
                .is_some()
        );
        assert!(
            release_lowering
                .body(splitscript::wasm_ir::BodyOwner::Function(function.id))
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
        .find(|setting| matches!(setting.kind, splitscript::ast::SettingKind::Choice { .. }))
        .expect("settings example has a choice");
    let splitscript::ast::SettingKind::Choice {
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
        .find(|item| item.id == *enumeration)
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
                Selected.Number(process.read.u16(0x1234 as address) else 0)
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
        let splitscript::hir::TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } = &expression.kind
        else {
            continue;
        };
        let splitscript::wasm_ir::ExpressionKind::If {
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
    let source = include_str!("while_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("while loops should typecheck");
    let while_attached = checked
        .typed_hir()
        .action_body(splitscript::ast::ActionKind::WhileAttached)
        .expect("the fixture has a whileAttached action");
    assert!(while_attached.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            splitscript::hir::TypedStatementKind::Expression(_)
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
            splitscript::hir::TypedStatementKind::While { .. }
        )
    }));

    let lowered = splitscript::lower_wasm(&checked);
    assert!(lowered.bodies().any(|body| {
        body.entry
            .statements
            .iter()
            .any(|statement| matches!(statement, splitscript::wasm_ir::Statement::While { .. }))
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
    let source = include_str!("async_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("await and retry should work inside while loops");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::wasm_ir::BodyOwner::Action(
            splitscript::ast::ActionKind::OnAttach,
        ))
        .expect("the fixture has an onAttach body");
    assert!(body.async_state_count >= 15);
    assert!(matches!(
        body.entry.terminator,
        splitscript::wasm_ir::Terminator::AsyncWhile { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("suspending loops should produce valid WebAssembly GC");
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
        ast::{ActionKind, BinaryOp},
        wasm_ir::{BodyOwner, Statement},
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
fn print_is_a_regular_builtin_available_in_actions() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            print("tick")
        }
    "#;
    let wasm = splitscript::compile(source).expect("print should work in whileAttached");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("print action should produce valid Wasm");
}

#[test]
fn strings_are_gc_values_with_content_equality_and_length() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let message = "tick"
            if (message == "tick" && message != "tock" && String.length(message) == 4u32) {
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
    let diagnostics = splitscript::compile(source).expect_err("bool has no supported String cast");
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
            if (isFinalLevel(13) && String.length(label) == 5u32) {
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
    let splitscript::ast::Stmt::Variable(counter) = &statements[0] else {
        panic!("expected the method receiver binding");
    };
    let splitscript::ast::Stmt::Variable(direct) = &statements[1] else {
        panic!("expected the direct call binding");
    };
    let splitscript::ast::Stmt::Variable(method) = &statements[2] else {
        panic!("expected the method call binding");
    };
    assert_eq!(
        checked.semantics().call(direct.value.id),
        Some(&ResolvedCall::UserFunction {
            function: direct_target
        })
    );
    assert_eq!(
        checked.semantics().call(method.value.id),
        Some(&ResolvedCall::UserMethod {
            function: method_target,
            receiver: ResolvedValue::Variable(counter.id),
            receiver_type: checked
                .semantics()
                .expression_type(counter.value.id)
                .expect("the method receiver has a semantic type"),
            receiver_members: Vec::new(),
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
    let splitscript::ast::Stmt::Return {
        value: Some(matched),
        ..
    } = &checked.syntax().functions[1].body.statements[0]
    else {
        panic!("expected the match return expression");
    };
    let splitscript::ast::ExprKind::Match { arms, .. } = &matched.kind else {
        panic!("expected a match expression");
    };
    let splitscript::ast::MatchPattern::Enum {
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
    assert_eq!(*receiver, ResolvedValue::Variable(binding.id));

    let lowered = splitscript::lower_wasm(&checked);
    let splitscript::wasm_ir::ExpressionKind::Match {
        value: lowered_value,
        arms: lowered_arms,
    } = &lowered
        .expression(matched.id)
        .expect("match expression should have a Wasm IR plan")
        .kind
    else {
        panic!("resolved match must not remain deferred to typed HIR")
    };
    let splitscript::ast::ExprKind::Match {
        value: matched_value,
        ..
    } = &matched.kind
    else {
        unreachable!()
    };
    assert_eq!(*lowered_value, matched_value.id);
    assert_eq!(lowered_arms[0].pattern_id, arms[0].pattern_id);
    let splitscript::wasm_ir::LoweredPattern::Enum {
        enumeration,
        variant,
        binding: lowered_binding,
    } = lowered_arms[0].pattern
    else {
        panic!("enum patterns should retain their resolved identities")
    };
    assert_eq!(
        enumeration,
        splitscript::ast::EnumTypeId::Source(checked.syntax().enums[0].id)
    );
    assert_eq!(
        variant,
        ResolvedEnumVariantId::Source(checked.syntax().enums[0].variants[0].id)
    );
    assert_eq!(lowered_binding, Some(binding.id));

    let splitscript::ast::Stmt::Variable(result) = &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the result binding");
    };
    let splitscript::ast::ExprKind::Call { args, .. } = &result.value.kind else {
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
    let splitscript::ast::Stmt::Variable(outer) = &statements[1] else {
        panic!("expected the outer binding");
    };
    assert_eq!(
        checked.semantics().record_literal_fields(outer.value.id),
        Some([outer_inner].as_slice())
    );
    assert_eq!(
        checked.typed_hir().record_literal_fields(outer.value.id),
        Some([outer_inner].as_slice())
    );
    let splitscript::ast::Stmt::Variable(nested) = &statements[2] else {
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

    let splitscript::ast::Stmt::Variable(method) = &statements[3] else {
        panic!("expected the nested receiver binding");
    };
    let Some(ResolvedCall::UserMethod {
        receiver,
        receiver_members,
        ..
    }) = checked.semantics().call(method.value.id)
    else {
        panic!("expected a resolved nested method receiver");
    };
    assert_eq!(*receiver, ResolvedValue::Variable(outer.id));
    assert_eq!(
        receiver_members,
        &[ResolvedMember::RecordField(outer_inner)]
    );

    let splitscript::ast::Stmt::Variable(address) = &statements[4] else {
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
    let splitscript::ast::Stmt::Return {
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
    let splitscript::ast::Stmt::Variable(copy) = &statements[0] else {
        panic!("expected the local copy");
    };
    assert_eq!(
        checked.semantics().value(copy.value.id),
        Some(ResolvedValue::Variable(global))
    );
    let splitscript::ast::Stmt::Variable(result) = &statements[1] else {
        panic!("expected the result binding");
    };
    let splitscript::ast::ExprKind::Call { args, .. } = &result.value.kind else {
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
            enabled: bool = true
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
        let splitscript::ast::Stmt::Variable(variable) = statement else {
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
    let splitscript::ast::Stmt::Variable(local) = &statements[0] else {
        panic!("expected the local declaration");
    };
    let splitscript::ast::Stmt::Assign {
        id: local_write, ..
    } = &statements[1]
    else {
        panic!("expected the local assignment");
    };
    let splitscript::ast::Stmt::Assign {
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

#[test]
fn user_function_types_are_inferred_across_bodies_and_call_sites() {
    let source = r#"
        state "game.exe" {}

        record Clock {
            minutes: f32
            seconds: f32
        }

        record Snapshot {
            clock: Clock
        }

        fn increment(value) {
            return value + 1
        }

        fn same(left, right) {
            return left == right
        }

        fn formatClock(snapshot) {
            return snapshot.clock.seconds
        }

        whileAttached {
            let result: u64 = increment(41)
            if (same(result, 42)) {
                print("inferred through the call graph")
            }
            let seconds: f32 = formatClock(Snapshot {
                clock: Clock { minutes: 1.0, seconds: 2.0 }
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("function and record receiver types should be inferred");
    let snapshot = checked.syntax().records[1].id;
    let format_clock = &checked.syntax().functions[2];
    assert_eq!(
        checked.semantics().types().kind(
            checked
                .semantics()
                .value_type(format_clock.params[0].id)
                .unwrap()
        ),
        &TypeKind::Record(snapshot)
    );
    let splitscript::ast::Stmt::Return {
        value: Some(returned),
        ..
    } = &format_clock.body.statements[0]
    else {
        panic!("expected formatClock's return expression");
    };
    let path = returned;
    assert_eq!(
        checked.semantics().path_members(path.id),
        Some(
            [
                ResolvedMember::RecordField(checked.syntax().records[1].fields[0].id),
                ResolvedMember::RecordField(checked.syntax().records[0].fields[1].id),
            ]
            .as_slice()
        )
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred function signatures should produce valid Wasm");

    let ambiguous = r#"
        record First { value: i32 }
        record Second { value: i32 }
        state "game.exe" {}
        fn inspect(item) { return item.value }
    "#;
    let errors = splitscript::check(splitscript::parse(ambiguous).unwrap())
        .expect_err("shared field names need enough call-site context");
    assert!(errors.iter().any(|error| {
        error.message.contains("does not uniquely determine")
            && error.message.contains("First")
            && error.message.contains("Second")
    }));
}

#[test]
fn global_types_are_inferred_from_uses_and_assignments() {
    let source = r#"
        let base = 0
        let fieldOffset = 0
        let timerState = TimerState.NotRunning
        let largeCounter = 0

        state "game.exe" {
            value: i32 = process.read.i32(base.offset(fieldOffset))
        }

        fn consumeU64(value: u64) {}

        whileAttached {
            timerState = timer.state()
            consumeU64(largeCounter)
        }
    "#;
    let wasm = splitscript::compile(source).expect("global uses should determine their types");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred globals should produce type-correct Wasm globals");
}

#[test]
fn state_field_types_are_inferred_from_expressions_and_uses() {
    let source = r#"
        state "game.exe" {
            expressionValue = process.read.u16(0)
            usageValue = 0
            pointerValue at 0x1234
        }

        fn consumeU32(value: u32) {}
        fn consumeU64(value: u64) {}

        whileAttached {
            consumeU32(current.usageValue)
            consumeU64(current.pointerValue)
        }
    "#;
    let wasm = splitscript::compile(source).expect("state field types should be inferred");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("inferred state fields should produce a concrete GC state type");

    let ambiguous = r#"
        state "game.exe" {
            mystery at 0x1234
        }
    "#;
    let diagnostics =
        splitscript::compile(ambiguous).expect_err("an unconstrained pointer field is ambiguous");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot infer type variable"))
    );
}

#[test]
fn lifecycle_blocks_use_event_and_polling_names_without_prototype_aliases() {
    use splitscript::ast::ActionKind;

    let source = r#"
        state "game.exe" {}
        onDetached { setTickRate(1.0) }
        onAttach { setTickRate(120.0) }
        whileAttached { print("tick") }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert_eq!(
        checked
            .syntax()
            .actions
            .iter()
            .map(|action| action.kind)
            .collect::<Vec<_>>(),
        [
            ActionKind::OnDetached,
            ActionKind::OnAttach,
            ActionKind::WhileAttached,
        ]
    );
    assert_eq!(ActionKind::parse("update"), None);
    assert_eq!(ActionKind::parse("detached"), None);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("canonical lifecycle blocks should produce valid Wasm");
}

#[test]
fn action_fallthroughs_use_domain_defaults_and_null_is_scoped() {
    let source = r#"
        state "game.exe" {}

        start {}
        split { return }
        reset {
            if (false) {
                return true
            }
        }
        isLoading { return None }
        gameTime {}
    "#;
    let wasm = splitscript::compile(source).expect("action fallthroughs should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("nullable action results should produce type-correct Wasm");

    let invalid = r#"
        state "game.exe" {}
        start { return None }
    "#;
    let diagnostics =
        splitscript::compile(invalid).expect_err("start must not expose a nullable result");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("can only construct an optional value")
    }));
}

#[test]
fn as_casts_lower_all_numeric_representations_and_integer_strings() {
    let source = r#"
        state "game.exe" {}

        fn exercise(small: i8, wide: u64, fraction: f32, pointer: address) {
            let widened = small as i64
            let narrowed = wide as u8
            let floating = widened as f64
            let integral = fraction as i16
            let addressValue = wide as address
            let rawAddress = pointer as u64
            print(widened as String)
            print(wide as String)
            print(addressValue as String)
        }
    "#;
    let wasm = splitscript::compile(source).expect("supported `as` casts should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("numeric casts should lower to type-correct Wasm");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let value = "not a number" as u32
        }
    "#;
    let diagnostics = splitscript::compile(invalid).expect_err("String-to-number must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not support this operation")
    }));
}

#[test]
fn gc_records_support_nesting_functions_and_async_frames() {
    let source = r#"
        state "game.exe" {}

        fn isHana(timer: TimerInfo) -> bool {
            return timer.digits.minutes == 0.0 && timer.character == "Hana"
        }

        record Digits {
            minutes: f32
            seconds: f32
            hundredths: f32
        }

        record TimerInfo {
            digits: Digits
            character: String
        }

        onAttach {
            let timer = TimerInfo {
                character: "Hana",
                digits: Digits {
                    hundredths: 0.0,
                    seconds: 0.0,
                    minutes: 0.0
                }
            }
            await process.module("GameAssembly.dll")
            if (isHana(timer)) {
                print(timer.character)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("nested records should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("record GC structs should produce valid Wasm");
}

#[test]
fn timer_state_is_a_compiler_provided_exhaustive_enum() {
    let source = r#"
        let previous = TimerState.NotRunning

        state "game.exe" {}

        whileAttached {
            let current = timer.state()
            let justStarted = previous == TimerState.NotRunning
                && current != TimerState.NotRunning
            let label = match current {
                TimerState.NotRunning => "Not Running",
                TimerState.Running => "Running",
                TimerState.Paused => "Paused",
                TimerState.Ended => "Ended",
                TimerState.Unknown => "Unknown"
            }
            previous = current
            if justStarted {
                setVariable("Transition", "Started")
            }
            setVariable("Timer State", label)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert!(
        checked
            .syntax()
            .enums
            .iter()
            .all(|enumeration| enumeration.name != "TimerState"),
        "compiler-provided declarations must not be injected into source syntax"
    );
    assert!(
        checked
            .enum_types()
            .iter()
            .all(|enumeration| enumeration.name != "TimerState"),
        "standard-library enums must not be materialized as source enum layouts"
    );
    let library = StandardLibrary::new();
    let timer_state = library.type_decl(StdlibTypeId::TimerState);
    assert_eq!(
        library
            .variants_of(timer_state.id)
            .map(|variant| variant.name)
            .collect::<Vec<_>>(),
        ["NotRunning", "Running", "Paused", "Ended", "Unknown"]
    );
    let timer_state_call = checked
        .typed_hir()
        .expressions()
        .find(|expression| {
            matches!(
                checked.typed_hir().call(expression.id),
                Some(ResolvedCall::StandardLibrary {
                    item: StdlibItemId::TimerState,
                    ..
                })
            )
        })
        .expect("timer.state should resolve through the standard-library catalog");
    assert_eq!(
        checked.semantics().types().kind(timer_state_call.ty),
        &TypeKind::Standard(StdlibTypeId::TimerState)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("TimerState host conversion and exhaustive matches should produce valid Wasm GC");

    let incomplete = r#"
        state "game.exe" {}
        whileAttached {
            let running = match timer.state() {
                TimerState.Running => true,
                TimerState.NotRunning => false
            }
        }
    "#;
    let errors = splitscript::compile(incomplete)
        .expect_err("TimerState matches must handle every state or use a wildcard");
    for missing in ["Paused", "Ended", "Unknown"] {
        assert!(errors.iter().any(|error| {
            error.message.contains("non-exhaustive match") && error.message.contains(missing)
        }));
    }

    let redeclared = r#"
        enum TimerState { Custom }
        state "game.exe" {}
    "#;
    let error = splitscript::parse(redeclared)
        .expect_err("standard-library nominal types cannot be redeclared");
    assert!(error[0].message.contains("standard-library enum"));
}

#[test]
fn enums_and_their_payloads_use_structural_equality() {
    let source = r#"
        record Position {
            x: i32
            y: i32
        }

        enum Value {
            Position(Position)
            Label(String)
            Empty
        }

        state "game.exe" {}

        whileAttached {
            let left = Value.Position(Position { x: 4, y: 8 })
            let right = Value.Position(Position { x: 4, y: 8 })
            let same = left == right
            let different = Value.Label("four") != Value.Label("five")
            let empty = Value.Empty == Value.Empty
            let recordsEqual = Position { x: 1, y: 2 } == Position { x: 1, y: 2 }
            if same && different && empty && recordsEqual {
                setVariable("Equality", "works")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("structural enum equality should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("structural enum equality should produce valid Wasm GC");

    let unsupported = r#"
        enum Values {
            Items(Array<i32>)
        }

        state "game.exe" {}

        whileAttached {
            let left = Values.Items([1, 2])
            let right = Values.Items([1, 2])
            let same = left == right
        }
    "#;
    let errors = splitscript::compile(unsupported)
        .expect_err("enum payloads without equality must be rejected precisely");
    assert!(errors.iter().any(|error| {
        error.message.contains("Values.Items")
            && error.message.contains("does not support equality")
    }));
}

#[test]
fn payload_enums_are_exhaustively_matched_and_survive_await() {
    let source = r#"
        state "game.exe" {}

        enum LevelOrScene {
            Level(i32)
            Scene(String)
        }

        fn isFirst(value: LevelOrScene) -> bool {
            return match value {
                LevelOrScene.Level(level) if level == 0 => true,
                LevelOrScene.Level(level) => false,
                LevelOrScene.Scene(scene) => scene == "Shrine01"
            }
        }

        onAttach {
            let location = LevelOrScene.Scene("Shrine01")
            await process.module("GameAssembly.dll")
            if (isFirst(location)) {
                print("first")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("payload enum should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("enum GC structs and match lowering should validate");
}

#[test]
fn match_requires_every_enum_variant() {
    let source = r#"
        state "game.exe" {}
        enum Choice {
            Yes
            No
        }
        fn choose(value: Choice) -> bool {
            return match value {
                Choice.Yes => true
            }
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("match must be exhaustive");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-exhaustive match"))
    );
}

#[test]
fn literal_matches_support_guards_wildcards_and_bidirectional_inference() {
    let source = r#"
        state "game.exe" {}

        fn character(value, dlc) {
            return match value {
                3 if dlc => "Accel",
                3 => "Erika",
                _ => "Unknown"
            }
        }

        fn booleanName(value) {
            return match value {
                true => "yes",
                false => "no"
            }
        }

        onAttach {
            print(character(3, true))
            print(booleanName(false))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("literal matches should compile");
    let lowered = splitscript::lower_wasm(&checked);
    let mut patterns = [false; 3];
    let mut guarded = false;
    for expression in lowered.expressions() {
        let splitscript::wasm_ir::ExpressionKind::Match { arms, .. } = &expression.kind else {
            continue;
        };
        for arm in arms {
            match arm.pattern {
                splitscript::wasm_ir::LoweredPattern::Bool(_) => patterns[0] = true,
                splitscript::wasm_ir::LoweredPattern::Int(_) => patterns[1] = true,
                splitscript::wasm_ir::LoweredPattern::Wildcard => patterns[2] = true,
                splitscript::wasm_ir::LoweredPattern::Enum { .. }
                | splitscript::wasm_ir::LoweredPattern::OptionNone(_)
                | splitscript::wasm_ir::LoweredPattern::OptionSome { .. }
                | splitscript::wasm_ir::LoweredPattern::ResultSuccess { .. }
                | splitscript::wasm_ir::LoweredPattern::ResultError { .. } => {}
            }
            guarded |= arm.guard.is_some();
        }
    }
    assert!(patterns.into_iter().all(|pattern| pattern));
    assert!(guarded);

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("guarded literal match lowering should validate");
}

#[test]
fn integer_matches_require_a_wildcard() {
    let source = r#"
        state "game.exe" {}
        fn character(value: u32) -> String {
            return match value {
                0 => "Hana"
            }
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("integer match must be exhaustive");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("non-exhaustive integer match") })
    );
}

#[test]
fn else_if_chains_parse_type_check_and_satisfy_return_analysis() {
    let source = r#"
        state "game.exe" {}

        fn signName(value: i32) -> String {
            if value < 0 {
                return "negative"
            } else if value & 1 == 0 {
                return "even"
            } else if value == 1 {
                return "one"
            } else {
                return "positive"
            }
        }

        onAttach {
            print(signName(1))
        }
    "#;
    let wasm = splitscript::compile(source).expect("else-if chain should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("else-if lowering should produce valid Wasm");
}

#[test]
fn comparisons_follow_rusts_non_chaining_rule() {
    let source = r#"
        state "game.exe" {}
        fn between(value: i32) -> bool {
            return 0 < value < 10
        }
    "#;
    let diagnostics =
        splitscript::compile(source).expect_err("comparison chains should require parentheses");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("comparison operators cannot be chained")
    }));
}

#[test]
fn methods_have_implicit_self_and_support_nested_receivers() {
    let source = r#"
        state "game.exe" {}

        record Digits {
            minutes: f32
            seconds: f32
        }

        record TimerInfo {
            digits: Digits
            stopped: bool
        }

        fn Digits.isZero() -> bool {
            return self.minutes == 0.0 && self.seconds == 0.0
        }

        fn TimerInfo.canStart(expectedStopped: bool) -> bool {
            return self.digits.isZero() && self.stopped == expectedStopped
        }

        whileAttached {
            let timer = TimerInfo {
                digits: Digits { minutes: 0.0, seconds: 0.0 },
                stopped: false
            }
            if (timer.canStart(false)) {
                print("method")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("methods should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("method calls should produce valid Wasm");
}

#[test]
fn generic_gc_arrays_infer_elements_and_support_core_methods() {
    let source = r#"
        state "game.exe" {}

        record ScanBuffer {
            bytes: Array<u8>
        }

        fn ScanBuffer.prepare() -> bool {
            self.bytes.set(1u32, 0x8bu8)
            return self.bytes.length() == 3u32
                && self.bytes.get(0u32) == 0x48u8
                && self.bytes.get(1u32) == 0x8bu8
        }

        onAttach {
            let inferred = [1, 2, 3]
            let empty: Array<u16> = []
            let buffer = ScanBuffer {
                bytes: [0x48u8, 0u8, 0u8]
            }
            await process.module("GameAssembly.dll")
            if (buffer.prepare()
                && inferred.get(2u32) == 3
                && empty.length() == 0u32) {
                print("array")
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("generic arrays should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("monomorphized GC arrays should validate");
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
        .body(splitscript::wasm_ir::BodyOwner::Action(
            splitscript::ast::ActionKind::OnAttach,
        ))
        .expect("Wasm lowering should expose the onAttach body");
    assert_eq!(
        body.cancellation_region,
        Some(splitscript::wasm_ir::CancellationRegion::ProcessLifetime)
    );
    let action = &checked.syntax().actions[0];
    let splitscript::ast::Stmt::Variable(before_only) = &action.body.statements[0] else {
        panic!("expected beforeOnly");
    };
    let splitscript::ast::Stmt::Variable(expected) = &action.body.statements[2] else {
        panic!("expected expected");
    };
    let splitscript::ast::Stmt::Variable(overwritten) = &action.body.statements[3] else {
        panic!("expected overwritten");
    };
    let splitscript::ast::Stmt::Variable(after_only) = &action.body.statements[5] else {
        panic!("expected afterOnly");
    };
    let splitscript::ast::Stmt::Suspend {
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
        splitscript::wasm_ir::Terminator::Suspend { .. }
    ));
    let splitscript::wasm_ir::Terminator::Suspend {
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
        Some(splitscript::wasm_ir::CancellationRegion::ProcessLifetime)
    );
    assert_eq!(live_values, &[expected.id]);
    assert!(matches!(
        continuation.statements.as_slice(),
        [
            splitscript::wasm_ir::Statement::Store { .. },
            splitscript::wasm_ir::Statement::Store { .. },
            splitscript::wasm_ir::Statement::If { .. }
        ]
    ));
    let splitscript::wasm_ir::Terminator::Suspend {
        cancellation,
        live_values,
        ..
    } = &continuation.terminator
    else {
        unreachable!()
    };
    assert_eq!(
        *cancellation,
        Some(splitscript::wasm_ir::CancellationRegion::ProcessLifetime)
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
        ast::ActionKind,
        wasm_ir::{BodyOwner, LocalPurpose},
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
        .body(splitscript::wasm_ir::BodyOwner::Action(
            splitscript::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach should have a lowered body");
    assert_eq!(body.async_state_count, 5);

    let splitscript::wasm_ir::Terminator::Suspend {
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
    let [splitscript::wasm_ir::Statement::If { then_block, .. }] =
        continuation.statements.as_slice()
    else {
        panic!("the first continuation should branch");
    };
    let splitscript::wasm_ir::Terminator::Suspend {
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
            return process.read.i32(0x3000)
        }

        onAttach {
            let marker = retry readMarker()
            print(`marker {marker}`)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("retry should unwrap a user function's inferred Result value");
    let action = &checked.syntax().actions[0];
    let splitscript::ast::Stmt::Suspend {
        mode: splitscript::ast::SuspensionMode::Retry,
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
        .body(splitscript::wasm_ir::BodyOwner::Action(
            splitscript::ast::ActionKind::OnAttach,
        ))
        .expect("onAttach should have a lowered body");
    assert!(matches!(
        body.entry.terminator,
        splitscript::wasm_ir::Terminator::Suspend {
            mode: splitscript::ast::SuspensionMode::Retry,
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
fn typed_process_reads_and_pointer_following_work_in_sync_and_async_code() {
    let source = r#"
        state "game.exe" {}
        onAttach {
            let module = await process.module("game.exe")
            let table = await process.scan(module.address, module.size, sig"48 8B ?? 00")
            let target = retry process.readRelative32(table + 0x3)
            let object = retry process.follow(module.address, [0x10u64, 0x28u64])
            let kind = retry process.read.u32(object + 0x8)
            if (target != 0 && kind == 7u32 && (process.read.bool(object + 0xc) else false)) {
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
            && error.message.contains("process.read.i32")
    }));
}

#[test]
fn memory_readable_records_have_shared_layouts_and_single_read_lowering() {
    use splitscript::memory::MemoryTypeLayout;

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
fn expression_backed_state_fields_use_discovered_addresses_and_rotate_snapshots() {
    let source = r#"
        state "game.exe" {
            points: i32 = process.read.i32(gameManager.offset(pointsOffset))
            stopped: bool = process.read.bool(timerInstance.offset(stoppedOffset))
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
            let instance = retry process.read.address(staticTable.offset(instanceOffset))
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
                records: Array<DeadRecord>,
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

#[test]
fn resolved_and_typed_hir_snapshot() {
    let source = r#"
        record Point {
            x: i32
            y: i32
        }

        enum Event {
            Idle
            Moved(Point)
        }

        state "game.exe" {
            event = Event.Idle
        }

        fn pointX(event) -> i32? {
            return match event {
                Event.Moved(point) => point.x,
                Event.Idle => None
            }
        }

        whileAttached {
            let next = Event.Moved(Point { x: 3, y: 4 })
            let x = pointX(next) else 0
            print(`x={x}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert_snapshot(
        include_str!("snapshots/resolved_typed_hir.snap"),
        &render_typed_hir_snapshot(&checked),
    );
}

#[test]
fn diagnostic_snapshot() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let wrong: i32 = "text"
            let impossible = true + 1
            let missing = unknown
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("fixture is intentionally invalid");
    let rendered = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.render("diagnostics.split", source))
        .collect::<Vec<_>>()
        .join("\n");
    assert_snapshot(include_str!("snapshots/diagnostics.snap"), &rendered);
}

#[test]
fn diagnostics_expose_stable_stage_codes_and_severity() {
    use splitscript::{DiagnosticCode, DiagnosticSeverity};

    let lexical = splitscript::parse("@").expect_err("the source contains an invalid character");
    assert_eq!(lexical[0].code, DiagnosticCode::Lexical);
    assert_eq!(lexical[0].code.as_str(), "SS0001");
    assert_eq!(lexical[0].severity, DiagnosticSeverity::Error);
    assert_eq!(
        lexical[0].render("invalid.split", "@"),
        "invalid.split:1:1: error[SS0001]: unexpected character"
    );

    let syntax = splitscript::parse(
        r#"
            state "game.exe" { broken = + }
        "#,
    )
    .expect_err("the state expression is malformed");
    assert!(
        syntax
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::Syntax)
    );
    assert_eq!(DiagnosticCode::Syntax.as_str(), "SS0002");

    let type_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value: i32 = "wrong" }
        "#,
    )
    .expect_err("the initializer has the wrong type");
    assert!(
        type_errors
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::Type)
    );
    assert_eq!(DiagnosticCode::Type.as_str(), "SS0003");

    let semantic = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetached { process.read.i32(0x1000) }
        "#,
    )
    .expect_err("process access requires an attachment");
    assert!(
        semantic
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::Semantic)
    );
    assert_eq!(DiagnosticCode::Semantic.as_str(), "SS0004");
}

#[test]
fn structured_diagnostics_render_labels_notes_and_multi_edit_fixes() {
    use splitscript::{
        Diagnostic, DiagnosticFix, DiagnosticLabelStyle, FixApplicability, TextEdit, ast::Span,
    };

    let source = "first\nsecond\n";
    let diagnostic = Diagnostic::type_error("values are reversed", Span { start: 6, end: 12 })
        .with_primary_label("this value belongs first")
        .with_secondary_label(Span { start: 0, end: 5 }, "this value belongs second")
        .with_note("the two values must appear in declaration order")
        .with_fix(DiagnosticFix {
            title: "swap the values".to_owned(),
            applicability: FixApplicability::MachineApplicable,
            edits: vec![
                TextEdit {
                    span: Span { start: 0, end: 5 },
                    replacement: "second".to_owned(),
                },
                TextEdit {
                    span: Span { start: 6, end: 12 },
                    replacement: "first".to_owned(),
                },
            ],
        });

    assert_eq!(diagnostic.labels.len(), 2);
    assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Primary);
    assert_eq!(diagnostic.labels[1].style, DiagnosticLabelStyle::Secondary);
    assert_eq!(diagnostic.fixes[0].edits.len(), 2);
    assert_eq!(
        diagnostic.render("example.split", source),
        concat!(
            "example.split:2:1: error[SS0003]: values are reversed\n",
            "  = primary: this value belongs first\n",
            "  = secondary example.split:1:1: this value belongs second\n",
            "  = note: the two values must appear in declaration order\n",
            "  = help: swap the values (machine-applicable)"
        )
    );
}

fn assert_snapshot(expected: &str, actual: &str) {
    assert_eq!(actual.trim_end(), expected.trim_end());
}

fn render_typed_hir_snapshot(checked: &splitscript::CheckedProgram) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    writeln!(output, "declarations:").unwrap();
    for declaration in checked.hir().declarations() {
        writeln!(output, "  {:?} name={}", declaration.id, declaration.name).unwrap();
    }

    writeln!(output, "signatures:").unwrap();
    for function in &checked.syntax().functions {
        let parameters = function
            .params
            .iter()
            .map(|parameter| {
                snapshot_type_name(
                    checked,
                    checked
                        .semantics()
                        .value_type(parameter.id)
                        .expect("checked parameters have types"),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let result = snapshot_type_name(
            checked,
            checked
                .semantics()
                .function_result(function.id)
                .expect("checked functions have result types"),
        );
        writeln!(
            output,
            "  FunctionId({}) {}({parameters}) -> {result}",
            function.id.index(),
            function.name
        )
        .unwrap();
    }

    writeln!(output, "bodies:").unwrap();
    for function in checked.typed_hir().function_bodies() {
        writeln!(output, "  function {}:", function.function.index()).unwrap();
        render_typed_block(&mut output, &function.body, 2);
    }
    for action in checked.typed_hir().action_bodies() {
        writeln!(output, "  action {}:", action.action.name()).unwrap();
        render_typed_block(&mut output, &action.body, 2);
    }

    writeln!(output, "expressions:").unwrap();
    for expression in checked.typed_hir().expressions() {
        let ty = snapshot_type_name(checked, expression.ty);
        let kind = snapshot_expression_kind(checked, &expression.kind);
        write!(output, "  e{}: {ty} = {kind}", expression.id.index()).unwrap();
        if let Some(resolution) = &expression.resolution {
            write!(output, " resolve={resolution:?}").unwrap();
        }
        if let Some(conversion) = expression.conversion {
            write!(
                output,
                " convert={:?} {} -> {}",
                conversion.kind,
                snapshot_type_name(checked, conversion.source),
                snapshot_type_name(checked, conversion.target)
            )
            .unwrap();
        }
        writeln!(output).unwrap();
    }
    output
}

fn render_typed_block(output: &mut String, block: &splitscript::hir::TypedBlock, depth: usize) {
    use splitscript::hir::TypedStatementKind;
    use std::fmt::Write as _;

    let indent = "  ".repeat(depth);
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Variable { value, initializer } => {
                writeln!(
                    output,
                    "{indent}let v{} = e{}",
                    value.index(),
                    initializer.index()
                )
                .unwrap();
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                writeln!(
                    output,
                    "{indent}assign a{} -> v{} op={op:?} value=e{}",
                    assignment.id.index(),
                    assignment.target.index(),
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                writeln!(output, "{indent}if e{}:", condition.index()).unwrap();
                render_typed_block(output, then_block, depth + 1);
                if let Some(else_block) = else_block {
                    writeln!(output, "{indent}else:").unwrap();
                    render_typed_block(output, else_block, depth + 1);
                }
            }
            TypedStatementKind::While { condition, body } => {
                writeln!(output, "{indent}while e{}:", condition.index()).unwrap();
                render_typed_block(output, body, depth + 1);
            }
            TypedStatementKind::Break => writeln!(output, "{indent}break").unwrap(),
            TypedStatementKind::Continue => writeln!(output, "{indent}continue").unwrap(),
            TypedStatementKind::Return(value) => match value {
                Some(value) => writeln!(output, "{indent}return e{}", value.index()).unwrap(),
                None => writeln!(output, "{indent}return").unwrap(),
            },
            TypedStatementKind::Throw { error, target } => {
                writeln!(
                    output,
                    "{indent}throw e{} -> t{}",
                    error.index(),
                    target.index()
                )
                .unwrap();
            }
            TypedStatementKind::Suspend {
                mode,
                binding,
                value,
            } => {
                writeln!(
                    output,
                    "{indent}suspend {mode:?} binding={binding:?} value=e{}",
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::Expression(expression) => {
                writeln!(output, "{indent}evaluate e{}", expression.index()).unwrap();
            }
        }
    }
}

fn snapshot_expression_kind(
    checked: &splitscript::CheckedProgram,
    kind: &splitscript::hir::TypedExpressionKind,
) -> String {
    use splitscript::hir::{TypedExpressionKind, TypedFallbackBranch, TypedInterpolatedPart};

    match kind {
        TypedExpressionKind::None => "None".to_owned(),
        TypedExpressionKind::Bool(value) => value.to_string(),
        TypedExpressionKind::Int { value, suffix } => format!("int {value} suffix={suffix:?}"),
        TypedExpressionKind::Float(value) => format!("float {value}"),
        TypedExpressionKind::String(value) => format!("string {value:?}"),
        TypedExpressionKind::InterpolatedString(parts) => format!(
            "interpolate [{}]",
            parts
                .iter()
                .map(|part| match part {
                    TypedInterpolatedPart::Text(text) => format!("text {text:?}"),
                    TypedInterpolatedPart::Expression {
                        expression,
                        conversion,
                    } => format!(
                        "e{} conversion={}",
                        expression.index(),
                        conversion.map_or_else(
                            || "none".to_owned(),
                            |splitscript::hir::ImplicitConversion::ToString { source }| {
                                format!("ToString<{}>", snapshot_type_name(checked, source))
                            }
                        )
                    ),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypedExpressionKind::Signature(value) => format!("signature {value:?}"),
        TypedExpressionKind::Array(values) => format!("array {values:?}"),
        TypedExpressionKind::Record { record, fields } => {
            format!("record {record} fields={fields:?}")
        }
        TypedExpressionKind::Enum {
            enumeration,
            variant,
            payload,
        } => format!("enum {enumeration}.{variant} payload={payload:?}"),
        TypedExpressionKind::Match { value, arms } => format!(
            "match e{} arms=[{}]",
            value.index(),
            arms.iter()
                .map(|arm| format!(
                    "p{} {:?} guard={:?} value=e{}",
                    arm.resolution.id.index(),
                    arm.pattern,
                    arm.guard,
                    arm.value.index()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => format!(
            "if e{} then=e{} else=e{}",
            condition.index(),
            then_expr.index(),
            else_expr.index()
        ),
        TypedExpressionKind::Fallback { value, fallback } => match fallback {
            TypedFallbackBranch::Value(fallback) => {
                format!("fallback e{} else=e{}", value.index(), fallback.index())
            }
            TypedFallbackBranch::Return(return_value) => {
                format!("fallback e{} else=return {return_value:?}", value.index())
            }
            TypedFallbackBranch::Break => format!("fallback e{} else=break", value.index()),
            TypedFallbackBranch::Continue => {
                format!("fallback e{} else=continue", value.index())
            }
        },
        TypedExpressionKind::Propagate { value, target } => {
            format!("propagate e{} -> t{}", value.index(), target.index())
        }
        TypedExpressionKind::Path(path) => format!("path {}", path.join(".")),
        TypedExpressionKind::Member { receiver, name, .. } => {
            format!("member e{}.{}", receiver.index(), name)
        }
        TypedExpressionKind::Unary { op, expression } => {
            format!("{op:?} e{}", expression.index())
        }
        TypedExpressionKind::Cast { expression, target } => {
            format!("cast e{} as {target:?}", expression.index())
        }
        TypedExpressionKind::Binary { op, left, right } => {
            format!("{op:?} e{} e{}", left.index(), right.index())
        }
        TypedExpressionKind::Call {
            source_path,
            arguments,
        } => format!("call {} args={arguments:?}", source_path.join(".")),
    }
}

fn snapshot_type_name(
    checked: &splitscript::CheckedProgram,
    ty: splitscript::types::TypeId,
) -> String {
    match checked.semantics().types().kind(ty) {
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Standard(standard) => StandardLibrary::new().type_decl(*standard).name.to_owned(),
        TypeKind::Record(id) => checked
            .syntax()
            .records
            .iter()
            .find(|record| record.id == *id)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| format!("record#{id}")),
        TypeKind::Enum(id) => checked
            .enum_types()
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{id}")),
        TypeKind::Array { element, .. } => {
            format!("Array<{}>", snapshot_type_name(checked, *element))
        }
        TypeKind::Option { value, .. } => format!("{}?", snapshot_type_name(checked, *value)),
        TypeKind::Result { value, .. } => format!("{}!", snapshot_type_name(checked, *value)),
    }
}
