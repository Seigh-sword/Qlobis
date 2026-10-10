//! AST node definitions for the Qlobis language.

use crate::{Span, Type};

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Ident { name: name.into(), span }
    }
}

/// Syntactic type as written by the programmer; resolved to [`Type`] by the
/// type checker.
#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// `int`, `list<T>`, `map<K, V>`, user types, generic parameters...
    Named { name: String, args: Vec<TypeExpr>, span: Span },
    /// `fn(T1, T2) -> R`
    Fn { params: Vec<TypeExpr>, ret: Box<TypeExpr>, span: Span },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named { span, .. } | TypeExpr::Fn { span, .. } => *span,
        }
    }

    pub fn named(name: impl Into<String>, span: Span) -> Self {
        TypeExpr::Named { name: name.into(), args: Vec::new(), span }
    }
}

/// A piece of a (possibly interpolated) string literal.
#[derive(Clone, Debug)]
pub enum StrPart {
    Lit(String),
    Interp(Expr),
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Import {
    /// e.g. `"std::io"`, `"my_app/util"`, `"my_json"`.
    pub path: String,
    /// Optional `as alias` binding (defaults to the last path segment).
    pub alias: Option<Ident>,
    pub doc: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Program {
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
    pub file: crate::FileId,
}

#[derive(Clone, Debug)]
pub enum Item {
    Func(FuncDef),
    Struct(StructDef),
    Trait(TraitDef),
    Impl(ImplDef),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Func(f) => f.span,
            Item::Struct(s) => s.span,
            Item::Trait(t) => t.span,
            Item::Impl(i) => i.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub is_self: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FuncDef {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// `None` for trait method signatures without a body.
    pub body: Option<Block>,
    /// `@target(...)` restriction; `None` means "all targets".
    pub target: Option<Vec<Ident>>,
    pub is_async: bool,
    pub doc: Vec<String>,
    pub span: Span,
}

impl FuncDef {
    pub fn is_target_restricted(&self) -> bool {
        self.target.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub doc: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub fields: Vec<Field>,
    pub doc: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TraitDef {
    pub name: Ident,
    pub methods: Vec<FuncDef>,
    pub doc: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ImplDef {
    /// `impl Trait for Type` vs inherent `impl Type`.
    pub trait_name: Option<Ident>,
    pub self_ty: TypeExpr,
    pub methods: Vec<FuncDef>,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Statements & expressions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        /// Compound assignment operator (`None` = plain `=`).
        op: Option<BinOp>,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then: Block,
        otherwise: Option<Block>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    For {
        var: Ident,
        iter: Expr,
        body: Block,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Throw {
        value: Expr,
        span: Span,
    },
    Try {
        body: Block,
        catch: Option<Catch>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    Block(Block),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::For { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Throw { span, .. }
            | Stmt::Try { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::Expr { span, .. } => *span,
            Stmt::Block(b) => b.span,
        }
    }
}

/// `try { ... } throw err { ... }` — the catch clause.
#[derive(Clone, Debug)]
pub struct Catch {
    pub name: Ident,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }

    /// True for comparison operators (used by the type checker).
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }

    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
        )
    }

    pub fn is_bitwise(self) -> bool {
        matches!(self, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    None(Span),
    Str(Vec<StrPart>, Span),
    Ident(Ident),
    List(Vec<Expr>, Span),
    Map(Vec<(Expr, Expr)>, Span),
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `recv.name(args)` — method invocation.
    Method {
        recv: Box<Expr>,
        name: Ident,
        args: Vec<Expr>,
        span: Span,
    },
    /// `recv.name` — field / module member access.
    Member {
        recv: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Index {
        recv: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
        span: Span,
    },
    Await {
        expr: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s)
            | Expr::Float(_, s)
            | Expr::Bool(_, s)
            | Expr::None(s)
            | Expr::Str(_, s) => *s,
            Expr::Ident(i) => i.span,
            Expr::List(_, s) | Expr::Map(_, s) => *s,
            Expr::Call { span, .. }
            | Expr::Method { span, .. }
            | Expr::Member { span, .. }
            | Expr::Index { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Await { span, .. } => *span,
        }
    }

    /// True when the expression can appear on the left of `=`.
    pub fn is_lvalue(&self) -> bool {
        matches!(self, Expr::Ident(_) | Expr::Member { .. } | Expr::Index { .. })
    }
}

/// Pretty-print a type expression (used in diagnostics and codegen).
pub fn type_expr_to_string(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                format!(
                    "{}<{}>",
                    name,
                    args.iter().map(type_expr_to_string).collect::<Vec<_>>().join(", ")
                )
            }
        }
        TypeExpr::Fn { params, ret, .. } => format!(
            "fn({}) -> {}",
            params.iter().map(type_expr_to_string).collect::<Vec<_>>().join(", "),
            type_expr_to_string(ret)
        ),
    }
}

/// Convenience: `Type` from a simple named type expression.
pub fn simple_type(name: &str) -> Type {
    match name {
        "int" => Type::Int,
        "float" => Type::Float,
        "str" => Type::Str,
        "bool" => Type::Bool,
        "none" => Type::None,
        "any" => Type::Any,
        other => Type::Struct(other.to_string(), Vec::new()),
    }
}
