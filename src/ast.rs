use std::fmt;

pub const TIMER_STATE_TYPE_NAME: &str = "TimerState";
pub const TIMER_STATE_VARIANTS: [&str; 5] = ["NotRunning", "Running", "Paused", "Ended", "Unknown"];

pub(crate) fn timer_state_enum(id: EnumId, first_variant: u32) -> EnumDecl {
    EnumDecl {
        id,
        name: TIMER_STATE_TYPE_NAME.to_owned(),
        variants: TIMER_STATE_VARIANTS
            .into_iter()
            .enumerate()
            .map(|(offset, name)| EnumVariant {
                id: EnumVariantId::from_index(first_variant + offset as u32),
                name: name.to_owned(),
                payload: None,
                span: Span::default(),
            })
            .collect(),
        span: Span::default(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Stable identity for an expression in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(u32);

impl ExprId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a user function or method in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(u32);

impl FunctionId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a user-declared value in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(u32);

impl ValueId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for an assignment statement in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssignmentId(u32);

impl AssignmentId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a user-declared record in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(u32);

impl RecordId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a user-declared enum in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumId(u32);

impl EnumId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for an array layout in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArrayTypeId(u32);

impl ArrayTypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for an optional-value GC layout in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OptionTypeId(u32);

impl OptionTypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a result-value GC layout in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResultTypeId(u32);

impl ResultTypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a field declared by a record in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordFieldId(u32);

impl RecordFieldId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a variant declared by an enum in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumVariantId(u32);

impl EnumVariantId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

macro_rules! display_stable_id {
    ($($ty:ty),* $(,)?) => {
        $(
            impl fmt::Display for $ty {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    self.0.fmt(formatter)
                }
            }
        )*
    };
}

display_stable_id!(
    RecordId,
    EnumId,
    ArrayTypeId,
    OptionTypeId,
    ResultTypeId,
    RecordFieldId,
    EnumVariantId
);

/// Stable identity for a match pattern in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PatternId(u32);

impl PatternId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for an option in a choice setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SettingChoiceOptionId(u32);

impl SettingChoiceOptionId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

display_stable_id!(PatternId, SettingChoiceOptionId);

impl Span {
    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub state: Option<StateDecl>,
    pub settings: Vec<SettingDecl>,
    pub globals: Vec<VariableDecl>,
    pub records: Vec<RecordDecl>,
    pub enums: Vec<EnumDecl>,
    pub array_types: Vec<ArrayTypeDecl>,
    pub option_types: Vec<OptionTypeDecl>,
    pub result_types: Vec<ResultTypeDecl>,
    pub functions: Vec<FunctionDecl>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Copy)]
pub struct ArrayTypeDecl {
    pub id: ArrayTypeId,
    pub element: TypeRef,
}

#[derive(Debug, Clone, Copy)]
pub struct OptionTypeDecl {
    pub id: OptionTypeId,
    pub value: TypeRef,
}

#[derive(Debug, Clone, Copy)]
pub struct ResultTypeDecl {
    pub id: ResultTypeId,
    pub value: TypeRef,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub id: EnumId,
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub id: EnumVariantId,
    pub name: String,
    pub payload: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct RecordDecl {
    pub id: RecordId,
    pub name: String,
    pub fields: Vec<RecordField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct RecordField {
    pub id: RecordFieldId,
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub id: FunctionId,
    pub name: String,
    pub debug_only: bool,
    pub method_of: Option<TypeRef>,
    pub params: Vec<Parameter>,
    pub return_annotation: Option<TypeRef>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub id: ValueId,
    pub name: String,
    pub annotation: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateDecl {
    pub processes: Vec<String>,
    pub fields: Vec<StateField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateField {
    pub id: ValueId,
    pub name: String,
    pub annotation: Option<TypeRef>,
    pub source: StateSource,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StateSource {
    Pointer(PointerPath),
    Expression(Expr),
}

#[derive(Debug, Clone)]
pub struct PointerPath {
    pub module: Option<String>,
    pub offsets: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct SettingDecl {
    pub id: ValueId,
    pub name: String,
    pub description: String,
    pub tooltip: Option<String>,
    pub kind: SettingKind,
    pub span: Span,
}

impl SettingDecl {
    pub fn value_type(&self) -> Option<TypeRef> {
        match &self.kind {
            SettingKind::Bool { .. } => Some(TypeRef::Bool),
            SettingKind::Choice { enumeration, .. } => Some(TypeRef::Enum(*enumeration)),
            SettingKind::File { .. } => Some(TypeRef::String),
            SettingKind::Title { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SettingKind {
    Bool {
        default: bool,
    },
    Title {
        heading_level: u32,
    },
    Choice {
        enumeration: EnumId,
        default_variant: String,
        options: Vec<SettingChoiceOption>,
    },
    File {
        filters: Vec<SettingFileFilter>,
    },
}

#[derive(Debug, Clone)]
pub struct SettingChoiceOption {
    pub id: SettingChoiceOptionId,
    pub variant: String,
    pub description: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum SettingFileFilter {
    Name {
        description: Option<String>,
        pattern: String,
    },
    Mime(String),
}

#[derive(Debug, Clone)]
pub struct VariableDecl {
    pub id: ValueId,
    pub name: String,
    pub mutable: bool,
    pub debug_only: bool,
    pub annotation: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    OnDetached,
    OnAttach,
    WhileAttached,
    Start,
    Split,
    Reset,
    IsLoading,
    GameTime,
}

impl ActionKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "onDetached" => Self::OnDetached,
            "onAttach" => Self::OnAttach,
            "whileAttached" => Self::WhileAttached,
            "start" => Self::Start,
            "split" => Self::Split,
            "reset" => Self::Reset,
            "isLoading" => Self::IsLoading,
            "gameTime" => Self::GameTime,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::OnDetached => "onDetached",
            Self::OnAttach => "onAttach",
            Self::WhileAttached => "whileAttached",
            Self::Start => "start",
            Self::Split => "split",
            Self::Reset => "reset",
            Self::IsLoading => "isLoading",
            Self::GameTime => "gameTime",
        }
    }

    pub fn return_type(self) -> TypeRef {
        match self {
            Self::OnDetached | Self::OnAttach | Self::WhileAttached => TypeRef::Void,
            Self::Start | Self::Split | Self::Reset | Self::IsLoading => TypeRef::Bool,
            Self::GameTime => TypeRef::Duration,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Action {
    pub kind: ActionKind,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Debug {
        statement: Box<Stmt>,
        span: Span,
    },
    Variable(VariableDecl),
    Assign {
        id: AssignmentId,
        name: String,
        op: Option<BinaryOp>,
        value: Expr,
        span: Span,
    },
    If {
        condition: Expr,
        then_block: Block,
        else_block: Option<Block>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Block,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Throw {
        error: Expr,
        span: Span,
    },
    Suspend {
        mode: SuspensionMode,
        binding: Option<SuspensionBinding>,
        value: Expr,
        span: Span,
    },
    Expression(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionMode {
    Await,
    Retry,
}

#[derive(Debug, Clone)]
pub struct SuspensionBinding {
    pub id: ValueId,
    pub name: String,
    pub annotation: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub(crate) fn parsed(id: ExprId, kind: ExprKind, span: Span) -> Self {
        Self { id, kind, span }
    }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    /// A syntax-only placeholder inserted by the recovering parser.
    Error,
    None,
    Bool(bool),
    Int {
        value: u64,
        suffix: Option<TypeRef>,
    },
    Float(f64),
    String(String),
    InterpolatedString(Vec<InterpolatedPart>),
    Signature(String),
    Array(Vec<Expr>),
    Record {
        record: RecordId,
        fields: Vec<(String, Expr)>,
    },
    Enum {
        enumeration: EnumId,
        variant: String,
        payload: Option<Box<Expr>>,
    },
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    If {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Fallback {
        value: Box<Expr>,
        fallback: FallbackBranch,
    },
    Propagate(Box<Expr>),
    Path(Vec<String>),
    Member {
        receiver: Box<Expr>,
        name: String,
        name_span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        target: TypeRef,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        callee: Vec<String>,
        name_span: Span,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum FallbackBranch {
    Value(Box<Expr>),
    Return {
        value: Option<Box<Expr>>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern_id: PatternId,
    pub pattern: MatchPattern,
    pub guard: Option<Expr>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PatternBinding {
    pub id: ValueId,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Enum {
        enumeration: EnumId,
        variant: String,
        binding: Option<PatternBinding>,
    },
    Bool(bool),
    Int {
        value: u64,
        suffix: Option<TypeRef>,
    },
    None,
    OptionSome(Option<PatternBinding>),
    ResultSuccess(Option<PatternBinding>),
    ResultError(Option<PatternBinding>),
    Wildcard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// A type written in source code. Parsed type references never contain
/// inference variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeRef {
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
    String,
    Signature,
    Duration,
    Module,
    UnityModule,
    UnityImage,
    UnityClass,
    UnityField,
    Record(RecordId),
    Enum(EnumId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
}

impl TypeRef {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "bool" => Self::Bool,
            "i8" => Self::I8,
            "u8" => Self::U8,
            "i16" => Self::I16,
            "u16" => Self::U16,
            "i32" => Self::I32,
            "u32" => Self::U32,
            "i64" => Self::I64,
            "u64" => Self::U64,
            "address" | "Address" => Self::Address,
            "f32" => Self::F32,
            "f64" => Self::F64,
            "String" | "string" => Self::String,
            "Duration" => Self::Duration,
            "Module" => Self::Module,
            "UnityModule" => Self::UnityModule,
            "UnityImage" => Self::UnityImage,
            "UnityClass" => Self::UnityClass,
            "UnityField" => Self::UnityField,
            _ => return None,
        })
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::Address
        )
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Address => "address",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::String => "String",
            Self::Signature => "Signature",
            Self::Duration => "Duration",
            Self::Module => "Module",
            Self::UnityModule => "UnityModule",
            Self::UnityImage => "UnityImage",
            Self::UnityClass => "UnityClass",
            Self::UnityField => "UnityField",
            Self::Record(id) => return write!(f, "record#{id}"),
            Self::Enum(id) => return write!(f, "enum#{id}"),
            Self::Array(id) => return write!(f, "Array#{id}"),
            Self::Option(id) => return write!(f, "Option#{id}"),
            Self::Result(id) => return write!(f, "Result#{id}"),
        })
    }
}
