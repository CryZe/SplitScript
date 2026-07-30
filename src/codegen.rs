use std::collections::HashMap;

use wasm_encoder::{
    ArrayType, BlockType, CodeSection, CompositeInnerType, CompositeType, ConstExpr, FieldType,
    Function, FunctionSection, GlobalSection, GlobalType, HeapType, Instruction, MemArg, RefType,
    StorageType, StructType, SubType, TypeSection, ValType,
};

use crate::abi::AbiImportId;
use crate::ast::{
    Action, ActionKind, ArrayTypeDecl, ArrayTypeId, BinaryOp, EnumDecl, EnumId, EnumTypeId,
    EnumVariantId, ExprId, FunctionDecl, FunctionId, OptionTypeDecl, OptionTypeId, Program,
    RecordDecl, RecordFieldId, RecordId, ResultTypeDecl, ResultTypeId, SettingFileFilter,
    SettingKind, StateField, StateSource, SuspensionMode, UnaryOp, ValueId,
};
use crate::equality::EqualityCapabilities;
use crate::hir::{TypedExpressionKind, TypedProgram};
use crate::memory::{MemoryLayouts, MemoryTypeLayout};
use crate::semantic::{
    ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue, SemanticModel,
    ValueConversionKind,
};
use crate::stdlib::{
    CoreTypeId, DeclaredTypeRef, Implementation, IntrinsicId, RuntimeRepresentation,
    StandardLibrary, StdlibFieldId, StdlibTypeId,
};
use crate::types::{BuiltinType, TypeId, TypeKind};
use crate::wasm_ir::{self, BodyOwner, LocalPurpose};

mod async_state;
mod data_plan;
mod dependencies;
mod expression;
mod function_plan;
mod gc_types;
mod global_plan;
mod imports;
mod module_assembly;
mod reachability;
mod runtime_helpers;
mod script_functions;
mod settings;
mod update;

use self::async_state::compile_async_attach;
use self::data_plan::{
    IL2CPP_ASSEMBLIES_SIGNATURE, IL2CPP_LEA_SIGNATURE, IL2CPP_METADATA_SIGNATURE,
    IL2CPP_RAX_SIGNATURE, IL2CPP_SHR_SIGNATURE, SignatureEntry, SignaturePool, StaticData,
    StringPool,
};
use self::dependencies::{BackendDependencies, GeneratedHelper};
use self::expression::{
    ExprContext, LocalStorage, compile_assignment, compile_block, compile_expr, compile_receiver,
};
use self::imports::Abi;
use self::runtime_helpers::emit_value_equality;
use self::script_functions::{
    compile_action, compile_read, compile_user_function, emit_action_default, plan_wasm_locals,
};
use self::settings::{compile_refresh_settings, compile_start, compile_string_from_memory};
use self::update::compile_update;

const STATE_TYPE: u32 = 0;

fn standard_gc_type_index(ty: StdlibTypeId) -> u32 {
    StandardLibrary::new()
        .types()
        .iter()
        .filter(|declaration| {
            matches!(
                declaration.representation,
                RuntimeRepresentation::GcArray { .. }
                    | RuntimeRepresentation::GcStruct { .. }
                    | RuntimeRepresentation::Enum { .. }
            )
        })
        .position(|declaration| declaration.id == ty)
        .map(|position| STATE_TYPE + 1 + position as u32)
        .unwrap_or_else(|| panic!("standard type `{ty:?}` has no static GC layout"))
}

fn standard_gc_type_count() -> u32 {
    StandardLibrary::new()
        .types()
        .iter()
        .filter(|declaration| {
            matches!(
                declaration.representation,
                RuntimeRepresentation::GcArray { .. }
                    | RuntimeRepresentation::GcStruct { .. }
                    | RuntimeRepresentation::Enum { .. }
            )
        })
        .count() as u32
}

