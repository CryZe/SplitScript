use std::collections::BTreeSet;

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program, SettingFileFilter, SettingKind, StateSource},
    intrinsic_registry::{self, DependencyRoot, RuntimeHelperId},
    semantic::SemanticModel,
    stdlib::{CoreTypeId, Implementation, IntrinsicId, StdlibItemId, StdlibTypeId},
    types::{TypeId, TypeKind},
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
        // Polling rates are lifecycle policy even when source never calls the
        // dynamic setTickRate API directly.
        dependencies.require_import(AbiImportId::RuntimeSetTickRate);

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
                | ActionKind::OnDetach
                | ActionKind::OnStateReady
                | ActionKind::WhileAttached => {}
            }
        }

        for (owner, expression_id) in reachability.expression_instances() {
            let expression = wasm_ir
                .expression(expression_id)
                .expect("reachable expressions belong to Wasm IR");
            let specialize = |ty| {
                owner
                    .as_ref()
                    .map_or(ty, |instance| semantics.specialize_type(instance, ty))
            };
            match &expression.kind {
                wasm_ir::ExpressionKind::Call { target, .. }
                    if matches!(
                        reachability.resolved_call_target(owner.as_ref(), expression.id, target),
                        wasm_ir::CallTarget::DefaultDisplay { .. }
                    ) =>
                {
                    let wasm_ir::CallTarget::DefaultDisplay { receiver_type, .. } =
                        reachability.resolved_call_target(owner.as_ref(), expression.id, target)
                    else {
                        unreachable!()
                    };
                    dependencies.require_display_helpers(
                        specialize(*receiver_type),
                        program,
                        semantics,
                        reachability,
                    );
                }
                wasm_ir::ExpressionKind::Call {
                    target, arguments, ..
                } if matches!(
                    reachability.resolved_call_target(owner.as_ref(), expression.id, target),
                    wasm_ir::CallTarget::Intrinsic { .. }
                ) =>
                {
                    let wasm_ir::CallTarget::Intrinsic {
                        item, intrinsic, ..
                    } = reachability.resolved_call_target(owner.as_ref(), expression.id, target)
                    else {
                        unreachable!()
                    };
                    dependencies.stdlib_items.insert(*item);
                    dependencies.require_intrinsic(*intrinsic);
                    for displayed in intrinsic_registry::contract(*intrinsic)
                        .dependency_roots
                        .iter()
                        .filter_map(|root| match root {
                            DependencyRoot::DisplayArgument(index) => {
                                arguments.get(usize::from(*index))
                            }
                            DependencyRoot::Helper(_) | DependencyRoot::HostImport(_) => None,
                        })
                    {
                        let ty = wasm_ir
                            .expression(*displayed)
                            .expect("intrinsic arguments belong to Wasm IR")
                            .ty;
                        dependencies.require_display_helpers(
                            specialize(ty),
                            program,
                            semantics,
                            reachability,
                        );
                    }
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
                        dependencies.require_display_helpers(
                            specialize(source),
                            program,
                            semantics,
                            reachability,
                        );
                    }
                }
                wasm_ir::ExpressionKind::Cast { value }
                    if matches!(
                        semantics.types().kind(specialize(expression.ty)),
                        TypeKind::Standard(StdlibTypeId::String)
                    ) =>
                {
                    let source = wasm_ir
                        .expression(*value)
                        .expect("cast operand belongs to Wasm IR")
                        .ty;
                    let source = specialize(source);
                    dependencies.require_display_helpers(source, program, semantics, reachability);
                }
                _ => {}
            }
        }

        if reachability.requires_string_equality() {
            dependencies.require(RuntimeHelperId::StringEquality);
        }

        if reachability.derived_debugs().next().is_some() {
            dependencies.require(RuntimeHelperId::JoinStrings);
            dependencies.require(RuntimeHelperId::IndentDisplay);
            dependencies.require(RuntimeHelperId::WrapDebugEntry);
            dependencies.require(RuntimeHelperId::WrapDebugVariant);
            dependencies.require(RuntimeHelperId::QuoteDebugString);
            for ty in reachability.derived_debugs() {
                dependencies.require_display_helpers(ty, program, semantics, reachability);
            }
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

    pub fn uses_float_format(&self) -> bool {
        self.uses_helper(RuntimeHelperId::FormatF32) || self.uses_helper(RuntimeHelperId::FormatF64)
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
                DependencyRoot::DisplayArgument(_) => {}
            }
        }
    }

    fn require_display_helpers(
        &mut self,
        ty: TypeId,
        program: &Program,
        semantics: &SemanticModel,
        reachability: &super::reachability::Reachability,
    ) {
        let mut pending = vec![ty];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            // A custom Display/Debug implementation is an opaque formatting
            // boundary. Its own reachable body contributes any helpers it
            // actually uses; deriving through the source type here would keep
            // an additional formatter that can never be called.
            if reachability.has_custom_formatting(ty) {
                continue;
            }
            match semantics.types().kind(ty) {
                TypeKind::Builtin(CoreTypeId::F32) => self.require(RuntimeHelperId::FormatF32),
                TypeKind::Builtin(CoreTypeId::F64) => self.require(RuntimeHelperId::FormatF64),
                TypeKind::Builtin(CoreTypeId::Char) => self.require(RuntimeHelperId::FormatChar),
                TypeKind::Builtin(
                    CoreTypeId::I8
                    | CoreTypeId::U8
                    | CoreTypeId::I16
                    | CoreTypeId::U16
                    | CoreTypeId::I32
                    | CoreTypeId::U32
                    | CoreTypeId::I64
                    | CoreTypeId::U64
                    | CoreTypeId::Address,
                ) => self.require(RuntimeHelperId::FormatI64),
                TypeKind::Record(record) => {
                    let declaration = &program.records[record.index()];
                    pending.extend(
                        declaration
                            .fields
                            .iter()
                            .filter_map(|field| semantics.record_field_type(field.id)),
                    );
                }
                TypeKind::Enum(enumeration) => {
                    let declaration = program
                        .enum_declaration(*enumeration)
                        .expect("reachable source enums retain their declaration");
                    pending.extend(
                        declaration
                            .variants
                            .iter()
                            .filter_map(|variant| semantics.enum_variant_payload(variant.id)),
                    );
                }
                TypeKind::Array { element, .. }
                | TypeKind::Option { value: element, .. }
                | TypeKind::Result { value: element, .. }
                | TypeKind::Set { element, .. }
                | TypeKind::Range { bound: element, .. } => pending.push(*element),
                TypeKind::Application { arguments, .. } => {
                    pending.extend(arguments.iter().copied())
                }
                TypeKind::Callable {
                    parameters, result, ..
                } => {
                    pending.extend(parameters.iter().copied());
                    pending.push(*result);
                }
                TypeKind::Async { value, .. } => pending.push(*value),
                TypeKind::Error
                | TypeKind::Builtin(_)
                | TypeKind::Standard(_)
                | TypeKind::StateSnapshot
                | TypeKind::SettingsView
                | TypeKind::ManagedClass(_)
                | TypeKind::ManagedReference(_)
                | TypeKind::GenericParameter { .. } => {}
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
