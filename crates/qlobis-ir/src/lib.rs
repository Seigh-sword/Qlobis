//! Mid-level IR for the native pipeline: `AST -> IR -> C++`.
//!
//! The IR is a typed, structured tree (functions, locals, statements) that is
//! easier to optimize than the AST: every expression carries its resolved
//! type, every call is resolved to a concrete target, and names are lowered
//! to local slot indices / global function ids.
//!
//! Passes provided: constant folding, branch simplification, dead-code
//! elimination (statements and unreachable functions).

pub mod lower;
pub mod passes;

use qlobis_ast::{BinOp, Type, UnOp};

/// A whole program in IR form.
#[derive(Debug, Default)]
pub struct IrProgram {
    pub funcs: Vec<IrFunc>,
    pub structs: Vec<IrStruct>,
    /// Emitted name of `main`.
    pub entry: Option<String>,
}

#[derive(Debug)]
pub struct IrStruct {
    pub name: String,
    pub generics: Vec<String>,
    pub fields: Vec<(String, Type)>,
    /// Methods belong to the struct (inherent impls and trait impls).
    pub methods: Vec<IrFunc>,
}

#[derive(Debug)]
pub struct IrFunc {
    /// Emitted (unique) name.
    pub name: String,
    /// Original `module::name` (for diagnostics/comments).
    pub qual_name: String,
    pub params: Vec<(String, Type)>,
    /// All locals: `0..params.len()` are the parameters, in order.
    pub locals: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Vec<Stmt>,
    pub is_async: bool,
    /// Generic parameters of this function (own generics).
    pub generics: Vec<String>,
    /// Set for methods: the struct they belong to.
    pub self_type: Option<Type>,
    pub doc: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(u32, RValue),
    /// `locals[slot] = value`
    Assign(u32, RValue),
    /// `recv[index] = value`
    AssignIndex(Box<RValue>, Box<RValue>, Box<RValue>),
    /// `recv.field = value`
    AssignField(Box<RValue>, String, Box<RValue>),
    /// `locals[slot] op= value`
    AssignCompound(u32, BinOp, Box<RValue>),
    Expr(RValue),
    /// A bare `{ ... }` scope.
    Scope(Vec<Stmt>),
    If(RValue, Vec<Stmt>, Vec<Stmt>),
    While(RValue, Vec<Stmt>),
    /// Iterate `iter` binding each element to `slot`.
    For(u32, RValue, Vec<Stmt>),
    Return(Option<RValue>),
    Throw(RValue),
    Try {
        body: Vec<Stmt>,
        /// Catch binding slot (if any).
        var: Option<u32>,
        catch: Vec<Stmt>,
    },
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub enum RValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// String with interpolations: literal parts and computed pieces.
    Str(Vec<StrPiece>),
    None,
    Local(u32),
    FuncRef(String),
    List(Vec<RValue>, Type),
    Map(Vec<(RValue, RValue)>, Type),
    StructCtor(String, Vec<RValue>, Type),
    Binary(BinOp, Box<RValue>, Box<RValue>, Type),
    Unary(UnOp, Box<RValue>, Type),
    Call(CallTarget, Vec<RValue>, Type),
    Method {
        recv: Box<RValue>,
        name: String,
        args: Vec<RValue>,
        ret: Type,
        /// `Some` when this is a built-in method on the receiver type.
        builtin: bool,
    },
    Index(Box<RValue>, Box<RValue>, Type),
    Field(Box<RValue>, String, Type),
    Await(Box<RValue>, Type),
}

#[derive(Debug, Clone)]
pub enum StrPiece {
    Lit(String),
    /// Interpolated value; stringified at emission.
    Expr(RValue),
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    /// Emitted function name.
    Func(String),
    /// Compiler intrinsic (`__println`, ...).
    Intrinsic(String),
    /// Built-in conversion / utility (`int`, `range`, ...).
    Builtin(String),
    /// Raw host call (escape hatch passthrough).
    Host(String),
    /// Call a first-class function value; `args[0]` is the callee.
    Indirect,
}

impl RValue {
    /// The type of this value where one was recorded.
    pub fn ty(&self) -> Type {
        match self {
            RValue::Int(_) => Type::Int,
            RValue::Float(_) => Type::Float,
            RValue::Bool(_) => Type::Bool,
            RValue::None => Type::None,
            RValue::Str(_) => Type::Str,
            RValue::Local(_) => Type::Any,
            RValue::FuncRef(_) => Type::Any,
            RValue::List(_, t) => t.clone(),
            RValue::Map(_, t) => t.clone(),
            RValue::StructCtor(_, _, t) => t.clone(),
            RValue::Binary(_, _, _, t) => t.clone(),
            RValue::Unary(_, _, t) => t.clone(),
            RValue::Call(_, _, t) => t.clone(),
            RValue::Method { ret, .. } => ret.clone(),
            RValue::Index(_, _, t) => t.clone(),
            RValue::Field(_, _, t) => t.clone(),
            RValue::Await(_, t) => t.clone(),
        }
    }
}