fn standard_field_index(field: StdlibFieldId) -> u32 {
    let library = StandardLibrary::new();
    let declaration = library.field(field);
    let owner = library.type_decl(declaration.owner);
    let RuntimeRepresentation::GcStruct { .. } = owner.representation else {
        panic!("standard field `{field:?}` is not stored in a GC struct")
    };
    library
        .fields_of(owner.id)
        .position(|candidate| candidate.id == field)
        .map(|index| index as u32)
        .expect("every standard field belongs to its owner's declared slots")
}

fn enum_variant_index(
    enumeration: EnumTypeId,
    variant: ResolvedEnumVariantId,
    enums: &[EnumDecl],
) -> usize {
    match (enumeration, variant) {
        (EnumTypeId::Source(enumeration), ResolvedEnumVariantId::Source(variant)) => enums
            .iter()
            .find(|declaration| declaration.id == enumeration)
            .and_then(|declaration| {
                declaration
                    .variants
                    .iter()
                    .position(|declared| declared.id == variant)
            })
            .expect("checked source enum variants belong to their declaration"),
        (EnumTypeId::Standard(enumeration), ResolvedEnumVariantId::Standard(variant)) => {
            StandardLibrary::new()
                .variants_of(enumeration)
                .position(|declared| declared.id == variant)
                .expect("checked standard enum variants belong to their declaration")
        }
        _ => unreachable!("checked enum and variant identities have the same owner"),
    }
}

fn async_frame_type_index() -> u32 {
    STATE_TYPE + 1 + standard_gc_type_count()
}

fn dynamic_gc_type_base() -> u32 {
    async_frame_type_index() + 1
}

/// Physical value categories used while encoding WebAssembly. These are
/// derived from semantic `TypeId` values and are independent of inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Type {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Address,
    F32,
    F64,
    Standard(StdlibTypeId),
    Record(RecordId),
    Enum(EnumId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
}

struct GcLayout {
    dynamic: HashMap<Type, u32>,
    ordered: Vec<Type>,
    type_count: u32,
}

impl GcLayout {
    fn plan(
        program: &Program,
        enums: &[EnumDecl],
        arrays: &[ArrayTypeDecl],
        options: &[OptionTypeDecl],
        results: &[ResultTypeDecl],
        reachability: &reachability::Reachability,
    ) -> Self {
        let mut dynamic = HashMap::new();
        let mut ordered = Vec::new();
        let mut next = dynamic_gc_type_base();
        for record in program
            .records
            .iter()
            .filter(|record| reachability.contains_record_type(record.id))
        {
            dynamic.insert(Type::Record(record.id), next);
            ordered.push(Type::Record(record.id));
            next += 1;
        }
        for enumeration in enums
            .iter()
            .filter(|enumeration| reachability.contains_enum_type(enumeration.id))
        {
            dynamic.insert(Type::Enum(enumeration.id), next);
            ordered.push(Type::Enum(enumeration.id));
            next += 1;
        }
        let mut constructed = arrays
            .iter()
            .filter(|array| reachability.contains_array_type(array.id))
            .map(|array| (array.id.index(), Type::Array(array.id)))
            .chain(
                options
                    .iter()
                    .filter(|option| reachability.contains_option_type(option.id))
                    .map(|option| (option.id.index(), Type::Option(option.id))),
            )
            .chain(
                results
                    .iter()
                    .filter(|result| reachability.contains_result_type(result.id))
                    .map(|result| (result.id.index(), Type::Result(result.id))),
            )
            .collect::<Vec<_>>();
        constructed.sort_by_key(|(id, _)| *id);
        for (_, ty) in constructed {
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }
        Self {
            dynamic,
            ordered,
            type_count: next,
        }
    }

