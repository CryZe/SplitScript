use super::*;

pub(super) struct GlobalPlan {
    pub section: GlobalSection,
    pub variables: HashMap<ValueId, u32>,
    pub variable_types: HashMap<ValueId, Type>,
    pub settings: HashMap<ValueId, SettingStorage>,
}

pub(super) fn encode(
    program: &Program,
    semantics: &SemanticModel,
    typed_hir: &TypedProgram,
    gc: &GcLayout,
    wasm_ir: &wasm_ir::Program,
) -> GlobalPlan {
    let mut section = GlobalSection::new();
    section.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(0),
    );
    let nullable_state = ValType::Ref(RefType {
        nullable: true,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    for _ in 0..2 {
        section.global(
            GlobalType {
                val_type: nullable_state,
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(STATE_TYPE)),
        );
    }
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    section.global(
        GlobalType {
            val_type: ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(async_frame_type_index()),
            }),
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(async_frame_type_index())),
    );
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
        if ty.is_enum() {
            section.global(
                global_type,
                &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
            );
        } else {
            section.global(global_type, &constant(variable.value.id, typed_hir, ty));
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
        emit_setting_global(&mut section, ty, gc);
        let old = section.len();
        emit_setting_global(&mut section, ty, gc);
        settings.insert(setting.id, SettingStorage { current, old, ty });
    }

    GlobalPlan {
        section,
        variables,
        variable_types,
        settings,
    }
}

fn emit_setting_global(section: &mut GlobalSection, ty: Type, gc: &GcLayout) {
    let global_type = GlobalType {
        val_type: gc.val_type(ty),
        mutable: true,
        shared: false,
    };
    match ty {
        Type::Bool => section.global(global_type, &ConstExpr::i32_const(0)),
        Type::Standard(StdlibTypeId::String) => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(standard_gc_type_index(
                StdlibTypeId::String,
            ))),
        ),
        ty if ty.is_enum() => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
        ),
        _ => unreachable!(),
    };
}
