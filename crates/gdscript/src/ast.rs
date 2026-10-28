//! The syntax tree.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    In,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    NodePath(String),
    Member(Box<Expr>, String),
    Call(Box<Expr>, Vec<Expr>),
    /// `.name(args)`: the inherited version of a method.
    SuperCall(String, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Unary(UOp, Box<Expr>),
    Binary(BOp, Box<Expr>, Box<Expr>),
    /// `then if cond else other`
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Is(Box<Expr>, Box<Expr>),
    /// `yield(...)`: parsed, never run.
    Yield,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Wildcard,
    Bind(String),
    Value(Expr),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr, usize),
    Var(String, Option<Expr>, usize),
    Const(String, Expr, usize),
    Assign { target: Expr, op: Option<BOp>, value: Expr, line: usize },
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    While(Expr, Vec<Stmt>),
    For(String, Expr, Vec<Stmt>),
    Match(Expr, Vec<(Vec<Pattern>, Vec<Stmt>)>),
    Return(Option<Expr>),
    Break,
    Continue,
    Pass,
    Assert(Expr),
}

#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub params: Vec<(String, Option<Expr>)>,
    pub body: Vec<Stmt>,
    pub is_static: bool,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub init: Option<Expr>,
    pub exported: bool,
    pub onready: bool,
}

#[derive(Clone, Debug)]
pub enum Extends {
    /// A class name (`CharacterState`) or a built-in (`Node`).
    Class(String),
    /// A script path (`res://characters/states/Idle.gd`).
    Path(String),
}

#[derive(Clone, Debug, Default)]
pub struct Script {
    pub path: String,
    pub extends: Option<Extends>,
    pub class_name: Option<String>,
    pub members: Vec<Member>,
    pub consts: Vec<(String, Expr)>,
    pub funcs: HashMap<String, Arc<Func>>,
    pub enums: Vec<(Option<String>, Vec<(String, Option<Expr>)>)>,
    pub signals: Vec<String>,
    pub inner: Vec<(String, Arc<Script>)>,
}
