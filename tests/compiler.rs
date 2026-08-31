use wasmparser::{Parser, Payload, TypeRef, Validator, WasmFeatures};

use splitscript::compiler::stdlib::semantic::StandardLibrarySemanticExt;
use splitscript::{
    compiler::{
        abi::{AbiCatalog, AbiEffect, AbiImportId, AbiOwnership},
        semantic::{
            ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedReceiver,
            ResolvedStructFieldId, ResolvedValue,
        },
        stdlib::{
            Availability, CancellationKind, CoreTypeId, Effect, FieldVisibility, Implementation,
            IntrinsicId, ItemVisibility, StandardBinaryOperator, StandardLibrary,
            StandardUnaryOperator, StdlibFieldId, StdlibItemId, StdlibOwner, StdlibStateProviderId,
            StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId, SuspensionKind, TypeVisibility,
        },
        types::{BuiltinType, TypeKind},
    },
    tooling::language::{LanguageCatalog, LanguageItemId, LanguageItemKind},
};

const EXAMPLE: &str = include_str!("../examples/lunistice.split");
const HELLO: &str = include_str!("../examples/hello_lunistice.split");
const SETTINGS_EXAMPLE: &str = include_str!("../examples/lso_desktop_settings.split");

#[path = "compiler/async_runtime.rs"]
mod async_runtime;
#[path = "compiler/catalogs_types.rs"]
mod catalogs_types;
#[path = "compiler/cli.rs"]
mod cli;
#[path = "compiler/closures.rs"]
mod closures;
#[path = "compiler/compiler_queries.rs"]
mod compiler_queries;
#[path = "compiler/diagnostics_migration.rs"]
mod diagnostics_migration;
#[path = "compiler/expressions_control.rs"]
mod expressions_control;
#[path = "compiler/failure_semantics.rs"]
mod failure_semantics;
#[path = "compiler/file_runtime.rs"]
mod file_runtime;
#[path = "compiler/inference_language.rs"]
mod inference_language;
#[path = "compiler/iterators.rs"]
mod iterators;
#[path = "compiler/parser_recovery.rs"]
mod parser_recovery;
#[path = "compiler/port_review.rs"]
mod port_review;
#[path = "compiler/profiles_codegen.rs"]
mod profiles_codegen;
#[path = "compiler/ranges.rs"]
mod ranges;
#[path = "compiler/sets.rs"]
mod sets;
#[path = "compiler/snapshots.rs"]
mod snapshots;
#[path = "compiler/state_layouts.rs"]
mod state_layouts;
