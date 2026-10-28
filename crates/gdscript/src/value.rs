//! Run-time values. Arrays and dictionaries are shared references, as in GDScript.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::ast::Script;

/// A handle the embedding gives to something it owns (a fighter, a projectile, a node).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjId(pub u32);

pub type ArrayRef = Arc<RwLock<Vec<Value>>>;
pub type DictRef = Arc<RwLock<Vec<(Value, Value)>>>;
pub type InstRef = Arc<RwLock<Instance>>;

#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    Vec2(f64, f64),
    /// RGBA, 0..1 each.
    Color(f64, f64, f64, f64),
    Array(ArrayRef),
    Dict(DictRef),
    /// Something the embedding owns.
    Obj(ObjId),
    /// An object made from a script.
    Inst(InstRef),
    /// A script as a value (`preload("a.gd")`, a class name): for static calls and enum access.
    Class(Arc<Script>),
    /// A scene or resource by path (`preload("a.tscn")`).
    Res(Arc<str>),
    /// A built-in library (`fixed`, `Utils` when it is native).
    Lib(&'static str),
}

/// An object made from a script: the scripts it is built from, base first, and its variables.
#[derive(Debug)]
pub struct Instance {
    pub chain: Vec<Arc<Script>>,
    pub fields: HashMap<String, Value>,
    /// The node this script is attached to, for `$Path` lookups.
    pub obj: Option<ObjId>,
    /// How many scripts at the start of `chain` the embedding implements itself (the engine's
    /// base classes). Their variables and methods go to the host first, and their code runs only
    /// for what the host does not implement.
    pub native: usize,
}

impl Instance {
    /// A variable only the native part of the chain declares (the host's to answer).
    pub fn native_member(&self, name: &str) -> bool {
        let declares = |s: &Arc<Script>| s.members.iter().any(|m| m.name == name);
        self.obj.is_some() && self.chain[..self.native].iter().any(declares) && !self.chain[self.native..].iter().any(declares)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.fields.get(name).cloned()
    }

    pub fn set(&mut self, name: &str, v: Value) {
        self.fields.insert(name.to_string(), v);
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn array(v: Vec<Value>) -> Value {
        Value::Array(Arc::new(RwLock::new(v)))
    }

    pub fn dict(v: Vec<(Value, Value)>) -> Value {
        Value::Dict(Arc::new(RwLock::new(v)))
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Vec2(x, y) => *x != 0.0 || *y != 0.0,
            Value::Array(a) => !a.read().unwrap().is_empty(),
            Value::Dict(d) => !d.read().unwrap().is_empty(),
            _ => true,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(*b as i64 as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "String",
            Value::Vec2(..) => "Vector2",
            Value::Color(..) => "Color",
            Value::Array(_) => "Array",
            Value::Dict(_) => "Dictionary",
            Value::Obj(_) => "Object",
            Value::Inst(_) => "Object",
            Value::Class(_) => "Script",
            Value::Res(_) => "Resource",
            Value::Lib(_) => "Library",
        }
    }

    /// `str(value)`.
    pub fn to_text(&self) -> String {
        match self {
            Value::Nil => "Null".into(),
            Value::Bool(b) => if *b { "True" } else { "False" }.into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.0}")
                } else {
                    let s = format!("{f:.5}");
                    s.trim_end_matches('0').trim_end_matches('.').to_string()
                }
            }
            Value::Str(s) => s.to_string(),
            Value::Vec2(x, y) => format!("({}, {})", Value::Float(*x).to_text(), Value::Float(*y).to_text()),
            Value::Color(r, g, b, a) => format!("{},{},{},{}", Value::Float(*r).to_text(), Value::Float(*g).to_text(), Value::Float(*b).to_text(), Value::Float(*a).to_text()),
            Value::Array(a) => format!("[{}]", a.read().unwrap().iter().map(Value::to_text).collect::<Vec<_>>().join(", ")),
            Value::Dict(d) => format!("{{{}}}", d.read().unwrap().iter().map(|(k, v)| format!("{}: {}", k.to_text(), v.to_text())).collect::<Vec<_>>().join(", ")),
            Value::Obj(o) => format!("[Object:{}]", o.0),
            Value::Inst(_) => "[Object]".into(),
            Value::Class(s) => format!("[Script:{}]", s.path),
            Value::Res(p) => format!("[Resource:{p}]"),
            Value::Lib(n) => format!("[{n}]"),
        }
    }
}

/// GDScript equality: numbers compare by value across int and float; containers by contents.
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => a.as_f64() == b.as_f64(),
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Vec2(a1, a2), Value::Vec2(b1, b2)) => a1 == b1 && a2 == b2,
        (Value::Color(a, b, c, d), Value::Color(e, f, g, h)) => a == e && b == f && c == g && d == h,
        (Value::Array(x), Value::Array(y)) => {
            let (x, y) = (x.read().unwrap(), y.read().unwrap());
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q))
        }
        (Value::Dict(x), Value::Dict(y)) => {
            let (x, y) = (x.read().unwrap(), y.read().unwrap());
            x.len() == y.len() && x.iter().all(|(k, v)| y.iter().any(|(k2, v2)| equal(k, k2) && equal(v, v2)))
        }
        (Value::Obj(x), Value::Obj(y)) => x == y,
        (Value::Inst(x), Value::Inst(y)) => Arc::ptr_eq(x, y),
        (Value::Res(x), Value::Res(y)) => x == y,
        _ => false,
    }
}