    fn dynamic_types(&self) -> impl ExactSizeIterator<Item = Type> + '_ {
        self.ordered.iter().copied()
    }

    fn index(&self, ty: Type) -> u32 {
        match ty {
            Type::Standard(standard) => standard_gc_type_index(standard),
            Type::Record(_)
            | Type::Enum(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_) => *self
                .dynamic
                .get(&ty)
                .unwrap_or_else(|| panic!("dynamic GC type `{ty:?}` was not marked reachable")),
            _ => unreachable!("scalar types have no GC heap index"),
        }
    }

    fn val_type(&self, ty: Type) -> ValType {
        match ty {
            Type::Void => unreachable!(),
            Type::Bool | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
                ValType::I32
            }
            Type::I64 | Type::U64 | Type::Address => ValType::I64,
            Type::F32 => ValType::F32,
            Type::F64 => ValType::F64,
            Type::Standard(standard) => {
                match StandardLibrary::new().type_decl(standard).representation {
                    RuntimeRepresentation::Scalar { storage } => {
                        self.val_type(Type::from_declared(DeclaredTypeRef::Core(storage)))
                    }
                    RuntimeRepresentation::GcArray { nullable, .. }
                    | RuntimeRepresentation::GcStruct { nullable, .. }
                    | RuntimeRepresentation::Enum { nullable } => ValType::Ref(RefType {
                        nullable,
                        heap_type: HeapType::Concrete(self.index(ty)),
                    }),
                }
            }
            Type::Record(_)
            | Type::Enum(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_) => ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(self.index(ty)),
            }),
        }
    }

    fn storage_type(&self, ty: Type) -> StorageType {
        match ty {
            Type::Bool | Type::I8 | Type::U8 => StorageType::I8,
            Type::I16 | Type::U16 => StorageType::I16,
            _ => StorageType::Val(self.val_type(ty)),
        }
    }
}

impl Type {
    fn from_standard(ty: StdlibTypeId) -> Self {
        Self::Standard(ty)
    }

    fn from_declared(ty: DeclaredTypeRef) -> Self {
        match ty {
            DeclaredTypeRef::Core(core) => match core {
                CoreTypeId::Void => Self::Void,
                CoreTypeId::Bool => Self::Bool,
                CoreTypeId::I8 => Self::I8,
                CoreTypeId::U8 => Self::U8,
                CoreTypeId::I16 => Self::I16,
                CoreTypeId::U16 => Self::U16,
                CoreTypeId::I32 => Self::I32,
                CoreTypeId::U32 => Self::U32,
                CoreTypeId::I64 => Self::I64,
                CoreTypeId::U64 => Self::U64,
                CoreTypeId::Address => Self::Address,
                CoreTypeId::F32 => Self::F32,
                CoreTypeId::F64 => Self::F64,
            },
            DeclaredTypeRef::Standard(standard) => Self::from_standard(standard),
        }
    }

    fn is_signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    fn is_enum(self) -> bool {
        matches!(self, Self::Enum(_))
            || matches!(
                self,
                Self::Standard(standard)
                    if matches!(
                        StandardLibrary::new().type_decl(standard).representation,
                        RuntimeRepresentation::Enum { .. }
                    )
            )
    }
}

const PROCESS_GLOBAL: u32 = 0;
const CURRENT_GLOBAL: u32 = 1;
const OLD_GLOBAL: u32 = 2;
const ATTACH_READY_GLOBAL: u32 = 3;
const ASYNC_FRAME_GLOBAL: u32 = 4;
const DETACHED_ENTERED_GLOBAL: u32 = 5;
const STRING_SCRATCH: i32 = 32_768;
const SCAN_SCRATCH: i32 = 40_960;
const C_STRING_SCRATCH: i32 = 49_152;
const MANAGED_UTF16_SCRATCH: i32 = 57_344;
const MANAGED_UTF8_SCRATCH: i32 = 61_440;
const SETTINGS_LENGTH_SCRATCH: i32 = 32_764;
const SETTINGS_STRING_SCRATCH: i32 = 32_768;
const SETTINGS_STRING_CAPACITY: i32 = 16_384;

#[derive(Default)]
struct AsyncFrameLayout {
    fields: HashMap<ValueId, (u32, Type)>,
    types: Vec<Type>,
}

#[derive(Default)]
struct MatchLayout {
    values: HashMap<ExprId, u32>,
    bindings: HashMap<crate::ast::PatternId, (u32, Type)>,
    fallback_values: HashMap<ExprId, u32>,
    intrinsic_temps: HashMap<ExprId, Vec<u32>>,
    suspension_temps: HashMap<ExprId, u32>,
}

