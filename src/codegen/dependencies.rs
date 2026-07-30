use std::collections::BTreeSet;

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program, SettingFileFilter, SettingKind, StateSource},
    semantic::{ResolvedCall, SemanticModel},
    stdlib::{Implementation, IntrinsicId, StandardLibrary, StdlibItemId, StdlibTypeId},
    types::TypeKind,
    wasm_ir,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum GeneratedHelper {
    PrintString,
    TimerSetVariable,
    FormatI64,
    ConcatStrings,
    StringEquality,
    ScanProcessRange,
    ReadRelative32,
    ReadManagedString,
    FollowAddress,
    UnityAttach,
    UnityGetImage,
    UnityGetClass,
    UnityGetFieldOffset,
    UnityGetFieldAny,
    UnityGetStaticInstance,
    CStringEquality,
    BackingFieldEquality,
    StringFromMemory,
    RefreshSettings,
}

impl GeneratedHelper {
    pub const CORE_ORDER: &'static [Self] = &[
        Self::PrintString,
        Self::TimerSetVariable,
        Self::FormatI64,
        Self::StringEquality,
        Self::ScanProcessRange,
        Self::ReadRelative32,
        Self::ReadManagedString,
        Self::UnityAttach,
        Self::CStringEquality,
        Self::BackingFieldEquality,
        Self::UnityGetImage,
        Self::UnityGetClass,
        Self::UnityGetFieldOffset,
        Self::UnityGetFieldAny,
        Self::UnityGetStaticInstance,
        Self::ConcatStrings,
        Self::FollowAddress,
    ];

    pub const SETTINGS_ORDER: &'static [Self] = &[Self::StringFromMemory, Self::RefreshSettings];
}

#[derive(Debug, Default)]
pub(super) struct BackendDependencies {
    stdlib_items: BTreeSet<StdlibItemId>,
    helpers: BTreeSet<GeneratedHelper>,
    host_imports: BTreeSet<AbiImportId>,
}

