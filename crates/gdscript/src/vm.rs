//! The evaluator.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::ast::*;
use crate::fixedlib;
use crate::loader::Loader;
use crate::value::{equal, Instance, InstRef, ObjId, Value};
use crate::{Error, Host};

type R<T> = Result<T, Error>;

enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

/// A running function: the object it belongs to, which script of the chain it is defined in
/// (for `.method()` calls) and its local variables.
struct Frame {
    inst: Option<InstRef>,
    owner: usize,
    scopes: Vec<HashMap<String, Value>>,
}

impl Frame {
    fn lookup(&self, name: &str) -> Option<&Value> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn set_local(&mut self, name: &str, v: Value) -> bool {
        for s in self.scopes.iter_mut().rev() {
            if let Some(slot) = s.get_mut(name) {
                *slot = v;
                return true;
            }
        }
        false
    }
}

pub struct Vm {
    loader: Arc<dyn Loader>,
    steps: u64,
    depth: u32,
    /// Statements and calls a single top-level call may run.
    pub budget: u64,
    /// The script line last run (errors name it).
    line: usize,
}

const MAX_DEPTH: u32 = 64;

impl Vm {
    pub fn new(loader: Box<dyn Loader>) -> Vm {
        Vm::shared(Arc::from(loader))
    }

    /// A VM over a loader other VMs may use too.
    pub fn shared(loader: Arc<dyn Loader>) -> Vm {
        Vm { loader, steps: 0, depth: 0, budget: 200_000, line: 0 }
    }

    pub fn loader(&self) -> &dyn Loader {
        self.loader.as_ref()
    }

    /// The scripts a script is built from, base first.
    fn chain_of(&self, script: Arc<Script>) -> Vec<Arc<Script>> {
        let mut chain = vec![script.clone()];
        let mut cur = script;
        for _ in 0..16 {
            let parent = match &cur.extends {
                Some(Extends::Path(p)) => self.loader.by_path(p),
                Some(Extends::Class(c)) => self.loader.by_class(c),
                None => None,
            };
            match parent {
                Some(p) => {
                    chain.push(p.clone());
                    cur = p;
                }
                None => break,
            }
        }
        chain.reverse();
        chain
    }

    /// Makes an object from a script. `obj` is the node it is attached to, if any. Variables are
    /// initialised (not the `onready` ones: see [`Vm::ready`]).
    pub fn instantiate(&mut self, host: &mut dyn Host, script: Arc<Script>, obj: Option<ObjId>) -> R<InstRef> {
        let chain = self.chain_of(script);
        let native = chain.iter().take_while(|s| self.loader.is_native(&s.path)).count();
        let inst = Arc::new(RwLock::new(Instance { chain: chain.clone(), fields: HashMap::new(), obj, native }));
        // every member exists (null) before any is set up, so an initialiser may name one
        // declared further down, as Godot allows
        for s in &chain {
            for m in &s.members {
                inst.write().unwrap().fields.insert(m.name.clone(), Value::Nil);
            }
        }
        for (idx, s) in chain.iter().enumerate() {
            for m in &s.members {
                let v = if m.onready || m.init.is_none() {
                    Value::Nil
                } else {
                    let mut f = Frame { inst: Some(inst.clone()), owner: idx, scopes: vec![HashMap::new()] };
                    self.steps = 0;
                    match self.eval(host, &mut f, m.init.as_ref().expect("checked")) {
                        Ok(v) => v,
                        // the host stands in for the native part, whose own set-up may not run here
                        Err(_) if idx < native => Value::Nil,
                        Err(e) => return Err(e),
                    }
                };
                inst.write().unwrap().fields.insert(m.name.clone(), v);
            }
        }
        Ok(inst)
    }