#[derive(Default)]
struct EqualityFunctions {
    records: HashMap<RecordId, u32>,
    enums: HashMap<EnumId, u32>,
    options: HashMap<OptionTypeId, u32>,
    results: HashMap<ResultTypeId, u32>,
}

#[derive(Clone, Copy)]
struct SettingStorage {
    current: u32,
    old: u32,
    ty: Type,
}

impl AsyncFrameLayout {
    fn for_action(
        action: Option<ActionKind>,
        wasm_ir: &wasm_ir::Program,
        semantics: &SemanticModel,
    ) -> Option<Self> {
        let action = action?;
        let body = wasm_ir
            .body(BodyOwner::Action(action))
            .expect("checked actions have Wasm IR bodies");
        let mut layout = Self::default();
        for local in &body.locals {
            if let LocalPurpose::Value(value) = local.purpose
                && body.frame_values.contains(&value)
            {
                let ty = semantic_type(local.ty, semantics);
                let field = 1 + layout.types.len() as u32;
                layout.fields.insert(value, (field, ty));
                layout.types.push(ty);
            }
        }
        Some(layout)
    }

    fn field(&self, binding: Option<ValueId>) -> Option<(u32, Type)> {
        binding.and_then(|binding| self.fields.get(&binding).copied())
    }
}

struct Stdlib {
    helpers: HashMap<GeneratedHelper, u32>,
}

impl Stdlib {
    fn helper(&self, helper: GeneratedHelper) -> u32 {
        self.helpers[&helper]
    }

    fn optional_helper(&self, helper: GeneratedHelper) -> Option<u32> {
        self.helpers.get(&helper).copied()
    }
}

struct RuntimeContext<'a> {
    abi: &'a Abi,
    strings: &'a StringPool,
    signatures: &'a SignaturePool,
    lowering: &'a LoweringContext<'a>,
}

struct LoweringContext<'a> {
    abi: &'a Abi,
    state: &'a crate::ast::StateDecl,
    globals: &'a HashMap<ValueId, u32>,
    global_types: &'a HashMap<ValueId, Type>,
    settings: &'a HashMap<ValueId, SettingStorage>,
    stdlib: &'a Stdlib,
    functions: &'a HashMap<FunctionId, u32>,
    equality_functions: &'a EqualityFunctions,
    records: &'a [RecordDecl],
    enums: &'a [EnumDecl],
    arrays: &'a [ArrayTypeDecl],
    memory: &'a MemoryLayouts,
    semantics: &'a SemanticModel,
    wasm_ir: &'a wasm_ir::Program,
    gc: &'a GcLayout,
}

pub struct ConstructedTypes<'a> {
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ArrayTypeDecl],
    pub options: &'a [OptionTypeDecl],
    pub results: &'a [ResultTypeDecl],
}