impl BackendDependencies {
    pub fn analyze(
        program: &Program,
        semantics: &SemanticModel,
        wasm_ir: &wasm_ir::Program,
        reachability: &super::reachability::Reachability,
    ) -> Self {
        let library = StandardLibrary::new();
        let mut dependencies = Self::default();
        dependencies.require_import(AbiImportId::TimerGetState);
        dependencies.require_import(AbiImportId::ProcessAttach);
        dependencies.require_import(AbiImportId::ProcessDetach);
        dependencies.require_import(AbiImportId::ProcessIsOpen);

        if let Some(state) = &program.state {
            for field in &state.fields {
                if let StateSource::Pointer(path) = &field.source {
                    dependencies.require_import(AbiImportId::ProcessRead);
                    if path.module.is_some() {
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
                ActionKind::OnAttach | ActionKind::OnDetached | ActionKind::WhileAttached => {}
            }
        }

        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            match &expression.kind {
                wasm_ir::ExpressionKind::Call {
                    target: ResolvedCall::StandardLibrary { item, .. },
                    ..
                } => {
                    dependencies.stdlib_items.insert(*item);
                    let Implementation::Intrinsic(intrinsic) = library.item(*item).implementation;
                    dependencies.require_intrinsic(intrinsic);
                }
                wasm_ir::ExpressionKind::InterpolatedString(parts) => {
                    dependencies.require(GeneratedHelper::ConcatStrings);
                    if parts.iter().any(|part| {
                        matches!(
                            part,
                            wasm_ir::InterpolatedPart::Expression {
                                string_conversion_source: Some(_),
                                ..
                            }
                        )
                    }) {
                        dependencies.require(GeneratedHelper::FormatI64);
                    }
                }
                wasm_ir::ExpressionKind::Cast { .. }
                    if matches!(
                        semantics.types().kind(expression.ty),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    dependencies.require(GeneratedHelper::FormatI64);
                }
                _ => {}
            }
        }

        if reachability.requires_string_equality() {
            dependencies.require(GeneratedHelper::StringEquality);
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
                dependencies.require(GeneratedHelper::RefreshSettings);
                dependencies.require_import(AbiImportId::SettingsMapLoad);
                dependencies.require_import(AbiImportId::SettingsMapFree);
                dependencies.require_import(AbiImportId::SettingsMapGet);
                dependencies.require_import(AbiImportId::SettingValueFree);
            }
            if has_string_values {
                dependencies.require(GeneratedHelper::StringFromMemory);
                dependencies.require(GeneratedHelper::StringEquality);
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
                    SettingKind::File { filters } => {
                        dependencies.require_import(AbiImportId::UserSettingsAddFileSelect);
                        dependencies.require_import(AbiImportId::SettingValueGetString);
                        for filter in filters {
                            dependencies.require_import(match filter {
                                SettingFileFilter::Name { .. } => {
                                    AbiImportId::UserSettingsAddFileSelectNameFilter
                                }
                                SettingFileFilter::Mime(_) => {
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

    pub fn uses_helper(&self, helper: GeneratedHelper) -> bool {
        self.helpers.contains(&helper)
    }

    pub fn uses_unity_metadata(&self) -> bool {
        self.uses_helper(GeneratedHelper::UnityAttach)
    }

    pub fn core_helpers(&self) -> impl Iterator<Item = GeneratedHelper> + '_ {
        GeneratedHelper::CORE_ORDER
            .iter()
            .copied()
            .filter(|helper| self.uses_helper(*helper))
    }

    pub fn settings_helpers(&self) -> impl Iterator<Item = GeneratedHelper> + '_ {
        GeneratedHelper::SETTINGS_ORDER
            .iter()
            .copied()
            .filter(|helper| self.uses_helper(*helper))
    }

    pub fn host_imports(&self) -> impl Iterator<Item = AbiImportId> + '_ {
        self.host_imports.iter().copied()
    }

    fn require_intrinsic(&mut self, intrinsic: IntrinsicId) {
        match intrinsic {
            IntrinsicId::Print => self.require(GeneratedHelper::PrintString),
            IntrinsicId::StringConcat => self.require(GeneratedHelper::ConcatStrings),
            IntrinsicId::TimerSetVariable => self.require(GeneratedHelper::TimerSetVariable),
            IntrinsicId::TimerState => self.require_import(AbiImportId::TimerGetState),
            IntrinsicId::RuntimeSetTickRate => {
                self.require_import(AbiImportId::RuntimeSetTickRate);
            }
            IntrinsicId::ProcessModule => {
                self.require_import(AbiImportId::ProcessGetModuleAddress);
                self.require_import(AbiImportId::ProcessGetModuleSize);
            }
            IntrinsicId::ProcessRead => self.require_import(AbiImportId::ProcessRead),
            IntrinsicId::ProcessFollow => self.require(GeneratedHelper::FollowAddress),
            IntrinsicId::ProcessScan | IntrinsicId::ModuleScan => {
                self.require(GeneratedHelper::ScanProcessRange);
            }
            IntrinsicId::ProcessReadRelative32 => {
                self.require(GeneratedHelper::ReadRelative32);
            }
            IntrinsicId::ProcessReadManagedString => {
                self.require(GeneratedHelper::ReadManagedString);
            }
            IntrinsicId::UnityIl2Cpp => {
                self.require(GeneratedHelper::UnityAttach);
            }
            IntrinsicId::UnityModuleImage => {
                self.require(GeneratedHelper::UnityGetImage);
            }
            IntrinsicId::UnityImageClass => {
                self.require(GeneratedHelper::UnityGetClass);
            }
            IntrinsicId::UnityClassField => {
                self.require(GeneratedHelper::UnityGetFieldOffset);
            }
            IntrinsicId::UnityClassFieldAny => {
                self.require(GeneratedHelper::UnityGetFieldAny);
            }
            IntrinsicId::UnityClassStaticInstance => {
                self.require(GeneratedHelper::UnityGetStaticInstance);
            }
            IntrinsicId::NumericMin
            | IntrinsicId::NumericMax
            | IntrinsicId::NumericClamp
            | IntrinsicId::StringLength
            | IntrinsicId::NextTick
            | IntrinsicId::DurationFromFrames
            | IntrinsicId::DurationFromParts
            | IntrinsicId::DurationSaturatingSecondsF32
            | IntrinsicId::AddressOffset
            | IntrinsicId::AddressAdd
            | IntrinsicId::ArrayLength
            | IntrinsicId::ArrayGet
            | IntrinsicId::ArraySet => {}
            IntrinsicId::UnityClassStaticTable => {
                self.require_import(AbiImportId::ProcessRead);
            }
        }
    }

    fn require(&mut self, helper: GeneratedHelper) {
        if !self.helpers.insert(helper) {
            return;
        }
        match helper {
            GeneratedHelper::PrintString => {
                self.require_import(AbiImportId::RuntimePrintMessage);
            }
            GeneratedHelper::TimerSetVariable => {
                self.require_import(AbiImportId::TimerSetVariable);
            }
            GeneratedHelper::ScanProcessRange
            | GeneratedHelper::ReadRelative32
            | GeneratedHelper::ReadManagedString
            | GeneratedHelper::FollowAddress
            | GeneratedHelper::CStringEquality
            | GeneratedHelper::BackingFieldEquality
            | GeneratedHelper::UnityGetImage
            | GeneratedHelper::UnityGetClass
            | GeneratedHelper::UnityGetFieldOffset
            | GeneratedHelper::UnityGetStaticInstance => {
                self.require_import(AbiImportId::ProcessRead);
            }
            GeneratedHelper::UnityAttach => {
                self.require_import(AbiImportId::ProcessGetModuleAddress);
                self.require_import(AbiImportId::ProcessGetModuleSize);
            }
            GeneratedHelper::FormatI64
            | GeneratedHelper::ConcatStrings
            | GeneratedHelper::StringEquality
            | GeneratedHelper::UnityGetFieldAny
            | GeneratedHelper::StringFromMemory
            | GeneratedHelper::RefreshSettings => {}
        }
        match helper {
            GeneratedHelper::UnityAttach => {
                self.require(GeneratedHelper::ScanProcessRange);
                self.require(GeneratedHelper::ReadRelative32);
            }
            GeneratedHelper::UnityGetImage | GeneratedHelper::UnityGetClass => {
                self.require(GeneratedHelper::CStringEquality);
            }
            GeneratedHelper::UnityGetFieldOffset => {
                self.require(GeneratedHelper::CStringEquality);
                self.require(GeneratedHelper::BackingFieldEquality);
            }
            GeneratedHelper::UnityGetFieldAny => {
                self.require(GeneratedHelper::UnityGetFieldOffset);
            }
            GeneratedHelper::UnityGetStaticInstance => {
                self.require(GeneratedHelper::UnityGetFieldAny);
            }
            GeneratedHelper::PrintString
            | GeneratedHelper::TimerSetVariable
            | GeneratedHelper::FormatI64
            | GeneratedHelper::ConcatStrings
            | GeneratedHelper::StringEquality
            | GeneratedHelper::ScanProcessRange
            | GeneratedHelper::ReadRelative32
            | GeneratedHelper::ReadManagedString
            | GeneratedHelper::FollowAddress
            | GeneratedHelper::CStringEquality
            | GeneratedHelper::BackingFieldEquality
            | GeneratedHelper::StringFromMemory
            | GeneratedHelper::RefreshSettings => {}
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
    use super::*;

    #[test]
    fn helper_dependencies_are_closed_transitively() {
        let mut dependencies = BackendDependencies::default();
        dependencies.require(GeneratedHelper::UnityGetStaticInstance);

        assert!(dependencies.uses_helper(GeneratedHelper::UnityGetStaticInstance));
        assert!(dependencies.uses_helper(GeneratedHelper::UnityGetFieldAny));
        assert!(dependencies.uses_helper(GeneratedHelper::UnityGetFieldOffset));
        assert!(dependencies.uses_helper(GeneratedHelper::CStringEquality));
        assert!(dependencies.uses_helper(GeneratedHelper::BackingFieldEquality));
    }

    #[test]
    fn unity_attach_requires_its_scanning_helpers() {
        let mut dependencies = BackendDependencies::default();
        dependencies.require_intrinsic(IntrinsicId::UnityIl2Cpp);

        assert!(dependencies.uses_unity_metadata());
        assert!(dependencies.uses_helper(GeneratedHelper::ScanProcessRange));
        assert!(dependencies.uses_helper(GeneratedHelper::ReadRelative32));
    }
}
