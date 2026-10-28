//! A deterministic, sandboxed interpreter for the GDScript the original game and its Workshop
//! mods use. It runs script code against a [`Host`] the embedding provides (the simulation);
//! scripts can touch nothing else: no files, network, clock, global randomness or scene tree.

mod ast;
mod fixedlib;
mod lexer;
mod loader;
mod parser;
mod value;
mod vm;

pub use ast::{Expr, Extends, Func, Script, Stmt};
pub use loader::{FsLoader, Loader};
pub use parser::parse;
pub use value::{equal, ArrayRef, DictRef, InstRef, Instance, ObjId, Value};
pub use vm::Vm;

/// The original's `fixed` text numbers as raw 16.16 values and back.
pub mod fixed {
    pub use crate::fixedlib::{call, raw, text};
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Parse { line: usize, msg: String },
    Runtime { msg: String },
    /// A function or feature the interpreter does not provide (collected into import reports).
    Unsupported(String),
    /// The script ran longer than its step budget.
    Budget,
}

impl Error {
    pub fn runtime(msg: impl Into<String>) -> Error {
        Error::Runtime { msg: msg.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse { line, msg } => write!(f, "line {line}: {msg}"),
            Error::Runtime { msg } => write!(f, "{msg}"),
            Error::Unsupported(what) => write!(f, "not supported: {what}"),
            Error::Budget => write!(f, "the script ran too long"),
        }
    }
}

impl std::error::Error for Error {}

/// What the embedding provides: the objects scripts talk to.
pub trait Host {
    /// A property of an object (`host.hp`).
    fn get(&mut self, obj: ObjId, name: &str) -> Result<Value, Error>;
    fn set(&mut self, obj: ObjId, name: &str, v: Value) -> Result<(), Error>;
    /// A method of an object (`host.apply_force(1, 2)`).
    fn call(&mut self, obj: ObjId, method: &str, args: &[Value]) -> Result<Value, Error>;
    /// A singleton by name (`Global`, `Network`...), or None.
    fn global(&mut self, _name: &str) -> Option<Value> {
        None
    }
    /// A free function the interpreter does not have itself (`randi_range`, `print`...).
    fn global_call(&mut self, name: &str, _args: &[Value]) -> Result<Value, Error> {
        Err(Error::Unsupported(name.to_string()))
    }
    /// The script instance attached to a host object (a fighter's own script), whose variables
    /// and methods scripts reach through that object.
    fn script(&mut self, _obj: ObjId) -> Option<InstRef> {
        None
    }
    /// `$Path` from the node a script is attached to.
    fn node(&mut self, _from: Option<ObjId>, path: &str) -> Result<Value, Error> {
        Err(Error::Unsupported(format!("${path}")))
    }
}