pub fn compile(
    program: &Program,
    semantics: &SemanticModel,
    typed_hir: &crate::hir::TypedProgram,
    wasm_ir: &wasm_ir::Program,
    constructed_types: ConstructedTypes<'_>,
    memory_layouts: &MemoryLayouts,
    equality: &EqualityCapabilities,
) -> Vec<u8> {
    let ConstructedTypes {
        enums,
        arrays: array_types,
        options: option_types,
        results: result_types,
    } = constructed_types;
    let state = program.state.as_ref().unwrap();
    let on_attach = program
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OnAttach)
        .map(|action| action.kind);
    let async_layout = AsyncFrameLayout::for_action(on_attach, wasm_ir, semantics);
    let cancellation_region = on_attach.and_then(|action| {
        wasm_ir
            .body(BodyOwner::Action(action))
            .and_then(|body| body.cancellation_region)
    });
    let reachability = reachability::Reachability::analyze(program, semantics, enums, wasm_ir);
    let dependencies = BackendDependencies::analyze(program, semantics, wasm_ir, &reachability);
    let static_data = StaticData::collect(program, wasm_ir, &reachability, &dependencies);
    let strings = &static_data.strings;
    let signatures = &static_data.signatures;

    let gc_types::EncodedTypes {
        section: mut types,
        next_type_index: first_import_type,
        layout: gc,
    } = gc_types::encode(gc_types::Inputs {
        program,
        semantics,
        async_layout: async_layout.as_ref(),
        enums,
        array_types,
        option_types,
        result_types,
        reachability: &reachability,
    });
    let imports::EncodedImports {
        section: imports,
        abi,
        function_count: imported_functions,
        next_type_index,
    } = imports::encode(&mut types, first_import_type, &dependencies);

    let global_plan::GlobalPlan {
        section: globals,
        variables: global_indices,
        variable_types: global_types,
        settings: setting_indices,
    } = global_plan::encode(program, semantics, typed_hir, &gc, wasm_ir);

    let mut codes = CodeSection::new();
    let function_plan::FunctionPlan {
        section: functions,
        stdlib,
        equality: equality_functions,
        users: user_functions,
        reads: read_functions,
        actions: action_functions,
        start: start_function,
        update: update_function,
        string_values,
        u64_offsets,
    } = function_plan::encode(
        &mut types,
        next_type_index,
        imported_functions,
        function_plan::Inputs {
            program,
            semantics,
            enums,
            arrays: array_types,
            options: option_types,
            results: result_types,
            equality,
            dependencies: &dependencies,
            reachability: &reachability,
            gc: &gc,
        },
    );
    let lowering = LoweringContext {
        abi: &abi,
        state,
        globals: &global_indices,
        global_types: &global_types,
        settings: &setting_indices,
        stdlib: &stdlib,
        functions: &user_functions,
        equality_functions: &equality_functions,
        records: &program.records,
        enums,
        arrays: array_types,
        memory: memory_layouts,
        semantics,
        wasm_ir,
        gc: &gc,
    };
    let runtime = RuntimeContext {
        abi: &abi,
        strings,
        signatures,
        lowering: &lowering,
    };

    let runtime_helpers::HelperBodies {
        core: helper_bodies,
        equality: equality_bodies,
    } = runtime_helpers::compile(
        &abi,
        strings,
        signatures,
        &stdlib,
        string_values,
        u64_offsets,
        &program.records,
        enums,
        option_types,
        result_types,
        semantics,
        &equality_functions,
        &dependencies,
        &gc,
    );
    for body in helper_bodies {
        codes.function(&body);
    }
    let refresh_settings = stdlib.optional_helper(GeneratedHelper::RefreshSettings);
    if stdlib
        .optional_helper(GeneratedHelper::StringFromMemory)
        .is_some()
    {
        codes.function(&compile_string_from_memory());
    }
    if refresh_settings.is_some() {
        codes.function(&compile_refresh_settings(
            program,
            &lowering,
            strings,
            &setting_indices,
            stdlib
                .optional_helper(GeneratedHelper::StringFromMemory)
                .unwrap_or(0),
            stdlib
                .optional_helper(GeneratedHelper::StringEquality)
                .unwrap_or(0),
        ));
    }
    for body in equality_bodies {
        codes.function(&body);
    }
    for function in &program.functions {
        if reachability.contains_function(function.id) {
            codes.function(&compile_user_function(function, &lowering));
        }
    }
    for field in &state.fields {
        codes.function(&compile_read(field, &abi, strings, &lowering));
    }
    for action in &program.actions {
        if action.kind == ActionKind::OnAttach {
            codes.function(&compile_async_attach(
                action,
                async_layout.as_ref().unwrap(),
                &runtime,
            ));
        } else {
            codes.function(&compile_action(action, &lowering));
        }
    }
    codes.function(&compile_start(
        program,
        &lowering,
        strings,
        &setting_indices,
        refresh_settings,
        async_layout.is_some(),
    ));
    codes.function(&compile_update(
        program,
        strings,
        &read_functions,
        &action_functions,
        refresh_settings,
        cancellation_region,
        &lowering,
    ));

    module_assembly::finish(
        module_assembly::Sections {
            types,
            imports,
            functions,
            globals,
            codes,
        },
        &static_data,
        start_function,
        update_function,
    )
}

