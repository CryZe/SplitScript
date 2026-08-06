use std::fmt;

pub use crate::{PrimitiveType, Span};

/// Stable identity for an expression in one parsed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(u32);

impl ExprId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// Creates an identity for a compiler-generated expression arena entry.
    ///
    /// Source parsers should allocate IDs through their parser context. This
    /// constructor exists for downstream lowering passes that extend the
    /// expression arena after parsing.
    #[doc(hidden)]
    pub fn from_index(index: u32) -> Self {
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

/// An enum name written in source syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumReference {
    pub name: String,
    pub span: Span,
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

/// Stable identity for an asynchronous-value type expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AsyncTypeId(u32);

impl AsyncTypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Stable identity for a named generic type application such as `Set<String>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeApplicationId(u32);

impl TypeApplicationId {
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
    AsyncTypeId,
    TypeApplicationId,
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

/// Stable identity for a nominal type name written in one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeNameId(u32);

impl TypeNameId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }
}

display_stable_id!(TypeNameId);

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub type_names: Vec<String>,
    pub type_name_spans: Vec<Span>,
    pub type_name_occurrences: Vec<Vec<Span>>,
    pub state: Option<StateDecl>,
    /// The complete `settings` declaration, including its keyword and body.
    pub settings_span: Option<Span>,
    pub settings: Vec<SettingDecl>,
    pub globals: Vec<VariableDecl>,
    pub records: Vec<RecordDecl>,
    pub enums: Vec<EnumDecl>,
    /// Interned source type-expression nodes. Their IDs are syntax identities,
    /// not inferred types or physical layouts; later stages translate them
    /// into their own semantic and backend type universes.
    pub array_types: Vec<ArrayTypeDecl>,
    pub option_types: Vec<OptionTypeDecl>,
    pub result_types: Vec<ResultTypeDecl>,
    pub async_types: Vec<AsyncTypeDecl>,
    pub type_applications: Vec<TypeApplicationDecl>,
    pub functions: Vec<FunctionDecl>,
    pub actions: Vec<Action>,
}

impl Program {
    /// Iterates both ordinary source enums and the enum generated by named
    /// state layouts.
    pub fn enum_declarations(&self) -> impl Iterator<Item = &EnumDecl> {
        self.enums.iter().chain(
            self.state
                .as_ref()
                .and_then(|state| state.layout_enum.as_ref()),
        )
    }

    pub fn enum_declaration(&self, id: EnumId) -> Option<&EnumDecl> {
        self.enum_declarations()
            .find(|enumeration| enumeration.id == id)
    }

    /// Iterates nominal type names together with their stable syntax identity
    /// and source span.
    pub fn type_names(&self) -> impl Iterator<Item = (TypeNameId, &str, Span)> {
        self.type_names
            .iter()
            .zip(&self.type_name_spans)
            .enumerate()
            .map(|(index, (name, span))| {
                (TypeNameId::from_index(index as u32), name.as_str(), *span)
            })
    }

    pub fn type_name(&self, id: TypeNameId) -> &str {
        &self.type_names[id.index()]
    }

    pub fn type_name_span(&self, id: TypeNameId) -> Span {
        self.type_name_spans[id.index()]
    }

    pub fn type_name_occurrences(&self, id: TypeNameId) -> &[Span] {
        &self.type_name_occurrences[id.index()]
    }
}

/// Allocates constructed-type identities in the shared per-program ID space.
///
/// Parsing reserves identities for written type expressions. Type inference
/// may then append layouts that arise only through inference. Keeping this
/// allocation behind one owner prevents compiler stages from fabricating raw
/// syntax IDs independently.
#[derive(Debug, Clone)]
pub struct ConstructedTypeIdAllocator {
    next: u32,
}

impl ConstructedTypeIdAllocator {
    pub fn starting_at(next: u32) -> Self {
        Self { next }
    }

    pub fn array(&mut self) -> ArrayTypeId {
        ArrayTypeId::from_index(self.take())
    }

    pub fn option(&mut self) -> OptionTypeId {
        OptionTypeId::from_index(self.take())
    }

    pub fn result(&mut self) -> ResultTypeId {
        ResultTypeId::from_index(self.take())
    }

    pub fn async_value(&mut self) -> AsyncTypeId {
        AsyncTypeId::from_index(self.take())
    }

    pub fn application(&mut self) -> TypeApplicationId {
        TypeApplicationId::from_index(self.take())
    }

    pub fn next_index(&self) -> u32 {
        self.next
    }

    fn take(&mut self) -> u32 {
        let current = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("a program cannot contain more than u32::MAX constructed types");
        current
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArrayTypeDecl {
    pub id: ArrayTypeId,
    pub element: TypeRef,
    /// An exact element count for `[T; N]`, or `None` for the general `[T]`
    /// array type.
    pub length: Option<u32>,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeApplicationDecl {
    pub id: TypeApplicationId,
    pub constructor: TypeNameId,
    pub arguments: Vec<TypeRef>,
    /// Every written occurrence retained even when structurally identical type
    /// applications share one semantic identity.
    pub occurrences: Vec<TypeApplicationOccurrence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeApplicationOccurrence {
    pub span: Span,
    pub constructor: Span,
    pub opening: Span,
    pub closing: Span,
}

#[derive(Debug, Clone, Copy)]
pub struct AsyncTypeDecl {
    pub id: AsyncTypeId,
    pub value: TypeRef,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub id: EnumId,
    pub name: String,
    pub documentation: Option<String>,
    pub name_span: Span,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub id: EnumVariantId,
    pub name: String,
    pub name_span: Span,
    pub documentation: Option<String>,
    pub payload: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct RecordDecl {
    pub id: RecordId,
    pub name: String,
    pub documentation: Option<String>,
    pub name_span: Span,
    pub fields: Vec<RecordField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct RecordField {
    pub id: RecordFieldId,
    pub name: String,
    pub name_span: Span,
    pub documentation: Option<String>,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub id: FunctionId,
    pub name: String,
    pub name_span: Span,
    pub documentation: Option<String>,
    pub debug_only: bool,
    pub method_of: Option<TypeRef>,
    pub params: Vec<Parameter>,
    pub return_annotation: Option<TypeRef>,
    /// Whether the explicitly written result is `async T` rather than `T`.
    ///
    /// An omitted result annotation has no syntax-level marker; its asyncness
    /// is inferred from the function body by semantic analysis.
    pub return_is_async: bool,
    pub return_async_span: Option<Span>,
    pub return_annotation_span: Option<Span>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub id: ValueId,
    pub name: String,
    pub name_span: Span,
    pub annotation: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateDecl {
    pub provider: Option<StateProviderRef>,
    pub processes: Vec<String>,
    /// Fields of the ordinary single-layout form. This is empty when named
    /// layouts are present.
    pub fields: Vec<StateField>,
    /// Versioned memory layouts. Semantic analysis projects compatible fields
    /// into a common interface and retains missing or conflicting fields as
    /// layout-specific declarations.
    pub layouts: Vec<StateLayoutDecl>,
    /// The generated enum represented by the named layout declarations.
    pub layout_enum: Option<EnumDecl>,
    /// Stable identity of the implicit read-only `layout` value.
    pub layout_value: Option<ValueId>,
    pub span: Span,
}

impl StateDecl {
    pub fn canonical_fields(&self) -> &[StateField] {
        self.layouts
            .first()
            .map_or(self.fields.as_slice(), |layout| layout.fields.as_slice())
    }

    pub fn all_fields(&self) -> impl Iterator<Item = &StateField> {
        self.fields
            .iter()
            .chain(self.layouts.iter().flat_map(|layout| &layout.fields))
    }

    /// Whether a field name is present in every layout without conflicting
    /// explicit annotations, and can therefore be projected through the
    /// common StateSnapshot interface.
    pub fn is_common_field(&self, name: &str) -> bool {
        if self.layouts.is_empty() {
            return self.fields.iter().any(|field| field.name == name);
        }
        let declarations = self
            .layouts
            .iter()
            .map(|layout| layout.fields.iter().find(|field| field.name == name))
            .collect::<Option<Vec<_>>>();
        declarations.is_some_and(|declarations| {
            let mut annotation = None;
            declarations.iter().all(|field| match field.annotation {
                Some(found) if annotation.is_some_and(|expected| expected != found) => false,
                Some(found) => {
                    annotation = Some(found);
                    true
                }
                None => true,
            })
        })
    }

    pub fn common_fields(&self) -> impl Iterator<Item = &StateField> {
        self.canonical_fields()
            .iter()
            .filter(|field| self.is_common_field(&field.name))
    }
}

#[derive(Debug, Clone)]
pub struct StateLayoutDecl {
    pub variant: EnumVariantId,
    pub fields: Vec<StateField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateProviderRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateField {
    pub id: ValueId,
    pub name: String,
    pub documentation: Option<String>,
    pub annotation: Option<TypeRef>,
    pub source: StateSource,
    pub transform: Option<StateTransform>,
    pub span: Span,
}

/// An optional ordinary expression that selects the value committed for one
/// successfully read state-field candidate.
#[derive(Debug, Clone)]
pub struct StateTransform {
    /// Implicit `value` binding containing the newly read candidate.
    pub value: ValueId,
    pub expression: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StateSource {
    Pointer(PointerPath),
    Expression(Expr),
}

#[derive(Debug, Clone)]
pub struct PointerPath {
    /// Exact `at` keyword for the ordinary state-field DSL. Legacy recovered
    /// forms that did not write the keyword retain `None`.
    pub at_span: Option<Span>,
    pub module: Option<String>,
    pub offsets: Vec<u64>,
    pub decoder: Option<StateMemoryDecoder>,
}

/// A bounded interpretation applied after resolving a state pointer path.
///
/// The ordinary expression API exposes the same operation directly. This is
/// only compact state-layout syntax; it is not a separate string type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMemoryDecoder {
    /// Reads at most `max_bytes`, stops at the first NUL byte, and requires
    /// the resulting bytes to be valid UTF-8.
    Utf8 { max_bytes: u32, span: Span },
    /// Reads at most `max_units` little-endian UTF-16 code units, stops at the
    /// first NUL unit, and replaces malformed surrogate sequences.
    Utf16Le { max_units: u32, span: Span },
}

#[derive(Debug, Clone)]
pub struct SettingDecl {
    pub id: ValueId,
    pub name: String,
    pub description: String,
    pub tooltip: Option<String>,
    /// Stable key used by the host settings map and data-driven lookup. When
    /// absent, the source identifier remains the key.
    pub external_key: Option<SettingExternalKey>,
    pub kind: SettingKind,
    pub span: Span,
}

impl SettingDecl {
    pub fn runtime_key(&self) -> &str {
        match &self.external_key {
            Some(key) => &key.value,
            None => &self.name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingExternalKey {
    pub value: String,
    pub keyword_span: Span,
    pub span: Span,
}

impl SettingExternalKey {
    pub fn span(&self) -> Span {
        self.span
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
        keyword_span: Span,
        enumeration: EnumReference,
        default_variant: String,
        options: Vec<SettingChoiceOption>,
    },
    File {
        keyword_span: Span,
        filters: Vec<SettingFileFilter>,
    },
}

#[derive(Debug, Clone)]
pub struct SettingChoiceOption {
    pub id: SettingChoiceOptionId,
    pub variant: String,
    pub description: String,
    pub default_span: Option<Span>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum SettingFileFilter {
    Name {
        description: Option<String>,
        pattern: String,
    },
    Mime {
        value: String,
        keyword_span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct VariableDecl {
    pub id: ValueId,
    pub name: String,
    pub name_span: Span,
    pub documentation: Option<String>,
    pub mutable: bool,
    pub debug_only: bool,
    pub annotation: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    Setup,
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
            "setup" => Self::Setup,
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
            Self::Setup => "setup",
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
    For {
        binding: ForBinding,
        in_span: Span,
        /// Compiler-owned storage for the iterable, which guarantees that the
        /// source expression is evaluated exactly once.
        iterable_value: ValueId,
        /// Compiler-owned `u32` index storage used by lowering.
        index_value: ValueId,
        iterable: Expr,
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
        /// Completes the surrounding function or action with the suspended
        /// operation's value instead of continuing with the next statement.
        returns: bool,
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
    pub name_span: Span,
    pub annotation: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ForBinding {
    pub id: ValueId,
    pub name: String,
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
        name: String,
        name_span: Span,
        fields: Vec<(String, Expr)>,
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
    Suspend {
        mode: SuspensionMode,
        /// Compiler-owned storage for the completed value. This gives an
        /// expression-level suspension a stable destination independent of
        /// the source context in which it appears.
        destination: ValueId,
        value: Box<Expr>,
    },
    Propagate(Box<Expr>),
    Path(Vec<String>),
    Member {
        receiver: Box<Expr>,
        name: String,
        name_span: Span,
    },
    Index {
        receiver: Box<Expr>,
        index: Box<Expr>,
        bracket_span: Span,
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
        receiver: Option<Box<Expr>>,
        type_arguments: Vec<TypeRef>,
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

#[derive(Clone)]
pub struct PatternBinding {
    pub id: ValueId,
    pub name: String,
    pub name_span: Span,
}

impl std::fmt::Debug for PatternBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PatternBinding")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Enum {
        enumeration: EnumReference,
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
    Core(PrimitiveType),
    /// A source-written nominal type name. Standard-library identity is
    /// resolved after parsing rather than embedded into syntax.
    Named(TypeNameId),
    /// IDs into the parsed program's interned constructed type-expression
    /// tables. Allocating these while parsing preserves syntax sharing without
    /// performing semantic type inference or layout allocation.
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
    Async(AsyncTypeId),
    Application(TypeApplicationId),
}

impl TypeRef {
    pub const fn core(core: PrimitiveType) -> Self {
        Self::Core(core)
    }

    pub fn core_type(self) -> Option<PrimitiveType> {
        match self {
            Self::Core(core) => Some(core),
            _ => None,
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        PrimitiveType::parse(name).map(Self::Core)
    }

    pub fn is_integer(self) -> bool {
        self.core_type().is_some_and(PrimitiveType::is_integer)
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(core) => core.fmt(f),
            Self::Named(id) => write!(f, "type-name#{id}"),
            Self::Array(id) => write!(f, "Array#{id}"),
            Self::Option(id) => write!(f, "Option#{id}"),
            Self::Result(id) => write!(f, "Result#{id}"),
            Self::Async(id) => write!(f, "Async#{id}"),
            Self::Application(id) => write!(f, "Application#{id}"),
        }
    }
}
