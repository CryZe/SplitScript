use std::collections::BTreeSet;

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program, SettingFileFilter, SettingKind, StateSource},
    intrinsic_registry::{self, DependencyRoot, RuntimeHelperId},
    semantic::SemanticModel,
    stdlib::{CoreTypeId, Implementation, IntrinsicId, StdlibItemId, StdlibTypeId},
    types::TypeKind,
    wasm_ir,
};

use super::runtime_helper_registry;

#[derive(Debug, Default)]
pub(super) struct BackendDependencies {
    stdlib_items: BTreeSet<StdlibItemId>,
    helpers: BTreeSet<RuntimeHelperId>,
    host_imports: BTreeSet<AbiImportId>,
}

impl BackendDependencies {
    pub fn analyze(
        program: &Program,
        semantics: &SemanticModel,
        wasm_ir: &wasm_ir::Program,
        reachability: &super::reachability::Reachability,
    ) -> Self {
        let mut dependencies = Self::default();
        dependencies.require_import(AbiImportId::TimerGetState);
        dependencies.require_import(AbiImportId::ProcessAttach);
        dependencies.require_import(AbiImportId::ProcessDetach);
        dependencies.require_import(AbiImportId::ProcessIsOpen);

        if let Some(state) = &program.state {
            if let Some(provider) = semantics.state_provider() {
                let provider = wasm_ir.standard_library().state_provider(provider);
                if state
                    .fields
                    .iter()
                    .any(|field| matches!(field.source, StateSource::Pointer(_)))
                {
                    let Implementation::Intrinsic(direct_read) = wasm_ir
                        .standard_library()
                        .item(provider.direct_read)
                        .implementation
                    else {
                        unreachable!("validated state-provider reads are intrinsic")
                    };
                    dependencies.require_intrinsic(direct_read);
                }
            }
            for field in state.all_fields() {
                if let StateSource::Pointer(path) = &field.source {
                    dependencies.require_import(AbiImportId::ProcessRead);
                    if let Some(decoder) = path.decoder {
                        dependencies.require_intrinsic(match decoder {
                            crate::ast::StateMemoryDecoder::Utf8 { .. } => {
                                IntrinsicId::ProcessReadUtf8
                            }
                            crate::ast::StateMemoryDecoder::Utf16Le { .. } => {
                                IntrinsicId::ProcessReadUtf16Le
                            }
                        });
                    }
                    if matches!(path.base, crate::ast::PointerPathBase::Module { .. }) {
                        dependencies.require_import(AbiImportId::ProcessGetModuleAddress);
                    }
                }
            }
        }
        for action in &program.actions {
            match action.kind {
                ActionKind::Start => dependencies.require_import(AbiImportId::TimerStart),
                ActionKind::Split => dependencies.require_import(AbiImportId::TimerSplit),
                ActionKind::Reset => dependencies.require_import(AbiImportId::TimerReset),
                ActionKind::IsLoading => {
                    dependencies.require_import(AbiImportId::TimerPauseGameTime);
                    dependencies.require_import(AbiImportId::TimerResumeGameTime);
                }
                ActionKind::GameTime => {
                    dependencies.require_import(AbiImportId::TimerSetGameTime);
                }
                ActionKind::Setup
                | ActionKind::OnAttach
                | ActionKind::OnDetached
                | ActionKind::OnProcessExit
                | ActionKind::OnStateReady
                | ActionKind::WhileAttached => {}
            }
        }

        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::Call {
                    target:
                        wasm_ir::CallTarget::Intrinsic {
                            item, intrinsic, ..
                        },
                    ..
                } => {
                    dependencies.stdlib_items.insert(*item);
                    dependencies.require_intrinsic(*intrinsic);
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    dependencies.require(RuntimeHelperId::JoinStrings);
                    for source in parts.iter().filter_map(|part| match part {
                        wasm_ir::InterpolatedPart::Expression {
                            string_conversion_source,
                            ..
                        } => *string_conversion_source,
                        wasm_ir::InterpolatedPart::Text(_) => None,
                    }) {
                        dependencies.require(
                            if matches!(
                                semantics.types().kind(source),
                                TypeKind::Builtin(CoreTypeId::Char)
                            ) {
                                RuntimeHelperId::FormatChar
                            } else {
                                RuntimeHelperId::FormatI64
                            },
                        );
                    }
                }
                wasm_ir::ExpressionKind::Cast { value }
                    if matches!(
                        semantics.types().kind(expression.ty),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    let source = wasm_ir
                        .expression(*value)
                        .expect("cast operand belongs to Wasm IR")
                        .ty;
                    dependencies.require(
                        if matches!(
                            semantics.types().kind(source),
                            TypeKind::Builtin(CoreTypeId::Char)
                        ) {
                            RuntimeHelperId::FormatChar
                        } else {
                            RuntimeHelperId::FormatI64
                        },
                    );
                }
                _ => {}
            }
        }

        if reachability.requires_string_equality() {
            dependencies.require(RuntimeHelperId::StringEquality);
        }

        if !program.settings.is_empty() {
            let has_values = program
                .settings
                .iter()
                .any(|setting| !matches!(&setting.kind, SettingKind::Title { .. }));
            let has_string_values = program.settings.iter().any(|setting| {
                matches!(
                    &setting.kind,
                    SettingKind::Choice { .. } | SettingKind::File { .. }
                )
            });
            if has_values {
                dependencies.require(RuntimeHelperId::RefreshSettings);
                dependencies.require_import(AbiImportId::SettingsMapLoad);
                dependencies.require_import(AbiImportId::SettingsMapFree);
                dependencies.require_import(AbiImportId::SettingsMapGet);
                dependencies.require_import(AbiImportId::SettingValueFree);
            }
            if has_string_values {
                dependencies.require(RuntimeHelperId::StringFromMemory);
                dependencies.require(RuntimeHelperId::StringEquality);
            }
            for setting in &program.settings {
                if setting.tooltip.is_some() {
                    dependencies.require_import(AbiImportId::UserSettingsSetTooltip);
                }
                match &setting.kind {
                    SettingKind::Bool { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddBool);
                        dependencies.require_import(AbiImportId::SettingValueGetBool);
                    }
                    SettingKind::Title { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddTitle);
                    }
                    SettingKind::Choice { .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddChoice);
                        dependencies.require_import(AbiImportId::UserSettingsAddChoiceOption);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                    }
                    SettingKind::File { filters, .. } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddFileSelect);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                        for filter in filters {
                            dependencies.require_import(match filter {
                                SettingFileFilter::Name { .. } => {
                                    AbiImportId::UserSettingsAddFileSelectNameFilter
                                }
                                SettingFileFilter::Mime { .. } => {
                                    AbiImportId::UserSettingsAddFileSelectMimeFilter
                                }
                            });
                        }
                    }
                }
            }
        }

        dependencies
    }

    pub fn uses_helper(&self, helper: RuntimeHelperId) -> bool {
        self.helpers.contains(&helper)
    }

    pub fn helpers(&self) -> impl Iterator<Item = RuntimeHelperId> + '_ {
        runtime_helper_registry::DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.id)
            .filter(|helper| self.uses_helper(*helper))
    }

    pub fn host_imports(&self) -> impl Iterator<Item = AbiImportId> + '_ {
        self.host_imports.iter().copied()
    }

    fn require_intrinsic(&mut self, intrinsic: IntrinsicId) {
        for root in intrinsic_registry::contract(intrinsic).dependency_roots {
            match root {
                DependencyRoot::Helper(helper) => self.require(*helper),
                DependencyRoot::HostImport(import) => self.require_import(*import),
            }
        }
    }

    fn require(&mut self, helper: RuntimeHelperId) {
        if !self.helpers.insert(helper) {
            return;
        }
        let descriptor = runtime_helper_registry::descriptor(helper);
        for import in descriptor.host_imports {
            self.require_import(*import);
        }
        for dependency in descriptor.dependencies {
            self.require(*dependency);
        }
    }

    fn require_import(&mut self, import: AbiImportId) {
        self.host_imports.insert(import);
    }

    #[cfg(test)]
    pub fn with_host_imports(imports: impl IntoIterator<Item = AbiImportId>) -> Self {
        let mut dependencies = Self::default();
        dependencies.host_imports.extend(imports);
        dependencies
    }
}

#[cfg(test)]
mod tests {
    use crate::intrinsic_registry::RuntimeHelperId;

    use super::BackendDependencies;

    #[test]
    fn helper_dependencies_are_closed_transitively() {
        let mut dependencies = BackendDependencies::default();
        dependencies.require(RuntimeHelperId::UnityGetStaticInstance);

        assert!(dependencies.uses_helper(RuntimeHelperId::UnityGetStaticInstance));
        assert!(dependencies.uses_helper(RuntimeHelperId::UnityGetFieldAny));
        assert!(dependencies.uses_helper(RuntimeHelperId::UnityGetFieldOffset));
        assert!(dependencies.uses_helper(RuntimeHelperId::CStringEquality));
        assert!(dependencies.uses_helper(RuntimeHelperId::BackingFieldEquality));
    }
}
