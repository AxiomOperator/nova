//! Owned AST for the vertical slice. Expressions carry an `ExprId` so later
//! passes can attach side tables (e.g. name resolution) without mutating the tree.

use nova_diag::{FileId, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId {
    pub file: FileId,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct File {
    pub id: FileId,
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub enum Item {
    Fn(FnDecl),
    Test(TestDecl),
    Effect(EffectDecl),
}

#[derive(Debug)]
pub struct FnDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub uses: Option<UsesClause>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub struct TestDecl {
    pub name: String,
    pub name_span: Span,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub struct EffectDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub members: Vec<EffectMember>,
    pub span: Span,
}

#[derive(Debug)]
pub enum EffectMember {
    Effect(EffectDecl),
    Op(OpDecl),
}

#[derive(Debug)]
pub struct OpDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug)]
pub struct UsesClause {
    pub paths: Vec<EffectPath>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EffectPath {
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl EffectPath {
    pub fn dotted(&self) -> String {
        self.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(".")
    }
}

#[derive(Debug)]
pub struct Param {
    pub name: Ident,
    /// Required on `fn` items and ops; optional on inline handler ops.
    pub ty: Option<TypeExpr>,
}

#[derive(Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum TypeKind {
    Path {
        path: Vec<Ident>,
        args: Vec<TypeExpr>,
    },
    Optional(Box<TypeExpr>),
    /// `()` is the empty tuple.
    Tuple(Vec<TypeExpr>),
}

#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug)]
pub enum Stmt {
    Let { mutable: bool, name: Ident, ty: Option<TypeExpr>, value: Expr, span: Span },
    Assign { target: Ident, op: AssignOp, value: Expr, span: Span },
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
}

#[derive(Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<StrSeg>),
    Name(Ident),
    Field(Box<Expr>, Ident),
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_branch: Option<Box<Expr>>,
    },
    Block(Block),
    List(Vec<Expr>),
    Return(Option<Box<Expr>>),
    Handle {
        bindings: Vec<HandlerBinding>,
        body: Block,
    },
    Abort(Box<Expr>),
    /// Placeholder produced by error recovery.
    Error,
}

#[derive(Debug)]
pub enum StrSeg {
    Text(String),
    Expr(Expr),
}

#[derive(Debug)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Debug)]
pub struct HandlerBinding {
    pub effect: EffectPath,
    pub ops: Vec<InlineOp>,
    pub span: Span,
}

#[derive(Debug)]
pub struct InlineOp {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Or => "||",
            BinOp::And => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
        }
    }
}
