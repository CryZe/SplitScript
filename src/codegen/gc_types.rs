//! Deterministic WebAssembly GC type and layout planning.

use wasm_encoder::{
    ArrayType, CompositeInnerType, CompositeType, FieldType, StorageType, StructType, SubType,
    TypeSection, ValType,
};

use crate::{
    ast::{EnumDecl, Program},
    semantic::SemanticModel,
    stdlib::{DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibTypeId},
    types::{ResolvedArrayType, ResolvedAsyncType, ResolvedOptionType, ResolvedResultType},
};

use super::{
    GcLayout, Type, array_element_type,
    async_frame::{AsyncFrameLayout, AsyncFrameLayouts},
    enum_variant_payload, option_value_type, reachability, record_field_type, result_value_type,
    standard_field_type, value_type,
};

pub(super) struct EncodedTypes {
    pub section: TypeSection,
    pub next_type_index: u32,
    pub layout: GcLayout,
}

pub(super) struct Inputs<'a> {
    pub standard_library: &'a StandardLibrary,
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub async_layout: Option<&'a AsyncFrameLayout>,
    pub async_frames: &'a AsyncFrameLayouts,
    pub enums: &'a [EnumDecl],
    pub array_types: &'a [ResolvedArrayType],
    pub option_types: &'a [ResolvedOptionType],
    pub result_types: &'a [ResolvedResultType],
    pub async_types: &'a [ResolvedAsyncType],
    pub reachability: &'a reachability::Reachability,
}