fn resolved_intrinsic(target: &ResolvedCall) -> Option<IntrinsicId> {
    match target {
        ResolvedCall::StandardLibrary { item, .. } => {
            let catalog_item = StandardLibrary::new().item(*item);
            match catalog_item.implementation {
                Implementation::Intrinsic(intrinsic) => Some(intrinsic),
            }
        }
        ResolvedCall::UserFunction { .. }
        | ResolvedCall::UserMethod { .. }
        | ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::ResultSuccess { .. } => None,
    }
}

fn call_target(wasm_ir: &wasm_ir::Program, expression: ExprId) -> &ResolvedCall {
    let wasm_ir::ExpressionKind::Call { target, .. } = &wasm_ir
        .expression(expression)
        .expect("checked call belongs to Wasm IR")
        .kind
    else {
        unreachable!("suspending values are resolved calls")
    };
    target
}

fn semantic_type(id: TypeId, semantics: &SemanticModel) -> Type {
    match semantics.types().kind(id) {
        TypeKind::Builtin(builtin) => match builtin {
            BuiltinType::Void => Type::Void,
            BuiltinType::Bool => Type::Bool,
            BuiltinType::I8 => Type::I8,
            BuiltinType::U8 => Type::U8,
            BuiltinType::I16 => Type::I16,
            BuiltinType::U16 => Type::U16,
            BuiltinType::I32 => Type::I32,
            BuiltinType::U32 => Type::U32,
            BuiltinType::I64 => Type::I64,
            BuiltinType::U64 => Type::U64,
            BuiltinType::Address => Type::Address,
            BuiltinType::F32 => Type::F32,
            BuiltinType::F64 => Type::F64,
        },
        TypeKind::Standard(standard) => Type::Standard(*standard),
        TypeKind::Record(record) => Type::Record(*record),
        TypeKind::Enum(enumeration) => Type::Enum(*enumeration),
        TypeKind::Array { layout, .. } => Type::Array(*layout),
        TypeKind::Option { layout, .. } => Type::Option(*layout),
        TypeKind::Result { layout, .. } => Type::Result(*layout),
    }
}

fn expression_type(
    expression: ExprId,
    wasm_ir: &wasm_ir::Program,
    semantics: &SemanticModel,
) -> Type {
    semantic_type(
        wasm_ir
            .expression(expression)
            .expect("checked expressions belong to Wasm IR")
            .ty,
        semantics,
    )
}

fn value_type(value: ValueId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .value_type(value)
            .expect("checked value declarations have semantic types"),
        semantics,
    )
}

fn function_result(function: FunctionId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .function_result(function)
            .expect("checked functions have semantic result types"),
        semantics,
    )
}

fn record_field_type(field: RecordFieldId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .record_field_type(field)
            .expect("checked record fields have semantic types"),
        semantics,
    )
}

fn enum_variant_payload(variant: EnumVariantId, semantics: &SemanticModel) -> Option<Type> {
    semantics
        .enum_variant_payload(variant)
        .map(|payload| semantic_type(payload, semantics))
}

fn array_element_type(array: ArrayTypeId, semantics: &SemanticModel) -> Type {
    semantic_type(
        semantics
            .array_element_type(array)
            .expect("checked array layouts have semantic element types"),
        semantics,
    )
}

fn option_value_type(option: OptionTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Option { layout, value } if *layout == option => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked option layouts have semantic value types")
}

fn result_value_type(result: ResultTypeId, semantics: &SemanticModel) -> Type {
    semantics
        .types()
        .iter()
        .find_map(|(_, kind)| match kind {
            TypeKind::Result { layout, value } if *layout == result => {
                Some(semantic_type(*value, semantics))
            }
            _ => None,
        })
        .expect("checked result layouts have semantic value types")
}

