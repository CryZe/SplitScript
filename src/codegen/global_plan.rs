use std::collections::HashMap;

use wasm_encoder::{ConstExpr, GlobalSection, GlobalType, HeapType, RefType, ValType};

use crate::{
    ast::{Program, ValueId},
    semantic::SemanticModel,
    stdlib::{
        CoreTypeId, RuntimeRepresentation, StandardLibrary, StateProviderAttachment, StdlibTypeId,
    },
    wasm_ir,
};

use super::{GcLayout, STATE_TYPE, Type, constant, semantic_type, value_type};

pub(super) struct GlobalPlan {
    pub section: GlobalSection,
    pub runtime: RuntimeGlobals,
    pub variables: HashMap<ValueId, u32>,
    pub variable_types: HashMap<ValueId, Type>,
    pub settings: HashMap<ValueId, SettingStorage>,
}

#[derive(Clone, Copy)]
pub(super) struct SettingStorage {
    pub current: u32,
    pub old: u32,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RuntimeGlobals {
    pub process: u32,
    /// Index of the source-declared process name that successfully attached.
    /// `-1` means no process is attached.
    pub process_name: u32,
    pub provider_value: Option<u32>,
    /// The typed enum value returned by `onAttach` for versioned state.
    pub selected_layout: Option<u32>,
    pub current: u32,
    pub old: u32,
    pub attach_ready: u32,
    pub async_frame: u32,
    pub detached_entered: u32,
}

pub(super) fn encode(
    program: &Program,
    semantics: &SemanticModel,
    gc: &GcLayout,
    wasm_ir: &wasm_ir::Program,
) -> GlobalPlan {
    let mut section = GlobalSection::new();
    let process = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(0),
    );
    let process_name = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(-1),
    );
    let provider_value = semantics.state_provider().and_then(|provider| {
        let ty = wasm_ir
            .standard_library()
            .state_provider(provider)
            .process_type;
        if wasm_ir
            .standard_library()
            .state_provider(provider)
            .attachment
            == StateProviderAttachment::Identity
        {
            return None;
        }
        let index = section.len();
        let initial = match wasm_ir.standard_library().type_decl(ty).representation {
            RuntimeRepresentation::Scalar {
                storage: CoreTypeId::I64,
            } => ConstExpr::i64_const(0),
            RuntimeRepresentation::GcStruct { .. }
            | RuntimeRepresentation::GcArray { .. }
            | RuntimeRepresentation::Enum { .. } => {
                ConstExpr::ref_null(HeapType::Concrete(gc.standard_index(ty)))
            }
            representation => {
                unreachable!("unsupported state-provider representation: {representation:?}")
            }
        };
        section.global(
            GlobalType {
                val_type: gc.val_type(Type::Standard(ty)),
                mutable: true,
                shared: false,
            },
            &initial,
        );
        Some(index)
    });
    let selected_layout = program
        .state
        .as_ref()
        .and_then(|state| state.layout_enum.as_ref())
        .map(|enumeration| {
            let selected = section.len();
            section.global(
                GlobalType {
                    val_type: gc.val_type(Type::Enum(enumeration.id)),
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(Type::Enum(enumeration.id)))),
            );
            selected
        });
    let nullable_state = ValType::Ref(RefType {
        nullable: true,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    let current = section.len();
    section.global(
        GlobalType {
            val_type: nullable_state,
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(STATE_TYPE)),
    );
    let old = section.len();
    section.global(
        GlobalType {
            val_type: nullable_state,
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(STATE_TYPE)),
    );
    let attach_ready = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let async_frame = section.len();
    section.global(
        GlobalType {
            val_type: ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(gc.async_frame_index()),
            }),
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(gc.async_frame_index())),
    );
    let detached_entered = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );

    let mut variables = HashMap::new();
    let mut variable_types = HashMap::new();
    if let Some(state) = &program.state
        && let (Some(value), Some(global), Some(enumeration)) = (
            state.layout_value,
            selected_layout,
            state.layout_enum.as_ref(),
        )
    {
        variables.insert(value, global);
        variable_types.insert(value, Type::Enum(enumeration.id));
    }
    for variable in program
        .globals
        .iter()
        .filter(|variable| wasm_ir.contains_global(variable.id))
    {
        let index = section.len();
        let ty = value_type(variable.id, semantics);
        let global_type = GlobalType {
            val_type: gc.val_type(ty),
            mutable: variable.mutable,
            shared: false,
        };
        if let Type::Option(option) = ty {
            section.global(
                global_type,
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(Type::Option(option)))),
            );
        } else if ty.is_enum(wasm_ir.standard_library()) {
            section.global(
                global_type,
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
            );
        } else {
            section.global(global_type, &constant(variable.value.id, wasm_ir, ty));
        }
        variables.insert(variable.id, index);
        variable_types.insert(variable.id, ty);
    }

    let mut settings = HashMap::new();
    for setting in &program.settings {
        let Some(ty) = semantics.value_type(setting.id) else {
            continue;
        };
        let ty = semantic_type(ty, semantics);
        let current = section.len();
        emit_setting_global(&mut section, ty, gc, wasm_ir.standard_library());
        let old = section.len();
        emit_setting_global(&mut section, ty, gc, wasm_ir.standard_library());
        settings.insert(setting.id, SettingStorage { current, old, ty });
    }

    GlobalPlan {
        section,
        runtime: RuntimeGlobals {
            process,
            process_name,
            provider_value,
            selected_layout,
            current,
            old,
            attach_ready,
            async_frame,
            detached_entered,
        },
        variables,
        variable_types,
        settings,
    }
}

fn emit_setting_global(
    section: &mut GlobalSection,
    ty: Type,
    gc: &GcLayout,
    standard_library: &StandardLibrary,
) {
    let global_type = GlobalType {
        val_type: gc.val_type(ty),
        mutable: true,
        shared: false,
    };
    match ty {
        Type::Bool => section.global(global_type, &ConstExpr::i32_const(0)),
        Type::Standard(StdlibTypeId::String) => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(gc.standard_index(StdlibTypeId::String))),
        ),
        ty if ty.is_enum(standard_library) => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
        ),
        _ => unreachable!(),
    };
}