pub(super) fn encode(inputs: Inputs<'_>) -> EncodedTypes {
    let Inputs {
        standard_library,
        program,
        semantics,
        async_layout,
        async_frames,
        enums,
        array_types,
        option_types,
        result_types,
        async_types,
        reachability,
    } = inputs;
    let layout = GcLayout::plan(super::gc_layout::Inputs {
        standard_library: standard_library.clone(),
        program,
        enums,
        arrays: array_types,
        options: option_types,
        results: result_types,
        asyncs: async_types,
        async_frames,
        reachability,
    });
    let state = program
        .state
        .as_ref()
        .expect("checked programs have a state block");
    let mut recursive_types = vec![SubType {
        is_final: true,
        supertype_idx: None,
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: state
                    .canonical_fields()
                    .iter()
                    .map(|field| FieldType {
                        element_type: layout.storage_type(value_type(field.id, semantics)),
                        mutable: true,
                    })
                    .collect(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }];
    for declaration in standard_library.types() {
        let inner = match declaration.representation {
            RuntimeRepresentation::Scalar { .. } => continue,
            RuntimeRepresentation::GcArray {
                element, mutable, ..
            } => CompositeInnerType::Array(ArrayType(FieldType {
                element_type: layout
                    .storage_type(Type::from_declared(DeclaredTypeRef::Core(element))),
                mutable,
            })),
            RuntimeRepresentation::GcStruct { .. } => CompositeInnerType::Struct(StructType {
                fields: standard_library
                    .fields_of(declaration.id)
                    .map(|field| FieldType {
                        element_type: layout.storage_type(standard_field_type(field.id, semantics)),
                        mutable: false,
                    })
                    .collect(),
            }),
            RuntimeRepresentation::Enum { .. } => CompositeInnerType::Struct(StructType {
                fields: std::iter::once(FieldType {
                    element_type: StorageType::Val(ValType::I32),
                    mutable: false,
                })
                .chain(
                    standard_library
                        .variants_of(declaration.id)
                        .map(|_| FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        }),
                )
                .collect(),
            }),
        };
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner,
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    let mut fields = Vec::with_capacity(
        async_layout
            .as_ref()
            .map_or(1, |layout| layout.types.len() + 1),
    );
    fields.push(FieldType {
        element_type: StorageType::Val(ValType::I32),
        mutable: true,
    });
    if let Some(frame_layout) = &async_layout {
        fields.extend(frame_layout.types.iter().map(|ty| FieldType {
            element_type: layout.storage_type(*ty),
            mutable: true,
        }));
    }
    recursive_types.push(SubType {
        is_final: true,
        supertype_idx: None,
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: fields.into(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    });
    for ty in layout.dynamic_types() {
        let (inner, is_final, supertype_idx) = match ty {
            Type::Record(id) => {
                let record = program
                    .records
                    .iter()
                    .find(|record| record.id == id)
                    .expect("reachable record layouts have declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: record
                            .fields
                            .iter()
                            .map(|field| FieldType {
                                element_type: layout
                                    .storage_type(record_field_type(field.id, semantics)),
                                mutable: false,
                            })
                            .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::Enum(id) => {
                let enumeration = enums
                    .iter()
                    .find(|enumeration| enumeration.id == id)
                    .expect("reachable enum layouts have declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: std::iter::once(FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        })
                        .chain(enumeration.variants.iter().map(|variant| {
                            FieldType {
                                element_type: enum_variant_payload(variant.id, semantics)
                                    .map_or(StorageType::Val(ValType::I32), |ty| {
                                        layout.storage_type(ty)
                                    }),
                                mutable: false,
                            }
                        }))
                        .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::Array(id) => {
                let declaration = array_types
                    .iter()
                    .find(|array| array.id == id)
                    .expect("reachable arrays have resolved declarations");
                let supertype_idx = declaration.length.and_then(|_| {
                    array_types
                        .iter()
                        .find(|array| {
                            array.length.is_none() && array.element == declaration.element
                        })
                        .map(|array| layout.index(Type::Array(array.id)))
                });
                (
                    CompositeInnerType::Array(ArrayType(FieldType {
                        element_type: layout.storage_type(array_element_type(id, semantics)),
                        mutable: true,
                    })),
                    declaration.length.is_some(),
                    supertype_idx,
                )
            }
            Type::Option(id) => (
                CompositeInnerType::Struct(StructType {
                    fields: vec![FieldType {
                        element_type: layout.storage_type(option_value_type(id, semantics)),
                        mutable: false,
                    }]
                    .into(),
                }),
                true,
                None,
            ),
            Type::Result(id) => (
                CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: layout.storage_type(result_value_type(id, semantics)),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(
                                layout.val_type(Type::Standard(StdlibTypeId::String)),
                            ),
                            mutable: false,
                        },
                    ]
                    .into(),
                }),
                true,
                None,
            ),
            _ => unreachable!("only dynamic GC types are ordered by GcLayout"),
        };
        recursive_types.push(SubType {
            is_final,
            supertype_idx,
            composite_type: CompositeType {
                inner,
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for future in async_types
        .iter()
        .filter(|future| reachability.contains_async_type(future.id))
    {
        debug_assert_eq!(
            layout.index(Type::Async(future.id)),
            recursive_types.len() as u32
        );
        recursive_types.push(SubType {
            is_final: false,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                    ]
                    .into(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for (instance, frame) in async_frames.functions() {
        let result = semantics.specialize_type(
            instance,
            semantics
                .function_result(instance.function)
                .expect("checked functions have result types"),
        );
        let Type::Async(future) = super::semantic_type(result, semantics) else {
            unreachable!("suspending functions return async values")
        };
        let frame_index = layout.function_frame_index(instance);
        debug_assert_eq!(frame_index, recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: Some(layout.index(Type::Async(future))),
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                    ]
                    .into_iter()
                    .chain(frame.types.iter().map(|ty| FieldType {
                        element_type: layout.storage_type(*ty),
                        mutable: true,
                    }))
                    .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for (instance, frame) in async_frames.intrinsics() {
        let Type::Async(future) = frame.future else {
            unreachable!("intrinsic future layouts have async value types")
        };
        let frame_index = layout.intrinsic_frame_index(instance);
        debug_assert_eq!(frame_index, recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: Some(layout.index(Type::Async(future))),
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                    ]
                    .into_iter()
                    .chain(frame.types.iter().map(|ty| FieldType {
                        element_type: layout.storage_type(*ty),
                        mutable: true,
                    }))
                    .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    let mut types = TypeSection::new();
    types.ty().rec(recursive_types);

    // `TypeSection::len` counts encoded entries, while a recursive group can
    // contain multiple indexed subtypes. State, Duration, String, the attach
    // Module, the attach continuation frame, and then user records occupy the
    // first indices.
    EncodedTypes {
        section: types,
        next_type_index: layout.type_count,
        layout,
    }
}