fn emit_struct_get(function: &mut Function, field_index: u32, ty: Type) {
    emit_typed_struct_get(function, STATE_TYPE, field_index, ty);
}

fn emit_async_frame_ref(function: &mut Function) {
    function
        .instruction(&Instruction::GlobalGet(ASYNC_FRAME_GLOBAL))
        .instruction(&Instruction::RefAsNonNull);
}

fn emit_typed_struct_get(
    function: &mut Function,
    struct_type_index: u32,
    field_index: u32,
    ty: Type,
) {
    let instruction = match ty {
        Type::Bool | Type::U8 | Type::U16 => Instruction::StructGetU {
            struct_type_index,
            field_index,
        },
        Type::I8 | Type::I16 => Instruction::StructGetS {
            struct_type_index,
            field_index,
        },
        _ => Instruction::StructGet {
            struct_type_index,
            field_index,
        },
    };
    function.instruction(&instruction);
}

fn emit_array_get(function: &mut Function, array_type_index: u32, element: Type) {
    function.instruction(&match element {
        Type::Bool | Type::U8 | Type::U16 => Instruction::ArrayGetU(array_type_index),
        Type::I8 | Type::I16 => Instruction::ArrayGetS(array_type_index),
        _ => Instruction::ArrayGet(array_type_index),
    });
}

fn emit_memory_value(
    function: &mut Function,
    ty: TypeId,
    offset: u32,
    memory: &MemoryLayouts,
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    match memory
        .layout(ty, semantics)
        .expect("checked memory values are MemoryReadable")
    {
        MemoryTypeLayout::Scalar { .. } => {
            function.instruction(&Instruction::I32Const(offset as i32));
            emit_memory_load(function, semantic_type(ty, semantics));
        }
        MemoryTypeLayout::Record(layout) => {
            for field in &layout.fields {
                emit_memory_value(
                    function,
                    field.ty,
                    offset + field.offset,
                    memory,
                    semantics,
                    gc,
                );
            }
            function.instruction(&Instruction::StructNew(
                gc.index(Type::Record(layout.record)),
            ));
        }
    }
}

fn emit_memory_load(function: &mut Function, ty: Type) {
    function.instruction(&match ty {
        Type::Bool | Type::U8 => Instruction::I32Load8U(memarg()),
        Type::I8 => Instruction::I32Load8S(memarg()),
        Type::U16 => Instruction::I32Load16U(memarg()),
        Type::I16 => Instruction::I32Load16S(memarg()),
        Type::I32 | Type::U32 => Instruction::I32Load(memarg()),
        Type::I64 | Type::U64 | Type::Address => Instruction::I64Load(memarg()),
        Type::F32 => Instruction::F32Load(memarg()),
        Type::F64 => Instruction::F64Load(memarg()),
        _ => unreachable!(),
    });
}

fn emit_default(function: &mut Function, ty: Type, gc: &GcLayout) {
    function.instruction(&match gc.val_type(ty) {
        ValType::I32 => Instruction::I32Const(0),
        ValType::I64 => Instruction::I64Const(0),
        ValType::F32 => Instruction::F32Const(0.0.into()),
        ValType::F64 => Instruction::F64Const(0.0.into()),
        ValType::Ref(reference) => Instruction::RefNull(reference.heap_type),
        ValType::V128 => unreachable!(),
    });
}

/// Wraps the value already on the operand stack in a successful `T!`.
fn emit_result_success(function: &mut Function, result: ResultTypeId, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            standard_gc_type_index(StdlibTypeId::String),
        )))
        .instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

fn emit_result_error(
    function: &mut Function,
    result: ResultTypeId,
    value_type: Type,
    message: &str,
    gc: &GcLayout,
) {
    emit_default(function, value_type, gc);
    function.instruction(&Instruction::I32Const(1));
    emit_string_literal(function, message);
    function.instruction(&Instruction::StructNew(gc.index(Type::Result(result))));
}

