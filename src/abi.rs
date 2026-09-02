//! Declarative WebAssembly host ABI used by the generated runtime.
//!
//! This catalog is backend infrastructure. Source-level documentation and
//! editor tooling should expose [`crate::stdlib`] instead.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiType {
    I32,
    I64,
    F64,
}

impl AbiType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F64 => "f64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiOwnership {
    Value,
    BorrowedHandle,
    OwnedHandle,
    InputMemory,
    OutputMemory,
    InputOutputMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiValue {
    pub name: &'static str,
    pub ty: AbiType,
    pub ownership: AbiOwnership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbiEffect {
    ReadsTimer,
    ReadsRuntime,
    ReadsFileSystem,
    WritesTimer,
    WritesRuntime,
    ManagesProcess,
    ReadsProcess,
    RegistersSettings,
    ReadsSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiImport {
    pub id: AbiImportId,
    pub module: &'static str,
    pub name: &'static str,
    pub parameters: &'static [AbiValue],
    pub results: &'static [AbiValue],
    pub effects: &'static [AbiEffect],
    pub lifetime: &'static str,
    pub summary: &'static str,
}

const fn value(name: &'static str, ty: AbiType) -> AbiValue {
    AbiValue {
        name,
        ty,
        ownership: AbiOwnership::Value,
    }
}

const fn borrowed(name: &'static str, ty: AbiType) -> AbiValue {
    AbiValue {
        name,
        ty,
        ownership: AbiOwnership::BorrowedHandle,
    }
}

const fn owned(name: &'static str, ty: AbiType) -> AbiValue {
    AbiValue {
        name,
        ty,
        ownership: AbiOwnership::OwnedHandle,
    }
}

const fn input(name: &'static str) -> AbiValue {
    AbiValue {
        name,
        ty: AbiType::I32,
        ownership: AbiOwnership::InputMemory,
    }
}

const fn output(name: &'static str) -> AbiValue {
    AbiValue {
        name,
        ty: AbiType::I32,
        ownership: AbiOwnership::OutputMemory,
    }
}

const fn input_output(name: &'static str) -> AbiValue {
    AbiValue {
        name,
        ty: AbiType::I32,
        ownership: AbiOwnership::InputOutputMemory,
    }
}

const TIMER_READ: &[AbiEffect] = &[AbiEffect::ReadsTimer];
const RUNTIME_READ: &[AbiEffect] = &[AbiEffect::ReadsRuntime];
const FILESYSTEM_READ: &[AbiEffect] = &[AbiEffect::ReadsFileSystem];
const TIMER_WRITE: &[AbiEffect] = &[AbiEffect::WritesTimer];
const RUNTIME_WRITE: &[AbiEffect] = &[AbiEffect::WritesRuntime];
const PROCESS_MANAGEMENT: &[AbiEffect] = &[AbiEffect::ManagesProcess];
const PROCESS_READ: &[AbiEffect] = &[AbiEffect::ReadsProcess];
const SETTINGS_REGISTRATION: &[AbiEffect] = &[AbiEffect::RegistersSettings];
const SETTINGS_READ: &[AbiEffect] = &[AbiEffect::ReadsSettings];

macro_rules! import {
    ($id:ident, $(in $module:literal,)? $name:literal, $params:expr, $results:expr, $effects:expr, $lifetime:literal, $summary:literal) => {
        AbiImport {
            id: AbiImportId::$id,
            module: import!(@module $($module)?),
            name: $name,
            parameters: $params,
            results: $results,
            effects: $effects,
            lifetime: $lifetime,
            summary: $summary,
        }
    };
    (@module $module:literal) => {
        $module
    };
    (@module) => {
        "env"
    };
}

macro_rules! abi_catalog {
    ($(
        import!(
            $id:ident,
            $(in $module:literal,)?
            $name:literal,
            $parameters:expr,
            $results:expr,
            $effects:expr,
            $lifetime:literal,
            $summary:literal
        )
    ),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(usize)]
        pub enum AbiImportId {
            $($id),*
        }

        impl AbiImportId {
            pub const ALL: &'static [Self] = &[$(Self::$id),*];
            pub const COUNT: usize = Self::ALL.len();

            pub const fn index(self) -> usize {
                self as usize
            }
        }

        const IMPORTS: &[AbiImport] = &[$(
            import!(
                $id,
                $(in $module,)?
                $name,
                $parameters,
                $results,
                $effects,
                $lifetime,
                $summary
            )
        ),*];
    };
}

abi_catalog! {
    import!(
        TimerGetState,
        "timer_get_state",
        &[],
        &[value("state", AbiType::I32)],
        TIMER_READ,
        "Returns a plain timer-state value.",
        "Reads the current timer state."
    ),
    import!(
        TimerCurrentSplitIndex,
        "timer_current_split_index",
        &[],
        &[value("index", AbiType::I64)],
        TIMER_READ,
        "Returns -1 outside an attempt and the segment count after the final split.",
        "Reads the index of the split the current attempt is on."
    ),
    import!(
        TimerSegmentWasSplit,
        "timer_segment_splitted",
        &[value("index", AbiType::I64)],
        &[value("was_split", AbiType::I32)],
        TIMER_READ,
        "Returns -1 when the segment has not been reached, 0 when skipped, and 1 when split.",
        "Reads how an earlier segment was advanced during the current attempt."
    ),
    import!(
        TimerStart,
        "timer_start",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Starts the timer."
    ),
    import!(
        TimerSplit,
        "timer_split",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Advances the timer to the next split."
    ),
    import!(
        TimerSkipSplit,
        "timer_skip_split",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Skips the current split."
    ),
    import!(
        TimerUndoSplit,
        "timer_undo_split",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Undoes the previous split."
    ),
    import!(
        TimerReset,
        "timer_reset",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Resets the timer."
    ),
    import!(
        TimerSetGameTime,
        "timer_set_game_time",
        &[
            value("seconds", AbiType::I64),
            value("nanoseconds", AbiType::I32)
        ],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Sets the displayed game time."
    ),
    import!(
        TimerPauseGameTime,
        "timer_pause_game_time",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Pauses automatic game-time progression."
    ),
    import!(
        TimerResumeGameTime,
        "timer_resume_game_time",
        &[],
        &[],
        TIMER_WRITE,
        "Retains no guest values.",
        "Resumes automatic game-time progression."
    ),
    import!(
        TimerSetVariable,
        "timer_set_variable",
        &[
            input("name_pointer"),
            value("name_length", AbiType::I32),
            input("value_pointer"),
            value("value_length", AbiType::I32)
        ],
        &[],
        TIMER_WRITE,
        "String byte ranges are borrowed for this call only.",
        "Sets a LiveSplit custom variable."
    ),
    import!(
        RuntimeSetTickRate,
        "runtime_set_tick_rate",
        &[value("ticks_per_second", AbiType::F64)],
        &[],
        RUNTIME_WRITE,
        "Retains no guest values.",
        "Changes the runtime update frequency."
    ),
    import!(
        WasiClockTimeGet,
        in "wasi_snapshot_preview1",
        "clock_time_get",
        &[
            value("clock_id", AbiType::I32),
            value("precision", AbiType::I64),
            output("timestamp_pointer")
        ],
        &[value("errno", AbiType::I32)],
        RUNTIME_READ,
        "Writes one unsigned 64-bit monotonic timestamp to guest memory.",
        "Reads the WASI monotonic clock in nanoseconds."
    ),
    import!(
        WasiFdPrestatGet,
        in "wasi_snapshot_preview1",
        "fd_prestat_get",
        &[value("descriptor", AbiType::I32), output("prestat_pointer")],
        &[value("errno", AbiType::I32)],
        FILESYSTEM_READ,
        "Writes one WASI preopen descriptor record to guest memory.",
        "Inspects a preopened filesystem directory."
    ),
    import!(
        WasiFdPrestatDirName,
        in "wasi_snapshot_preview1",
        "fd_prestat_dir_name",
        &[
            value("descriptor", AbiType::I32),
            output("path_pointer"),
            value("path_length", AbiType::I32)
        ],
        &[value("errno", AbiType::I32)],
        FILESYSTEM_READ,
        "Writes exactly path_length preopen-name bytes to guest memory.",
        "Reads the portable path of a preopened filesystem directory."
    ),
    import!(
        WasiPathOpen,
        in "wasi_snapshot_preview1",
        "path_open",
        &[
            value("directory_descriptor", AbiType::I32),
            value("lookup_flags", AbiType::I32),
            input("path_pointer"),
            value("path_length", AbiType::I32),
            value("open_flags", AbiType::I32),
            value("rights_base", AbiType::I64),
            value("rights_inheriting", AbiType::I64),
            value("descriptor_flags", AbiType::I32),
            output("opened_descriptor_pointer")
        ],
        &[value("errno", AbiType::I32)],
        FILESYSTEM_READ,
        "The path is borrowed for this call; a successful descriptor is owned until fd_close.",
        "Opens a file read-only beneath a WASI preopened directory."
    ),
    import!(
        WasiFdRead,
        in "wasi_snapshot_preview1",
        "fd_read",
        &[
            value("descriptor", AbiType::I32),
            input("iovec_pointer"),
            value("iovec_count", AbiType::I32),
            output("bytes_read_pointer")
        ],
        &[value("errno", AbiType::I32)],
        FILESYSTEM_READ,
        "The descriptor is borrowed; the iovec destinations are written for this call only.",
        "Reads bytes from an open WASI file descriptor."
    ),
    import!(
        WasiFdClose,
        in "wasi_snapshot_preview1",
        "fd_close",
        &[owned("descriptor", AbiType::I32)],
        &[value("errno", AbiType::I32)],
        FILESYSTEM_READ,
        "Consumes the owned file descriptor.",
        "Closes a WASI file descriptor."
    ),
    import!(
        ProcessAttach,
        "process_attach",
        &[input("name_pointer"), value("name_length", AbiType::I32)],
        &[owned("process", AbiType::I64)],
        PROCESS_MANAGEMENT,
        "The returned nonzero handle is owned by the guest until process_detach.",
        "Attempts to attach to a named process."
    ),
    import!(
        ProcessAttachByPid,
        "process_attach_by_pid",
        &[value("process_id", AbiType::I64)],
        &[owned("process", AbiType::I64)],
        PROCESS_MANAGEMENT,
        "The returned nonzero handle is owned by the guest until process_detach.",
        "Attempts to attach to a process by its process ID."
    ),
    import!(
        ProcessListByName,
        "process_list_by_name",
        &[
            input("name_pointer"),
            value("name_length", AbiType::I32),
            output("process_ids_pointer"),
            input_output("process_ids_length_pointer")
        ],
        &[value("success", AbiType::I32)],
        PROCESS_MANAGEMENT,
        "The name and capacity are borrowed for the call. The host writes up to the input capacity and replaces the length with the total candidate count; candidate order is unspecified.",
        "Lists process IDs whose process name matches."
    ),
    import!(
        ProcessDetach,
        "process_detach",
        &[borrowed("process", AbiType::I64)],
        &[],
        PROCESS_MANAGEMENT,
        "Consumes the guest's use of the process handle.",
        "Detaches a process handle."
    ),
    import!(
        ProcessIsOpen,
        "process_is_open",
        &[borrowed("process", AbiType::I64)],
        &[value("is_open", AbiType::I32)],
        PROCESS_MANAGEMENT,
        "The process handle is borrowed for this call.",
        "Tests whether an attached process remains open."
    ),
    import!(
        ProcessRead,
        "process_read",
        &[
            borrowed("process", AbiType::I64),
            value("address", AbiType::I64),
            output("output_pointer"),
            value("length", AbiType::I32)
        ],
        &[value("success", AbiType::I32)],
        PROCESS_READ,
        "The process handle and output range are borrowed for this call only.",
        "Copies target-process memory into guest memory."
    ),
    import!(
        ProcessGetModuleAddress,
        "process_get_module_address",
        &[
            borrowed("process", AbiType::I64),
            input("name_pointer"),
            value("name_length", AbiType::I32)
        ],
        &[value("address", AbiType::I64)],
        PROCESS_READ,
        "The handle and module-name bytes are borrowed for this call.",
        "Finds a module base address."
    ),
    import!(
        ProcessGetModuleSize,
        "process_get_module_size",
        &[
            borrowed("process", AbiType::I64),
            input("name_pointer"),
            value("name_length", AbiType::I32)
        ],
        &[value("size", AbiType::I64)],
        PROCESS_READ,
        "The handle and module-name bytes are borrowed for this call.",
        "Finds a module image size."
    ),
    import!(
        ProcessGetModulePath,
        "process_get_module_path",
        &[
            borrowed("process", AbiType::I64),
            input("name_pointer"),
            value("name_length", AbiType::I32),
            output("path_pointer"),
            output("path_length_pointer")
        ],
        &[value("success", AbiType::I32)],
        PROCESS_READ,
        "The process handle and module-name bytes are borrowed; the path and length buffers are written only for this call.",
        "Returns a host-provided portable filesystem path for a module."
    ),
    import!(
        ProcessGetPath,
        "process_get_path",
        &[
            borrowed("process", AbiType::I64),
            output("path_pointer"),
            output("path_length_pointer")
        ],
        &[value("success", AbiType::I32)],
        PROCESS_READ,
        "The process handle is borrowed; the path and length buffers are written only for this call.",
        "Returns a host-provided portable filesystem path for the executable."
    ),
    import!(
        ProcessGetMemoryRangeCount,
        "process_get_memory_range_count",
        &[borrowed("process", AbiType::I64)],
        &[value("count", AbiType::I64)],
        PROCESS_READ,
        "The process handle is borrowed for this call.",
        "Returns the number of mapped process-memory ranges."
    ),
    import!(
        ProcessGetMemoryRangeAddress,
        "process_get_memory_range_address",
        &[
            borrowed("process", AbiType::I64),
            value("index", AbiType::I64)
        ],
        &[value("address", AbiType::I64)],
        PROCESS_READ,
        "The process handle is borrowed for this call.",
        "Returns a mapped memory range's base address."
    ),
    import!(
        ProcessGetMemoryRangeSize,
        "process_get_memory_range_size",
        &[
            borrowed("process", AbiType::I64),
            value("index", AbiType::I64)
        ],
        &[value("size", AbiType::I64)],
        PROCESS_READ,
        "The process handle is borrowed for this call.",
        "Returns a mapped memory range's size."
    ),
    import!(
        ProcessGetMemoryRangeFlags,
        "process_get_memory_range_flags",
        &[
            borrowed("process", AbiType::I64),
            value("index", AbiType::I64)
        ],
        &[value("flags", AbiType::I64)],
        PROCESS_READ,
        "The process handle is borrowed for this call.",
        "Returns a mapped memory range's access flags."
    ),
    import!(
        RuntimePrintMessage,
        "runtime_print_message",
        &[
            input("message_pointer"),
            value("message_length", AbiType::I32)
        ],
        &[],
        RUNTIME_WRITE,
        "Message bytes are borrowed for this call only.",
        "Writes a diagnostic message."
    ),
    import!(
        RuntimeGetOs,
        "runtime_get_os",
        &[output("name_pointer"), output("name_length_pointer")],
        &[value("success", AbiType::I32)],
        RUNTIME_READ,
        "The name and length buffers are written only for this call.",
        "Returns the host operating-system name."
    ),
    import!(
        RuntimeGetArch,
        "runtime_get_arch",
        &[output("name_pointer"), output("name_length_pointer")],
        &[value("success", AbiType::I32)],
        RUNTIME_READ,
        "The name and length buffers are written only for this call.",
        "Returns the host architecture name."
    ),
    import!(
        UserSettingsAddBool,
        "user_settings_add_bool",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32),
            value("default", AbiType::I32)
        ],
        &[value("accepted", AbiType::I32)],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Registers a boolean user setting."
    ),
    import!(
        UserSettingsAddTitle,
        "user_settings_add_title",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32),
            value("heading_level", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Registers a settings heading."
    ),
    import!(
        UserSettingsAddChoice,
        "user_settings_add_choice",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32),
            input("default_pointer"),
            value("default_length", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Registers an enum-backed choice setting."
    ),
    import!(
        UserSettingsAddChoiceOption,
        "user_settings_add_choice_option",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("option_pointer"),
            value("option_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32)
        ],
        &[value("accepted", AbiType::I32)],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Registers one choice option."
    ),
    import!(
        UserSettingsAddFileSelect,
        "user_settings_add_file_select",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Registers a file-selection setting."
    ),
    import!(
        UserSettingsAddFileSelectNameFilter,
        "user_settings_add_file_select_name_filter",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("description_pointer"),
            value("description_length", AbiType::I32),
            input("pattern_pointer"),
            value("pattern_length", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Adds a named glob filter to a file setting."
    ),
    import!(
        UserSettingsAddFileSelectMimeFilter,
        "user_settings_add_file_select_mime_filter",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("mime_pointer"),
            value("mime_length", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Adds a MIME filter to a file setting."
    ),
    import!(
        UserSettingsSetTooltip,
        "user_settings_set_tooltip",
        &[
            input("key_pointer"),
            value("key_length", AbiType::I32),
            input("tooltip_pointer"),
            value("tooltip_length", AbiType::I32)
        ],
        &[],
        SETTINGS_REGISTRATION,
        "String byte ranges are borrowed for this call only.",
        "Sets a setting or heading tooltip."
    ),
    import!(
        SettingsMapLoad,
        "settings_map_load",
        &[],
        &[owned("settings_map", AbiType::I64)],
        SETTINGS_READ,
        "The returned handle is owned by the guest until settings_map_free.",
        "Loads the current settings snapshot."
    ),
    import!(
        SettingsMapFree,
        "settings_map_free",
        &[borrowed("settings_map", AbiType::I64)],
        &[],
        SETTINGS_READ,
        "Consumes the guest's use of the settings-map handle.",
        "Frees a settings snapshot handle."
    ),
    import!(
        SettingsMapGet,
        "settings_map_get",
        &[
            borrowed("settings_map", AbiType::I64),
            input("key_pointer"),
            value("key_length", AbiType::I32)
        ],
        &[owned("setting_value", AbiType::I64)],
        SETTINGS_READ,
        "A nonzero returned handle is owned by the guest until setting_value_free.",
        "Looks up one setting value."
    ),
    import!(
        SettingValueFree,
        "setting_value_free",
        &[borrowed("setting_value", AbiType::I64)],
        &[],
        SETTINGS_READ,
        "Consumes the guest's use of the setting-value handle.",
        "Frees a setting value handle."
    ),
    import!(
        SettingValueGetBool,
        "setting_value_get_bool",
        &[
            borrowed("setting_value", AbiType::I64),
            output("output_pointer")
        ],
        &[value("success", AbiType::I32)],
        SETTINGS_READ,
        "The handle and output location are borrowed for this call.",
        "Decodes a boolean setting value."
    ),
    import!(
        SettingValueGetString,
        "setting_value_get_string",
        &[
            borrowed("setting_value", AbiType::I64),
            output("output_pointer"),
            value("capacity", AbiType::I32)
        ],
        &[value("length", AbiType::I32)],
        SETTINGS_READ,
        "The handle and output range are borrowed for this call.",
        "Decodes a UTF-8 setting value."
    ),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AbiCatalog;

impl AbiCatalog {
    pub const fn new() -> Self {
        Self
    }

    pub fn imports(self) -> impl ExactSizeIterator<Item = &'static AbiImport> {
        IMPORTS.iter()
    }

    pub fn import(self, id: AbiImportId) -> &'static AbiImport {
        &IMPORTS[id.index()]
    }

    pub fn import_by_name(self, name: &str) -> Option<&'static AbiImport> {
        IMPORTS.iter().find(|import| import.name == name)
    }

    pub fn render_signature(self, id: AbiImportId) -> String {
        let import = self.import(id);
        let mut rendered = "(".to_owned();
        for (index, parameter) in import.parameters.iter().enumerate() {
            if index != 0 {
                rendered.push_str(", ");
            }
            rendered.push_str(parameter.ty.name());
        }
        rendered.push_str(") -> ");
        match import.results {
            [] => rendered.push_str("()"),
            [result] => rendered.push_str(result.ty.name()),
            results => {
                rendered.push('(');
                for (index, result) in results.iter().enumerate() {
                    if index != 0 {
                        rendered.push_str(", ");
                    }
                    rendered.push_str(result.ty.name());
                }
                rendered.push(')');
            }
        }
        rendered
    }

    pub fn render_import_table(self) -> String {
        let mut rendered = String::from("| Import | WebAssembly type |\n| --- | --- |\n");
        for import in IMPORTS {
            rendered.push_str("| `");
            rendered.push_str(import.name);
            rendered.push_str("` | `");
            rendered.push_str(&self.render_signature(import.id));
            rendered.push_str("` |\n");
        }
        rendered
    }

    pub fn validate(self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for (index, import) in IMPORTS.iter().enumerate() {
            if import.id.index() != index {
                errors.push(format!(
                    "ABI import `{:?}` is out of stable order",
                    import.id
                ));
            }
            if !ids.insert(import.id) {
                errors.push(format!("duplicate ABI import ID `{:?}`", import.id));
            }
            if !names.insert((import.module, import.name)) {
                errors.push(format!(
                    "duplicate ABI import `{}.{}`",
                    import.module, import.name
                ));
            }
            if import.module.trim().is_empty()
                || import.name.trim().is_empty()
                || import.summary.trim().is_empty()
                || import.lifetime.trim().is_empty()
            {
                errors.push(format!(
                    "ABI import `{:?}` has incomplete metadata",
                    import.id
                ));
            }
            let mut parameter_names = HashSet::new();
            for parameter in import.parameters {
                if !parameter_names.insert(parameter.name) {
                    errors.push(format!(
                        "ABI import `{}` has duplicate parameter `{}`",
                        import.name, parameter.name
                    ));
                }
            }
            let mut result_names = HashSet::new();
            for result in import.results {
                if !result_names.insert(result.name) {
                    errors.push(format!(
                        "ABI import `{}` has duplicate result `{}`",
                        import.name, result.name
                    ));
                }
            }
            if import.effects.is_empty() {
                errors.push(format!(
                    "ABI import `{}` has no declared effects",
                    import.name
                ));
            }
        }
        if IMPORTS.len() != AbiImportId::COUNT {
            errors.push(format!(
                "ABI catalog has {} imports but {} IDs",
                IMPORTS.len(),
                AbiImportId::COUNT
            ));
        }
        errors
    }
}