    /// Evaluates the `onready` variables (call once the node the script is on can answer `$Path`).
    /// A variable that fails is left null and the rest still run; the first error is returned.
    pub fn ready(&mut self, host: &mut dyn Host, inst: &InstRef) -> R<()> {
        let (chain, native) = {
            let i = inst.read().unwrap();
            (i.chain.clone(), i.native)
        };
        let mut first = None;
        for (idx, s) in chain.iter().enumerate() {
            for m in s.members.iter().filter(|m| m.onready) {
                if let Some(init) = &m.init {
                    let mut f = Frame { inst: Some(inst.clone()), owner: idx, scopes: vec![HashMap::new()] };
                    self.steps = 0;
                    match self.eval(host, &mut f, init) {
                        Ok(v) => {
                            inst.write().unwrap().fields.insert(m.name.clone(), v);
                        }
                        // the native part's nodes may be ones the host does not have
                        Err(_) if idx < native => {}
                        Err(e) => {
                            first.get_or_insert(e);
                        }
                    }
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// The script that defines `name`, most derived first.
    fn find_method(chain: &[Arc<Script>], name: &str, below: usize) -> Option<(usize, Arc<Func>)> {
        chain.iter().enumerate().take(below).rev().find_map(|(i, s)| s.funcs.get(name).map(|f| (i, f.clone())))
    }

    /// Whether a script (not one of the native base classes) defines `name` for this object.
    pub fn has_script_method(&self, inst: &InstRef, name: &str) -> bool {
        let i = inst.read().unwrap();
        Self::find_method(&i.chain, name, i.chain.len()).is_some_and(|(owner, _)| owner >= i.native)
    }

    /// A variable as the object stores it, else a constant of its scripts (the native base
    /// classes' included), without asking the host: for a host answering what its native
    /// classes declare that it does not model itself.
    pub fn script_member(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str) -> Option<Value> {
        if let Some(v) = inst.read().unwrap().fields.get(name) {
            return Some(v.clone());
        }
        let chain = inst.read().unwrap().chain.clone();
        for (idx, s) in chain.iter().enumerate().rev() {
            if let Some((_, e)) = s.consts.iter().find(|(n, _)| n == name) {
                let mut cf = Frame { inst: Some(inst.clone()), owner: idx, scopes: vec![HashMap::new()] };
                return self.eval(host, &mut cf, e).ok();
            }
        }
        None
    }

    /// Whether `class` (a `class_name` or a script's file name) is one of the object's scripts.
    pub fn is_a(&self, inst: &InstRef, class: &str) -> bool {
        inst.read().unwrap().chain.iter().any(|s| s.class_name.as_deref() == Some(class) || s.path.rsplit('/').next().is_some_and(|f| f.trim_end_matches(".gd") == class))
    }

    pub fn has_method(&self, inst: &InstRef, name: &str) -> bool {
        let chain = inst.read().unwrap().chain.clone();
        Self::find_method(&chain, name, chain.len()).is_some()
    }

    /// Calls a method of an object made by [`Vm::instantiate`].
    pub fn call(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str, args: Vec<Value>) -> R<Value> {
        self.steps = 0;
        self.depth = 0;
        self.line = 0;
        self.call_method(host, inst, name, args, usize::MAX).map_err(|e| match e {
            Error::Runtime { msg } if self.line > 0 && !msg.contains("(line ") => Error::Runtime { msg: format!("{msg} (line {})", self.line) },
            Error::Unsupported(w) if self.line > 0 && !w.contains("(line ") => Error::Unsupported(format!("{w} (line {})", self.line)),
            e => e,
        })
    }

    fn call_method(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str, args: Vec<Value>, below: usize) -> R<Value> {
        let chain = inst.read().unwrap().chain.clone();
        let below = below.min(chain.len());
        let Some((owner, func)) = Self::find_method(&chain, name, below) else {
            return Err(Error::runtime(format!("no method `{name}`")));
        };
        let (obj, native) = {
            let i = inst.read().unwrap();
            (i.obj, i.native)
        };
        if let Some(obj) = obj.filter(|_| owner < native) {
            match host.call(obj, name, &args) {
                Err(Error::Unsupported(_)) => {}
                other => return other,
            }
        }
        self.run_func(host, Some(inst.clone()), owner, &func, args)
    }

    /// A variable of an object, asking the host for the ones its native part declares.
    fn inst_get(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str) -> R<Option<Value>> {
        let (obj, native) = {
            let i = inst.read().unwrap();
            (i.obj, i.native_member(name))
        };
        if let Some(obj) = obj.filter(|_| native) {
            match host.get(obj, name) {
                Err(Error::Unsupported(_)) => {}
                other => return other.map(Some),
            }
        }
        Ok(inst.read().unwrap().get(name))
    }

    fn inst_set(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str, v: Value) -> R<()> {
        let (obj, native) = {
            let i = inst.read().unwrap();
            (i.obj, i.native_member(name))
        };
        if let Some(obj) = obj.filter(|_| native) {
            match host.set(obj, name, v.clone()) {
                Err(Error::Unsupported(_)) => {}
                other => return other,
            }
        }
        inst.write().unwrap().set(name, v);
        Ok(())
    }

    /// The script-defined (not native) variable `name` of a host object's script, if any.
    fn script_field(inst: &Option<InstRef>, name: &str) -> Option<Value> {
        let i = inst.as_ref()?.read().unwrap();
        if i.native_member(name) {
            return None;
        }
        i.get(name)
    }

    fn obj_get(&mut self, host: &mut dyn Host, id: ObjId, name: &str) -> R<Value> {
        let script = host.script(id);
        if let Some(v) = Self::script_field(&script, name) {
            return Ok(v);
        }
        match host.get(id, name) {
            Err(Error::Unsupported(what)) => match script {
                Some(i) => match i.read().unwrap().get(name) {
                    Some(v) => Ok(v),
                    None => Err(Error::Unsupported(what)),
                },
                None => Err(Error::Unsupported(what)),
            },
            other => other,
        }
    }

    fn obj_set(&mut self, host: &mut dyn Host, id: ObjId, name: &str, v: Value) -> R<()> {
        let script = host.script(id);
        if let Some(i) = &script {
            if Self::script_field(&script, name).is_some() {
                i.write().unwrap().set(name, v);
                return Ok(());
            }
        }
        match host.set(id, name, v.clone()) {
            Err(Error::Unsupported(what)) => match script {
                Some(i) if i.read().unwrap().fields.contains_key(name) => {
                    i.write().unwrap().set(name, v);
                    Ok(())
                }
                _ => Err(Error::Unsupported(what)),
            },
            other => other,
        }
    }

    fn obj_call(&mut self, host: &mut dyn Host, id: ObjId, name: &str, args: Vec<Value>) -> R<Value> {
        let script = host.script(id);
        if let Some(i) = &script {
            let chain = i.read().unwrap().chain.clone();
            let native = i.read().unwrap().native;
            if let Some((owner, _)) = Self::find_method(&chain, name, chain.len()) {
                if owner >= native {
                    return self.call_method(host, i, name, args, usize::MAX);
                }
            }
        }
        match host.call(id, name, &args) {
            Err(Error::Unsupported(what)) => match script {
                Some(i) if self.has_method(&i, name) => self.run_native_code(host, &i, name, args),
                // Object.get / Object.set: a property by name, null when there is none
                _ if name == "get" => Ok(self.obj_get(host, id, &args.first().map(Value::to_text).unwrap_or_default()).unwrap_or(Value::Nil)),
                _ if name == "set" && args.len() == 2 => {
                    let _ = self.obj_set(host, id, &args[0].to_text(), args[1].clone());
                    Ok(Value::Nil)
                }
                _ => Err(Error::Unsupported(what)),
            },
            other => other,
        }
    }

    /// Runs the original code of a native method (after the host declined it).
    fn run_native_code(&mut self, host: &mut dyn Host, inst: &InstRef, name: &str, args: Vec<Value>) -> R<Value> {
        let chain = inst.read().unwrap().chain.clone();
        let Some((owner, func)) = Self::find_method(&chain, name, chain.len()) else {
            return Err(Error::runtime(format!("no method `{name}`")));
        };
        self.run_func(host, Some(inst.clone()), owner, &func, args)
    }

    /// A `static func` of a script (or any function that needs no object).
    pub fn call_static(&mut self, host: &mut dyn Host, script: &Arc<Script>, name: &str, args: Vec<Value>) -> R<Value> {
        let chain = self.chain_of(script.clone());
        let Some((owner, func)) = Self::find_method(&chain, name, chain.len()) else {
            return Err(Error::runtime(format!("no function `{name}`")));
        };
        let inst = Arc::new(RwLock::new(Instance { chain, fields: HashMap::new(), obj: None, native: 0 }));
        self.run_func(host, Some(inst), owner, &func, args)
    }

    fn run_func(&mut self, host: &mut dyn Host, inst: Option<InstRef>, owner: usize, func: &Arc<Func>, args: Vec<Value>) -> R<Value> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(Error::runtime("recursion too deep"));
        }
        let mut f = Frame { inst, owner, scopes: vec![HashMap::new()] };
        for (i, (pname, default)) in func.params.iter().enumerate() {
            let v = match args.get(i) {
                Some(v) => v.clone(),
                None => match default {
                    Some(d) => self.eval(host, &mut f, d)?,
                    None => Value::Nil,
                },
            };
            f.scopes[0].insert(pname.clone(), v);
        }
        let r = self.exec_block(host, &mut f, &func.body);
        self.depth -= 1;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Nil),
        }
    }

    fn tick(&mut self) -> R<()> {
        self.steps += 1;
        if self.steps > self.budget {
            return Err(Error::Budget);
        }
        Ok(())
    }

