use std::collections::HashMap;

use wasm_encoder::{ConstExpr, GlobalSection, GlobalType, HeapType, RefType, ValType};

use crate::{
    ast::{Program, ValueId},
    semantic::{FunctionInstance, SemanticModel},
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
    /// Pending source-defined state-provider attachment future.
    pub provider_attachment_frame: Option<u32>,
    /// Opaque provider-specific context produced before user attachment code.
    pub provider_preparation_value: Option<u32>,
    /// Pending source-defined state-provider preparation future.
    pub provider_preparation_frame: Option<u32>,
    /// Whether preparation completed for the current process attachment.
    pub provider_prepared: Option<u32>,
    /// The typed enum value returned by `onAttach` for versioned state.
    pub selected_layout: Option<u32>,
    pub current: u32,
    pub old: u32,
    pub attach_ready: u32,
    /// Whether this attachment has committed at least one complete snapshot.
    pub state_ready: u32,
    /// Current compiler-derived structural formatting depth. This bounds
    /// recursive container graphs without allocating traversal state.
    pub debug_depth: u32,
    pub async_frame: u32,
}

pub(super) fn encode(
    program: &Program,
    semantics: &SemanticModel,
    gc: &GcLayout,
    wasm_ir: &wasm_ir::Program,
    provider_attachment: Option<&FunctionInstance>,
    provider_preparation: Option<&FunctionInstance>,
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
    let provider_attachment_frame = provider_attachment.map(|attachment| {
        let index = section.len();
        let frame_type = gc.function_frame_index(attachment);
        section.global(
            GlobalType {
                val_type: ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(frame_type),
                }),
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(frame_type)),
        );
        index
    });
    let provider_preparation_value = provider_preparation.map(|preparation| {
        let completion = semantic_type(
            semantics.specialize_type(
                preparation,
                semantics
                    .function_completion(preparation.function)
                    .expect("checked provider preparation has a completion type"),
            ),
            semantics,
        );
        let index = section.len();
        section.global(
            GlobalType {
                val_type: gc.val_type(completion),
                mutable: true,
                shared: false,
            },
            &default_const_expr(gc.val_type(completion)),
        );
        index
    });
    let provider_preparation_frame = provider_preparation.map(|preparation| {
        let index = section.len();
        let frame_type = gc.function_frame_index(preparation);
        section.global(
            GlobalType {
                val_type: ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(frame_type),
                }),
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(frame_type)),
        );
        index
    });
    let provider_prepared = provider_preparation.map(|_| {
        let index = section.len();
        section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        index
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
    let state_ready = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let debug_depth = section.len();
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
        let ty = value_type(variable.id, semantics);
        if !ty.has_runtime_value() {
            variables.insert(variable.id, u32::MAX);
            variable_types.insert(variable.id, ty);
            continue;
        }
        let index = section.len();
        let mut val_type = gc.val_type(ty);
        if variable.value.is_none()
            && let ValType::Ref(reference) = &mut val_type
        {
            // The source value is non-null after successful initialization,
            // but detached storage needs a null sentinel so it can release
            // the previous attachment's GC graph.
            reference.nullable = true;
        }
        let global_type = GlobalType {
            val_type,
            mutable: variable.mutable,
            shared: false,
        };
        if let Type::Option(option) = ty {
            section.global(
                global_type,
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(Type::Option(option)))),
            );
        } else if ty.is_enum(wasm_ir.standard_library())
            || matches!(
                ty,
                Type::Record(_)
                    | Type::Array(_)
                    | Type::Range(_)
                    | Type::Set(_)
                    | Type::Standard(StdlibTypeId::String)
            )
        {
            section.global(
                global_type,
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
            );
        } else if let Some(value) = &variable.value {
            section.global(global_type, &constant(value.id, wasm_ir, ty));
        } else {
            section.global(global_type, &default_const_expr(gc.val_type(ty)));
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
            provider_attachment_frame,
            provider_preparation_value,
            provider_preparation_frame,
            provider_prepared,
            selected_layout,
            current,
            old,
            attach_ready,
            state_ready,
            debug_depth,
            async_frame,
        },
        variables,
        variable_types,
        settings,
    }
}

fn default_const_expr(ty: ValType) -> ConstExpr {
    match ty {
        ValType::I32 => ConstExpr::i32_const(0),
        ValType::I64 => ConstExpr::i64_const(0),
        ValType::F32 => ConstExpr::f32_const(0.0.into()),
        ValType::F64 => ConstExpr::f64_const(0.0.into()),
        ValType::Ref(reference) => ConstExpr::ref_null(reference.heap_type),
        ValType::V128 => unreachable!("SplitScript has no v128 source values"),
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