/// Transfers an error to the nearest compiled failure boundary.
///
/// Both an explicit `throw` and the failure arm of postfix `?` lower through
/// this operation. A future nested `catch` can replace the final return with a
/// branch to the selected handler without changing either source construct.
fn emit_failure_transfer(
    function: &mut Function,
    target: ResultTypeId,
    target_value: Type,
    gc: &GcLayout,
    emit_error: impl FnOnce(&mut Function),
) {
    emit_default(function, target_value, gc);
    function.instruction(&Instruction::I32Const(1));
    emit_error(function);
    function
        .instruction(&Instruction::StructNew(gc.index(Type::Result(target))))
        .instruction(&Instruction::Return);
}

fn emit_int(function: &mut Function, value: u64, ty: Type) {
    if matches!(ty, Type::I64 | Type::U64 | Type::Address) {
        function.instruction(&Instruction::I64Const(value as i64));
    } else {
        function.instruction(&Instruction::I32Const(value as i32));
    }
}

fn emit_string_literal(function: &mut Function, value: &str) {
    for byte in value.bytes() {
        function.instruction(&Instruction::I32Const(byte as i32));
    }
    function.instruction(&Instruction::ArrayNewFixed {
        array_type_index: standard_gc_type_index(StdlibTypeId::String),
        array_size: value.len() as u32,
    });
}

fn constant(expression: ExprId, typed_hir: &TypedProgram, ty: Type) -> ConstExpr {
    let expression = typed_hir
        .expression(expression)
        .expect("global initializer belongs to typed HIR");
    let negative = matches!(
        expression.kind,
        TypedExpressionKind::Unary {
            op: UnaryOp::Neg,
            ..
        }
    );
    let inner = if let TypedExpressionKind::Unary {
        expression: inner, ..
    } = expression.kind
    {
        typed_hir
            .expression(inner)
            .expect("global initializer operand belongs to typed HIR")
    } else {
        expression
    };
    match &inner.kind {
        TypedExpressionKind::Bool(value) => ConstExpr::i32_const(*value as i32),
        TypedExpressionKind::Int { value, .. }
            if matches!(ty, Type::I64 | Type::U64 | Type::Address) =>
        {
            ConstExpr::i64_const(if negative {
                -(*value as i64)
            } else {
                *value as i64
            })
        }
        TypedExpressionKind::Int { value, .. } => ConstExpr::i32_const(if negative {
            -(*value as i32)
        } else {
            *value as i32
        }),
        TypedExpressionKind::Float(value) if ty == Type::F32 => ConstExpr::f32_const(
            (if negative {
                -(*value as f32)
            } else {
                *value as f32
            })
            .into(),
        ),
        TypedExpressionKind::Float(value) => {
            ConstExpr::f64_const((if negative { -*value } else { *value }).into())
        }
        _ => unreachable!(),
    }
}

fn val_type(ty: Type) -> ValType {
    match ty {
        Type::Bool | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
            ValType::I32
        }
        Type::I64 | Type::U64 | Type::Address => ValType::I64,
        Type::F32 => ValType::F32,
        Type::F64 => ValType::F64,
        Type::Standard(standard) => match StandardLibrary::new().type_decl(standard).representation
        {
            RuntimeRepresentation::Scalar { storage } => {
                val_type(Type::from_declared(DeclaredTypeRef::Core(storage)))
            }
            RuntimeRepresentation::GcArray { nullable, .. }
            | RuntimeRepresentation::GcStruct { nullable, .. }
            | RuntimeRepresentation::Enum { nullable } => ValType::Ref(RefType {
                nullable,
                heap_type: HeapType::Concrete(standard_gc_type_index(standard)),
            }),
        },
        Type::Record(_) | Type::Enum(_) | Type::Array(_) | Type::Option(_) | Type::Result(_) => {
            unreachable!("dynamic GC types require a GcLayout lookup")
        }
        _ => unreachable!(),
    }
}

fn action_result_val_type(action: ActionKind) -> ValType {
    if action == ActionKind::GameTime {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(standard_gc_type_index(StdlibTypeId::Duration)),
        })
    } else {
        ValType::I32
    }
}

fn memarg() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}