    // ---- statements -----------------------------------------------------------------------

    fn exec_block(&mut self, host: &mut dyn Host, f: &mut Frame, body: &[Stmt]) -> R<Flow> {
        f.scopes.push(HashMap::new());
        let mut out = Flow::Normal;
        for s in body {
            match self.exec(host, f, s) {
                Ok(Flow::Normal) => {}
                Ok(other) => {
                    out = other;
                    break;
                }
                Err(e) => {
                    f.scopes.pop();
                    return Err(e);
                }
            }
        }
        f.scopes.pop();
        Ok(out)
    }

    fn exec(&mut self, host: &mut dyn Host, f: &mut Frame, s: &Stmt) -> R<Flow> {
        self.tick()?;
        if let Stmt::Expr(_, l) | Stmt::Var(_, _, l) | Stmt::Const(_, _, l) | Stmt::Assign { line: l, .. } = s {
            self.line = *l;
        }
        match s {
            Stmt::Expr(e, _) => {
                self.eval(host, f, e)?;
            }
            Stmt::Pass => {}
            Stmt::Var(name, init, _) => {
                let v = match init {
                    Some(e) => self.eval(host, f, e)?,
                    None => Value::Nil,
                };
                f.scopes.last_mut().expect("a scope").insert(name.clone(), v);
            }
            Stmt::Const(name, e, _) => {
                let v = self.eval(host, f, e)?;
                f.scopes.last_mut().expect("a scope").insert(name.clone(), v);
            }
            Stmt::Assign { target, op, value, .. } => {
                let mut v = self.eval(host, f, value)?;
                if let Some(op) = op {
                    let cur = self.eval(host, f, target)?;
                    v = binop(*op, &cur, &v)?;
                }
                self.assign(host, f, target, v)?;
            }
            Stmt::If(arms, other) => {
                for (cond, body) in arms {
                    if self.eval(host, f, cond)?.truthy() {
                        return self.exec_block(host, f, body);
                    }
                }
                if let Some(body) = other {
                    return self.exec_block(host, f, body);
                }
            }
            Stmt::While(cond, body) => {
                while self.eval(host, f, cond)?.truthy() {
                    self.tick()?;
                    match self.exec_block(host, f, body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Stmt::For(var, iter, body) => {
                let items: Vec<Value> = match self.eval(host, f, iter)? {
                    Value::Array(a) => a.read().unwrap().clone(),
                    Value::Dict(d) => d.read().unwrap().iter().map(|(k, _)| k.clone()).collect(),
                    Value::Int(n) => (0..n).map(Value::Int).collect(),
                    Value::Str(s) => s.chars().map(|c| Value::str(&c.to_string())).collect(),
                    other => return Err(Error::runtime(format!("cannot loop over {}", other.type_name()))),
                };
                for item in items {
                    self.tick()?;
                    f.scopes.push(HashMap::from([(var.clone(), item)]));
                    let r = self.exec_block(host, f, body);
                    f.scopes.pop();
                    match r? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Stmt::Match(subject, arms) => {
                let v = self.eval(host, f, subject)?;
                for (pats, body) in arms {
                    let mut hit = None;
                    for p in pats {
                        match p {
                            Pattern::Wildcard => hit = Some(None),
                            Pattern::Bind(n) => hit = Some(Some((n.clone(), v.clone()))),
                            Pattern::Value(e) => {
                                if equal(&self.eval(host, f, e)?, &v) {
                                    hit = Some(None);
                                }
                            }
                        }
                        if hit.is_some() {
                            break;
                        }
                    }
                    if let Some(bind) = hit {
                        f.scopes.push(bind.into_iter().collect());
                        let r = self.exec_block(host, f, body);
                        f.scopes.pop();
                        return r;
                    }
                }
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(host, f, e)?,
                    None => Value::Nil,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Assert(e) => {
                if !self.eval(host, f, e)?.truthy() {
                    return Err(Error::runtime("assertion failed"));
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn assign(&mut self, host: &mut dyn Host, f: &mut Frame, target: &Expr, v: Value) -> R<()> {
        match target {
            Expr::Ident(name) => {
                if f.set_local(name, v.clone()) {
                    return Ok(());
                }
                match f.inst.clone() {
                    Some(i) => self.inst_set(host, &i, name, v),
                    None => Err(Error::runtime(format!("cannot assign to `{name}`"))),
                }
            }
            Expr::Member(obj, name) => {
                let o = self.eval(host, f, obj)?;
                match o {
                    Value::Inst(i) => self.inst_set(host, &i, name, v),
                    Value::Obj(id) => self.obj_set(host, id, name, v),
                    Value::Dict(d) => {
                        dict_set(&d, Value::str(name), v);
                        Ok(())
                    }
                    Value::Color(r, g, b, a) => {
                        // `c.a = 0.5` rebinds the color (and a node's `modulate.a` writes back)
                        let n = v.as_f64().ok_or_else(|| Error::runtime("Color components are numbers"))?;
                        let nv = match name.as_str() {
                            "r" => Value::Color(n, g, b, a),
                            "g" => Value::Color(r, n, b, a),
                            "b" => Value::Color(r, g, n, a),
                            "a" => Value::Color(r, g, b, n),
                            _ => return Err(Error::runtime(format!("Color has no `{name}`"))),
                        };
                        self.assign(host, f, obj, nv)
                    }
                    Value::Vec2(x, y) => {
                        // `v.x = 1` rebinds the vector variable
                        let n = v.as_f64().ok_or_else(|| Error::runtime("Vector2 components are numbers"))?;
                        let nv = match name.as_str() {
                            "x" => Value::Vec2(n, y),
                            "y" => Value::Vec2(x, n),
                            _ => return Err(Error::runtime(format!("Vector2 has no `{name}`"))),
                        };
                        self.assign(host, f, obj, nv)
                    }
                    other => Err(Error::runtime(format!("cannot set `{name}` on {}", other.type_name()))),
                }
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(host, f, obj)?;
                let i = self.eval(host, f, idx)?;
                match o {
                    Value::Array(a) => {
                        let n = i.as_i64().ok_or_else(|| Error::runtime("array index must be an integer"))?;
                        let mut a = a.write().unwrap();
                        let len = a.len() as i64;
                        let k = if n < 0 { n + len } else { n };
                        if k < 0 || k >= len {
                            return Err(Error::runtime("array index out of range"));
                        }
                        a[k as usize] = v;
                        Ok(())
                    }
                    Value::Dict(d) => {
                        dict_set(&d, i, v);
                        Ok(())
                    }
                    other => Err(Error::runtime(format!("cannot index-assign {}", other.type_name()))),
                }
            }
            other => Err(Error::runtime(format!("cannot assign to {other:?}"))),
        }
    }

    // ---- expressions ----------------------------------------------------------------------

    fn eval(&mut self, host: &mut dyn Host, f: &mut Frame, e: &Expr) -> R<Value> {
        self.tick()?;
        Ok(match e {
            Expr::Nil => Value::Nil,
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Int(i) => Value::Int(*i),
            Expr::Float(x) => Value::Float(*x),
            Expr::Str(s) => Value::str(s),
            Expr::Yield => return Err(Error::Unsupported("yield".into())),
            Expr::NodePath(p) => {
                let from = f.inst.as_ref().and_then(|i| i.read().unwrap().obj);
                host.node(from, p)?
            }
            Expr::Ident(name) => self.ident(host, f, name)?,
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.eval(host, f, i)?);
                }
                Value::array(out)
            }
            Expr::Dict(items) => {
                let mut out: Vec<(Value, Value)> = Vec::new();
                for (k, v) in items {
                    let key = self.eval(host, f, k)?;
                    let val = self.eval(host, f, v)?;
                    match out.iter_mut().find(|(k2, _)| equal(k2, &key)) {
                        Some(slot) => slot.1 = val,
                        None => out.push((key, val)),
                    }
                }
                Value::dict(out)
            }
            Expr::Unary(op, x) => {
                let v = self.eval(host, f, x)?;
                match op {
                    UOp::Not => Value::Bool(!v.truthy()),
                    UOp::Neg => match v {
                        Value::Int(i) => Value::Int(-i),
                        Value::Float(x) => Value::Float(-x),
                        Value::Vec2(x, y) => Value::Vec2(-x, -y),
                        other => return Err(Error::runtime(format!("cannot negate {}", other.type_name()))),
                    },
                    UOp::BitNot => Value::Int(!v.as_i64().ok_or_else(|| Error::runtime("~ needs an integer"))?),
                }
            }
            Expr::Binary(BOp::And, a, b) => {
                let l = self.eval(host, f, a)?;
                if !l.truthy() {
                    Value::Bool(false)
                } else {
                    Value::Bool(self.eval(host, f, b)?.truthy())
                }
            }
            Expr::Binary(BOp::Or, a, b) => {
                let l = self.eval(host, f, a)?;
                if l.truthy() {
                    Value::Bool(true)
                } else {
                    Value::Bool(self.eval(host, f, b)?.truthy())
                }
            }
            Expr::Binary(op, a, b) => {
                let l = self.eval(host, f, a)?;
                let r = self.eval(host, f, b)?;
                binop(*op, &l, &r)?
            }
            Expr::Ternary(then, cond, other) => {
                if self.eval(host, f, cond)?.truthy() {
                    self.eval(host, f, then)?
                } else {
                    self.eval(host, f, other)?
                }
            }
            Expr::Is(l, r) => {
                let v = self.eval(host, f, l)?;
                let rhs = match &**r {
                    Expr::Ident(n) => self.ident(host, f, n).ok(),
                    other => self.eval(host, f, other).ok(),
                };
                match (&v, rhs) {
                    (Value::Inst(i), Some(Value::Class(c))) => Value::Bool(i.read().unwrap().chain.iter().any(|s| s.path == c.path)),
                    (Value::Obj(id), _) => {
                        let name = match &**r {
                            Expr::Ident(n) => n.clone(),
                            _ => String::new(),
                        };
                        host.call(*id, "is_class_name", &[Value::str(&name)]).unwrap_or(Value::Bool(false))
                    }
                    _ => Value::Bool(false),
                }
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(host, f, obj)?;
                let i = self.eval(host, f, idx)?;
                index(&o, &i)?
            }
            Expr::Member(obj, name) => {
                let o = self.eval(host, f, obj)?;
                self.member(host, &o, name)?
            }
            Expr::SuperCall(name, args) => {
                let argv = self.eval_args(host, f, args)?;
                let Some(inst) = f.inst.clone() else { return Err(Error::runtime("`.` call outside an object")) };
                let owner = f.owner;
                if Self::find_method(&inst.read().unwrap().chain.clone(), name, owner).is_none() {
                    return Ok(Value::Nil); // the native base has nothing to run
                }
                self.call_method(host, &inst, name, argv, owner)?
            }
            Expr::Call(callee, args) => {
                let argv = self.eval_args(host, f, args)?;
                match &**callee {
                    Expr::Ident(name) => self.call_ident(host, f, name, argv)?,
                    Expr::Member(obj, name) => {
                        let o = self.eval(host, f, obj)?;
                        self.call_on(host, &o, name, argv)?
                    }
                    other => return Err(Error::runtime(format!("cannot call {other:?}"))),
                }
            }
        })
    }

    fn eval_args(&mut self, host: &mut dyn Host, f: &mut Frame, args: &[Expr]) -> R<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            out.push(self.eval(host, f, a)?);
        }
        Ok(out)
    }

    fn ident(&mut self, host: &mut dyn Host, f: &mut Frame, name: &str) -> R<Value> {
        if let Some(v) = f.lookup(name) {
            return Ok(v.clone());
        }
        if name == "self" {
            if let Some(i) = &f.inst {
                return Ok(Value::Inst(i.clone()));
            }
        }
        if let Some(inst) = f.inst.clone() {
            if let Some(v) = self.inst_get(host, &inst, name)? {
                return Ok(v);
            }
            let chain = inst.read().unwrap().chain.clone();
            for (idx, s) in chain.iter().enumerate().rev() {
                if let Some((_, e)) = s.consts.iter().find(|(n, _)| n == name) {
                    let mut cf = Frame { inst: f.inst.clone(), owner: idx, scopes: vec![HashMap::new()] };
                    return self.eval(host, &mut cf, e);
                }
                for (en, vals) in &s.enums {
                    if en.as_deref() == Some(name) {
                        return Ok(Value::dict(self.enum_values(host, f, vals)?.into_iter().map(|(k, v)| (Value::str(&k), v)).collect()));
                    }
                    if en.is_none() {
                        if let Some(v) = self.enum_values(host, f, vals)?.into_iter().find(|(k, _)| k == name) {
                            return Ok(v.1);
                        }
                    }
                }
                if let Some((_, c)) = s.inner.iter().find(|(n, _)| n == name) {
                    return Ok(Value::Class(c.clone()));
                }
            }
            // a property of the node the script is attached to (`name`, `position`...)
            let obj = inst.read().unwrap().obj;
            if let Some(obj) = obj {
                match host.get(obj, name) {
                    Err(Error::Unsupported(_)) => {}
                    other => return other,
                }
            }
        }
        match name {
            "PI" => return Ok(Value::Float(std::f64::consts::PI)),
            "TAU" => return Ok(Value::Float(std::f64::consts::TAU)),
            "INF" => return Ok(Value::Float(f64::INFINITY)),
            "NAN" => return Ok(Value::Float(f64::NAN)),
            "fixed" => return Ok(Value::Lib("fixed")),
            "Vector2" => return Ok(Value::Lib("Vector2")),
            "Color" => return Ok(Value::Lib("Color")),
            _ => {}
        }
        if let Some(s) = self.loader.autoload(name) {
            return Ok(Value::Class(s));
        }
        if let Some(v) = host.global(name) {
            return Ok(v);
        }
        if let Some(s) = self.loader.by_class(name) {
            return Ok(Value::Class(s));
        }
        Err(Error::runtime(format!("undefined name `{name}`")))
    }

    fn enum_values(&mut self, host: &mut dyn Host, f: &mut Frame, vals: &[(String, Option<Expr>)]) -> R<Vec<(String, Value)>> {
        let mut next = 0i64;
        let mut out = Vec::new();
        for (n, e) in vals {
            if let Some(e) = e {
                next = self.eval(host, f, e)?.as_i64().unwrap_or(next);
            }
            out.push((n.clone(), Value::Int(next)));
            next += 1;
        }
        Ok(out)
    }

    fn member(&mut self, host: &mut dyn Host, o: &Value, name: &str) -> R<Value> {
        match o {
            Value::Inst(i) => {
                if let Some(v) = self.inst_get(host, i, name)? {
                    return Ok(v);
                }
                let chain = i.read().unwrap().chain.clone();
                for (idx, s) in chain.iter().enumerate().rev() {
                    if let Some((_, e)) = s.consts.iter().find(|(n, _)| n == name) {
                        let mut cf = Frame { inst: Some(i.clone()), owner: idx, scopes: vec![HashMap::new()] };
                        return self.eval(host, &mut cf, e);
                    }
                }
                Err(Error::runtime(format!("no property `{name}`")))
            }
            Value::Obj(id) => self.obj_get(host, *id, name),
            Value::Color(r, g, b, a) => match name {
                "r" => Ok(Value::Float(*r)),
                "g" => Ok(Value::Float(*g)),
                "b" => Ok(Value::Float(*b)),
                "a" => Ok(Value::Float(*a)),
                _ => Err(Error::runtime(format!("Color has no `{name}`"))),
            },
            Value::Lib("Color") => Ok(match name {
                "white" => Value::Color(1.0, 1.0, 1.0, 1.0),
                "black" => Value::Color(0.0, 0.0, 0.0, 1.0),
                "red" => Value::Color(1.0, 0.0, 0.0, 1.0),
                "transparent" => Value::Color(1.0, 1.0, 1.0, 0.0),
                _ => Value::Color(1.0, 1.0, 1.0, 1.0),
            }),
            Value::Vec2(x, y) => match name {
                "x" => Ok(Value::Float(*x)),
                "y" => Ok(Value::Float(*y)),
                _ => Err(Error::runtime(format!("Vector2 has no `{name}`"))),
            },
            Value::Dict(d) => Ok(dict_get(d, &Value::str(name)).unwrap_or(Value::Nil)),
            Value::Lib("Vector2") => Ok(match name {
                "ZERO" => Value::Vec2(0.0, 0.0),
                "ONE" => Value::Vec2(1.0, 1.0),
                "LEFT" => Value::Vec2(-1.0, 0.0),
                "RIGHT" => Value::Vec2(1.0, 0.0),
                "UP" => Value::Vec2(0.0, -1.0),
                "DOWN" => Value::Vec2(0.0, 1.0),
                _ => return Err(Error::Unsupported(format!("Vector2.{name}"))),
            }),
            Value::Class(s) => {
                if let Some((_, e)) = s.consts.iter().find(|(n, _)| n == name) {
                    let inst = Arc::new(RwLock::new(Instance { chain: self.chain_of(s.clone()), fields: HashMap::new(), obj: None, native: 0 }));
                    let mut cf = Frame { inst: Some(inst), owner: 0, scopes: vec![HashMap::new()] };
                    return self.eval(host, &mut cf, e);
                }
                for (en, vals) in &s.enums {
                    if en.as_deref() == Some(name) {
                        let inst = Arc::new(RwLock::new(Instance { chain: self.chain_of(s.clone()), fields: HashMap::new(), obj: None, native: 0 }));
                        let mut cf = Frame { inst: Some(inst), owner: 0, scopes: vec![HashMap::new()] };
                        return Ok(Value::dict(self.enum_values(host, &mut cf, vals)?.into_iter().map(|(k, v)| (Value::str(&k), v)).collect()));
                    }
                }
                Err(Error::runtime(format!("script has no `{name}`")))
            }
            other => Err(Error::runtime(format!("{} has no property `{name}`", other.type_name()))),
        }
    }

    fn call_ident(&mut self, host: &mut dyn Host, f: &mut Frame, name: &str, args: Vec<Value>) -> R<Value> {
        // a method of the current object (or of one it extends)
        if let Some(inst) = f.inst.clone() {
            if self.has_method(&inst, name) {
                return self.call_method(host, &inst, name, args, usize::MAX);
            }
        }
        if let Some(v) = builtin(name, &args)? {
            return Ok(v);
        }
        match name {
            "preload" | "load" => {
                let Some(Value::Str(p)) = args.first() else { return Err(Error::runtime("preload needs a path")) };
                if p.ends_with(".gd") {
                    return self.loader.by_path(p).map(Value::Class).ok_or_else(|| Error::runtime(format!("script {p} not found")));
                }
                Ok(Value::Res(Arc::from(p.strip_prefix("res://").unwrap_or(p))))
            }
            _ => {
                // a method of the node the script is attached to
                let obj = f.inst.as_ref().and_then(|i| i.read().unwrap().obj);
                if let Some(obj) = obj {
                    match host.call(obj, name, &args) {
                        Err(Error::Unsupported(_)) => {}
                        other => return other,
                    }
                }
                host.global_call(name, &args)
            }
        }
    }

    fn call_on(&mut self, host: &mut dyn Host, o: &Value, name: &str, args: Vec<Value>) -> R<Value> {
        match o {
            Value::Inst(i) => {
                if self.has_method(i, name) {
                    self.call_method(host, i, name, args, usize::MAX)
                } else if name == "get" {
                    Ok(self.inst_get(host, i, &args.first().map(Value::to_text).unwrap_or_default())?.unwrap_or(Value::Nil))
                } else if name == "has_method" {
                    Ok(Value::Bool(self.has_method(i, &args.first().map(Value::to_text).unwrap_or_default())))
                } else {
                    Err(Error::runtime(format!("no method `{name}`")))
                }
            }
            Value::Obj(id) => self.obj_call(host, *id, name, args),
            Value::Lib(lib) => match *lib {
                "fixed" => fixedlib::call(name, &args),
                other => Err(Error::Unsupported(format!("{other}.{name}"))),
            },
            Value::Class(s) => match name {
                "new" => {
                    let inst = self.instantiate(host, s.clone(), None)?;
                    if self.has_method(&inst, "_init") {
                        self.call_method(host, &inst, "_init", args, usize::MAX)?;
                    }
                    Ok(Value::Inst(inst))
                }
                _ => self.call_static(host, s, name, args),
            },
            Value::Res(p) => match name {
                "instance" => host.call(ObjId(0), "instance_scene", &[Value::Str(p.clone())]),
                _ => Err(Error::Unsupported(format!("Resource.{name}"))),
            },
            other => method_of_value(other, name, &args),
        }
    }
}

fn dict_get(d: &Arc<RwLock<Vec<(Value, Value)>>>, k: &Value) -> Option<Value> {
    d.read().unwrap().iter().find(|(k2, _)| equal(k2, k)).map(|(_, v)| v.clone())
}

fn dict_set(d: &Arc<RwLock<Vec<(Value, Value)>>>, k: Value, v: Value) {
    let mut d = d.write().unwrap();
    match d.iter_mut().find(|(k2, _)| equal(k2, &k)) {
        Some(slot) => slot.1 = v,
        None => d.push((k, v)),
    }
}

fn index(o: &Value, i: &Value) -> R<Value> {
    match o {
        Value::Array(a) => {
            let n = i.as_i64().ok_or_else(|| Error::runtime("array index must be an integer"))?;
            let a = a.read().unwrap();
            let k = if n < 0 { n + a.len() as i64 } else { n };
            a.get(k as usize).filter(|_| k >= 0).cloned().ok_or_else(|| Error::runtime("array index out of range"))
        }
        Value::Dict(d) => dict_get(d, i).ok_or_else(|| Error::runtime(format!("no key {}", i.to_text()))),
        Value::Str(s) => {
            let n = i.as_i64().ok_or_else(|| Error::runtime("string index must be an integer"))?;
            s.chars().nth(n as usize).map(|c| Value::str(&c.to_string())).ok_or_else(|| Error::runtime("string index out of range"))
        }
        Value::Vec2(x, y) => match i.as_i64() {
            Some(0) => Ok(Value::Float(*x)),
            Some(1) => Ok(Value::Float(*y)),
            _ => Err(Error::runtime("Vector2 index out of range")),
        },
        other => Err(Error::runtime(format!("cannot index {}", other.type_name()))),
    }
}

fn num_pair(a: &Value, b: &Value) -> Option<(f64, f64)> {
    Some((a.as_f64()?, b.as_f64()?))
}

fn binop(op: BOp, a: &Value, b: &Value) -> R<Value> {
    use Value::*;
    let bad = || Error::runtime(format!("cannot apply {op:?} to {} and {}", a.type_name(), b.type_name()));
    Ok(match op {
        BOp::Eq => Bool(equal(a, b)),
        BOp::Ne => Bool(!equal(a, b)),
        BOp::Lt | BOp::Gt | BOp::Le | BOp::Ge => {
            let ord = match (a, b) {
                (Str(x), Str(y)) => Some(x.cmp(y)),
                _ => num_pair(a, b).and_then(|(x, y)| x.partial_cmp(&y)),
            };
            let Some(ord) = ord else { return Err(bad()) };
            Bool(match op {
                BOp::Lt => ord.is_lt(),
                BOp::Gt => ord.is_gt(),
                BOp::Le => ord.is_le(),
                _ => ord.is_ge(),
            })
        }
        BOp::In => match b {
            Array(arr) => Bool(arr.read().unwrap().iter().any(|x| equal(x, a))),
            Dict(d) => Bool(d.read().unwrap().iter().any(|(k, _)| equal(k, a))),
            Str(s) => match a {
                Str(t) => Bool(s.contains(&**t)),
                _ => return Err(bad()),
            },
            _ => return Err(bad()),
        },
        BOp::Add => match (a, b) {
            (Int(x), Int(y)) => Int(x.wrapping_add(*y)),
            (Str(x), Str(y)) => Value::str(&format!("{x}{y}")),
            (Vec2(a1, a2), Vec2(b1, b2)) => Vec2(a1 + b1, a2 + b2),
            (Array(x), Array(y)) => Value::array(x.read().unwrap().iter().chain(y.read().unwrap().iter()).cloned().collect()),
            _ => num_pair(a, b).map(|(x, y)| Float(x + y)).ok_or_else(bad)?,
        },
        BOp::Sub => match (a, b) {
            (Int(x), Int(y)) => Int(x.wrapping_sub(*y)),
            (Vec2(a1, a2), Vec2(b1, b2)) => Vec2(a1 - b1, a2 - b2),
            _ => num_pair(a, b).map(|(x, y)| Float(x - y)).ok_or_else(bad)?,
        },
        BOp::Mul => match (a, b) {
            (Int(x), Int(y)) => Int(x.wrapping_mul(*y)),
            (Vec2(x, y), Vec2(p, q)) => Vec2(x * p, y * q),
            (Vec2(x, y), n) | (n, Vec2(x, y)) => {
                let k = n.as_f64().ok_or_else(bad)?;
                Vec2(x * k, y * k)
            }
            _ => num_pair(a, b).map(|(x, y)| Float(x * y)).ok_or_else(bad)?,
        },
        BOp::Div => match (a, b) {
            (Int(_), Int(0)) => return Err(Error::runtime("division by zero")),
            (Int(x), Int(y)) => Int(x / y),
            (Vec2(x, y), n) => {
                let k = n.as_f64().ok_or_else(bad)?;
                Vec2(x / k, y / k)
            }
            _ => num_pair(a, b).map(|(x, y)| Float(x / y)).ok_or_else(bad)?,
        },
        BOp::Mod => match (a, b) {
            (Int(_), Int(0)) => return Err(Error::runtime("modulo by zero")),
            (Int(x), Int(y)) => Int(x % y),
            (Str(fmt), _) => Value::str(&format_string(fmt, b)),
            _ => num_pair(a, b).map(|(x, y)| Float(x % y)).ok_or_else(bad)?,
        },
        BOp::Pow => match (a, b) {
            (Int(x), Int(y)) if *y >= 0 => Int(x.wrapping_pow(*y as u32)),
            _ => num_pair(a, b).map(|(x, y)| Float(x.powf(y))).ok_or_else(bad)?,
        },
        BOp::BitAnd | BOp::BitOr | BOp::BitXor | BOp::Shl | BOp::Shr => {
            let (x, y) = (a.as_i64().ok_or_else(bad)?, b.as_i64().ok_or_else(bad)?);
            Int(match op {
                BOp::BitAnd => x & y,
                BOp::BitOr => x | y,
                BOp::BitXor => x ^ y,
                BOp::Shl => x.wrapping_shl(y as u32),
                _ => x.wrapping_shr(y as u32),
            })
        }
        BOp::And | BOp::Or => unreachable!("short-circuited above"),
    })
}

/// `"%d and %s" % [1, "x"]`.
fn format_string(fmt: &str, arg: &Value) -> String {
    let items: Vec<Value> = match arg {
        Value::Array(a) => a.read().unwrap().clone(),
        other => vec![other.clone()],
    };
    let mut out = String::new();
    let mut it = items.into_iter();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        let mut spec = String::new();
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '-' || chars[i] == '+') {
            spec.push(chars[i]);
            i += 1;
        }
        let Some(&kind) = chars.get(i) else { break };
        i += 1;
        if kind == '%' {
            out.push('%');
            continue;
        }
        let v = it.next().unwrap_or(Value::Nil);
        match kind {
            'd' => out.push_str(&v.as_i64().unwrap_or(0).to_string()),
            'f' => {
                let prec = spec.split('.').nth(1).and_then(|p| p.parse().ok()).unwrap_or(6);
                out.push_str(&format!("{:.*}", prec, v.as_f64().unwrap_or(0.0)));
            }
            _ => out.push_str(&v.to_text()),
        }
    }
    out
}

/// Free functions that need no host.
fn builtin(name: &str, args: &[Value]) -> R<Option<Value>> {
    let n = |i: usize| args.get(i).and_then(Value::as_f64).ok_or_else(|| Error::runtime(format!("{name}: argument {i} must be a number")));
    let int_like = args.iter().all(|a| matches!(a, Value::Int(_)));
    Ok(Some(match name {
        "str" => Value::str(&args.iter().map(Value::to_text).collect::<String>()),
        "int" => match args.first() {
            Some(Value::Str(s)) => Value::Int(s.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0)),
            Some(v) => Value::Int(v.as_i64().unwrap_or(0)),
            None => Value::Int(0),
        },
        "float" => match args.first() {
            Some(Value::Str(s)) => Value::Float(s.trim().parse().unwrap_or(0.0)),
            Some(v) => Value::Float(v.as_f64().unwrap_or(0.0)),
            None => Value::Float(0.0),
        },
        "bool" => Value::Bool(args.first().is_some_and(Value::truthy)),
        "len" => match args.first() {
            Some(Value::Array(a)) => Value::Int(a.read().unwrap().len() as i64),
            Some(Value::Dict(d)) => Value::Int(d.read().unwrap().len() as i64),
            Some(Value::Str(s)) => Value::Int(s.chars().count() as i64),
            _ => Value::Int(0),
        },
        "range" => {
            let (a, b, s) = match args.len() {
                1 => (0, n(0)? as i64, 1),
                2 => (n(0)? as i64, n(1)? as i64, 1),
                _ => (n(0)? as i64, n(1)? as i64, n(2)? as i64),
            };
            let mut v = Vec::new();
            let mut x = a;
            while s != 0 && ((s > 0 && x < b) || (s < 0 && x > b)) && v.len() < 100_000 {
                v.push(Value::Int(x));
                x += s;
            }
            Value::array(v)
        }
        "abs" => match args.first() {
            Some(Value::Int(i)) => Value::Int(i.abs()),
            _ => Value::Float(n(0)?.abs()),
        },
        "sign" => match args.first() {
            Some(Value::Int(i)) => Value::Int(i.signum()),
            _ => Value::Float(if n(0)? > 0.0 { 1.0 } else if n(0)? < 0.0 { -1.0 } else { 0.0 }),
        },
        "min" | "max" => {
            let pick_max = name == "max";
            if int_like {
                let (a, b) = (args[0].as_i64().unwrap_or(0), args[1].as_i64().unwrap_or(0));
                Value::Int(if pick_max { a.max(b) } else { a.min(b) })
            } else {
                Value::Float(if pick_max { n(0)?.max(n(1)?) } else { n(0)?.min(n(1)?) })
            }
        }
        "clamp" => {
            if int_like {
                Value::Int(args[0].as_i64().unwrap_or(0).clamp(args[1].as_i64().unwrap_or(0), args[2].as_i64().unwrap_or(0)))
            } else {
                Value::Float(n(0)?.max(n(1)?).min(n(2)?))
            }
        }
        "floor" => Value::Float(n(0)?.floor()),
        "ceil" => Value::Float(n(0)?.ceil()),
        "round" => Value::Float(n(0)?.round()),
        "sqrt" => Value::Float(n(0)?.sqrt()),
        "pow" => Value::Float(n(0)?.powf(n(1)?)),
        "lerp" => Value::Float(n(0)? + (n(1)? - n(0)?) * n(2)?),
        "deg2rad" => Value::Float(n(0)?.to_radians()),
        "rad2deg" => Value::Float(n(0)?.to_degrees()),
        "sin" => Value::Float(n(0)?.sin()),
        "cos" => Value::Float(n(0)?.cos()),
        "atan2" => Value::Float(n(0)?.atan2(n(1)?)),
        "stepify" => {
            let s = n(1)?;
            Value::Float(if s == 0.0 { n(0)? } else { (n(0)? / s).round() * s })
        }
        "Vector2" => Value::Vec2(args.first().and_then(Value::as_f64).unwrap_or(0.0), args.get(1).and_then(Value::as_f64).unwrap_or(0.0)),
        // colors and 3D vectors only appear in visuals; they are plain lists of numbers here
        "Color" => match args.first() {
            // Color("ff9444") / Color(r, g, b[, a])
            Some(Value::Str(hex)) => {
                let h = hex.trim_start_matches('#');
                let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f64 / 255.0;
                Value::Color(c(0), c(2), c(4), if h.len() >= 8 { c(6) } else { 1.0 })
            }
            _ => {
                let n = |i: usize, d: f64| args.get(i).and_then(Value::as_f64).unwrap_or(d);
                Value::Color(n(0, 1.0), n(1, 1.0), n(2, 1.0), n(3, 1.0))
            }
        },
        "Vector3" | "Rect2" => Value::array(args.iter().filter_map(Value::as_f64).map(Value::Float).collect()),
        "Array" => Value::array(vec![]),
        "Dictionary" => Value::dict(vec![]),
        "typeof" => Value::str(args.first().map(Value::type_name).unwrap_or("null")),
        // djb2 over the text form (the original hashes strings this way)
        "hash" => Value::Int(args.first().map(|v| v.to_text()).unwrap_or_default().bytes().fold(5381u32, |h, c| h.wrapping_mul(33).wrapping_add(c as u32)) as i64),
        "is_instance_valid" => Value::Bool(!matches!(args.first(), Some(Value::Nil) | None)),
        "push_error" | "push_warning" | "print" | "printerr" | "prints" | "print_debug" => Value::Nil,
        _ => return Ok(None),
    }))
}

fn method_of_value(o: &Value, name: &str, args: &[Value]) -> R<Value> {
    let unsupported = || Error::Unsupported(format!("{}.{name}", o.type_name()));
    match o {
        Value::Str(s) => Ok(match name {
            "empty" => Value::Bool(s.is_empty()),
            "length" => Value::Int(s.chars().count() as i64),
            "begins_with" => Value::Bool(matches!(args.first(), Some(Value::Str(p)) if s.starts_with(&**p))),
            "ends_with" => Value::Bool(matches!(args.first(), Some(Value::Str(p)) if s.ends_with(&**p))),
            "find" => Value::Int(match args.first() {
                Some(Value::Str(p)) => s.find(&**p).map(|i| i as i64).unwrap_or(-1),
                _ => -1,
            }),
            "rfind" => Value::Int(match args.first() {
                Some(Value::Str(p)) => s.rfind(&**p).map(|i| s[..i].chars().count() as i64).unwrap_or(-1),
                _ => -1,
            }),
            // the first / last n characters
            "left" | "right" => {
                let chars: Vec<char> = s.chars().collect();
                let n = args.first().and_then(Value::as_i64).unwrap_or(0).clamp(0, chars.len() as i64) as usize;
                // Godot 3's `right(i)` is the text from position i on
                Value::str(&if name == "left" { chars[..n].iter().collect::<String>() } else { chars[n..].iter().collect::<String>() })
            }
            "to_lower" => Value::str(&s.to_lowercase()),
            "to_upper" => Value::str(&s.to_uppercase()),
            "to_int" => Value::Int(s.trim().parse::<i64>().unwrap_or(0)),
            "to_float" => Value::Float(s.trim().parse::<f64>().unwrap_or(0.0)),
            "is_valid_integer" => Value::Bool(s.trim().parse::<i64>().is_ok()),
            "substr" => {
                let from = args.first().and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
                let len = args.get(1).and_then(Value::as_i64).map(|l| l.max(0) as usize).unwrap_or(usize::MAX);
                Value::str(&s.chars().skip(from).take(len).collect::<String>())
            }
            "split" => match args.first() {
                Some(Value::Str(d)) => Value::array(s.split(&**d).map(Value::str).collect()),
                _ => return Err(unsupported()),
            },
            _ => return Err(unsupported()),
        }),
        Value::Array(a) => Ok(match name {
            "size" => Value::Int(a.read().unwrap().len() as i64),
            "empty" => Value::Bool(a.read().unwrap().is_empty()),
            "append" | "push_back" => {
                a.write().unwrap().push(args.first().cloned().unwrap_or(Value::Nil));
                Value::Nil
            }
            "push_front" => {
                a.write().unwrap().insert(0, args.first().cloned().unwrap_or(Value::Nil));
                Value::Nil
            }
            "pop_back" => a.write().unwrap().pop().unwrap_or(Value::Nil),
            "pop_front" => {
                let mut a = a.write().unwrap();
                if a.is_empty() { Value::Nil } else { a.remove(0) }
            }
            "front" => a.read().unwrap().first().cloned().unwrap_or(Value::Nil),
            "back" => a.read().unwrap().last().cloned().unwrap_or(Value::Nil),
            "has" => Value::Bool(args.first().is_some_and(|x| a.read().unwrap().iter().any(|y| equal(x, y)))),
            "find" => Value::Int(args.first().and_then(|x| a.read().unwrap().iter().position(|y| equal(x, y))).map(|i| i as i64).unwrap_or(-1)),
            "erase" => {
                if let Some(x) = args.first() {
                    let mut a = a.write().unwrap();
                    if let Some(p) = a.iter().position(|y| equal(x, y)) {
                        a.remove(p);
                    }
                }
                Value::Nil
            }
            "clear" => {
                a.write().unwrap().clear();
                Value::Nil
            }
            "duplicate" => Value::array(a.read().unwrap().clone()),
            "append_array" => {
                if let Some(Value::Array(b)) = args.first() {
                    let items = b.read().unwrap().clone();
                    a.write().unwrap().extend(items);
                }
                Value::Nil
            }
            _ => return Err(unsupported()),
        }),
        Value::Dict(d) => Ok(match name {
            "size" => Value::Int(d.read().unwrap().len() as i64),
            "empty" => Value::Bool(d.read().unwrap().is_empty()),
            "has" => Value::Bool(args.first().is_some_and(|k| d.read().unwrap().iter().any(|(k2, _)| equal(k, k2)))),
            "keys" => Value::array(d.read().unwrap().iter().map(|(k, _)| k.clone()).collect()),
            "values" => Value::array(d.read().unwrap().iter().map(|(_, v)| v.clone()).collect()),
            "get" => args.first().and_then(|k| dict_get(d, k)).or_else(|| args.get(1).cloned()).unwrap_or(Value::Nil),
            "erase" => {
                if let Some(k) = args.first() {
                    d.write().unwrap().retain(|(k2, _)| !equal(k, k2));
                }
                Value::Nil
            }
            "clear" => {
                d.write().unwrap().clear();
                Value::Nil
            }
            "duplicate" => Value::dict(d.read().unwrap().clone()),
            "merge" => {
                if let Some(Value::Dict(o)) = args.first() {
                    for (k, v) in o.read().unwrap().clone() {
                        dict_set(d, k, v);
                    }
                }
                Value::Nil
            }
            _ => return Err(unsupported()),
        }),
        Value::Vec2(x, y) => Ok(match name {
            "length" => Value::Float(x.hypot(*y)),
            "length_squared" => Value::Float(x * x + y * y),
            "normalized" => {
                let l = x.hypot(*y);
                if l == 0.0 { Value::Vec2(0.0, 0.0) } else { Value::Vec2(x / l, y / l) }
            }
            "dot" => match args.first() {
                Some(Value::Vec2(a, b)) => Value::Float(x * a + y * b),
                _ => return Err(unsupported()),
            },
            "angle" => Value::Float(y.atan2(*x)),
            "distance_to" => match args.first() {
                Some(Value::Vec2(a, b)) => Value::Float((x - a).hypot(y - b)),
                _ => return Err(unsupported()),
            },
            "abs" => Value::Vec2(x.abs(), y.abs()),
            "rotated" => {
                let a = args.first().and_then(Value::as_f64).unwrap_or(0.0);
                Value::Vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos())
            }
            _ => return Err(unsupported()),
        }),
        _ => Err(unsupported()),
    }
}
