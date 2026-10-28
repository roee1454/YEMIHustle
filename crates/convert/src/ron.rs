//! A small RON writer producing exactly the text the Python converter wrote (formatting is part
//! of the contract: files are compared byte for byte in the parity tests).

use serde_json::Value;

/// A value to write: a number, text, tuple, list, or a named struct / unit variant.
#[derive(Clone, Debug, PartialEq)]
pub enum R {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Tuple(Vec<R>),
    List(Vec<R>),
    /// `Name(field: value, ...)`; without fields just `Name`.
    S(String, Vec<(String, R)>),
    /// `Some(value)` / `None` (only the effects file uses these).
    Some(Box<R>),
    None,
    /// A map written as `{ "key": value, ... }`.
    Map(Vec<(String, R)>),
    /// A tuple variant: `Name(a, b)`.
    Call(String, Vec<R>),
}

impl R {
    pub fn s(name: &str) -> R {
        R::S(name.to_string(), vec![])
    }

    pub fn st(name: &str, fields: Vec<(&str, R)>) -> R {
        R::S(name.to_string(), fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn str(s: impl Into<String>) -> R {
        R::Str(s.into())
    }

    pub fn strs<I: IntoIterator<Item = S2>, S2: Into<String>>(v: I) -> R {
        R::List(v.into_iter().map(|s| R::Str(s.into())).collect())
    }

    pub fn tup2(a: R, b: R) -> R {
        R::Tuple(vec![a, b])
    }

    /// Sets a struct field (replacing in place if present, else appending, like a Python dict).
    pub fn put(&mut self, key: &str, v: R) {
        if let R::S(_, f) | R::Map(f) = self {
            match f.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => f.push((key.to_string(), v)),
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&R> {
        match self {
            R::S(_, f) | R::Map(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let R::S(_, f) | R::Map(f) = self {
            f.retain(|(k, _)| k != key);
        }
    }

    pub fn name(&self) -> &str {
        match self {
            R::S(n, _) => n,
            _ => "",
        }
    }

    /// From a JSON number/bool/string, keeping integers as integers.
    pub fn json(v: &Value) -> R {
        match v {
            Value::Bool(b) => R::Bool(*b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => R::Int(i),
                None => R::Float(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => R::Str(s.clone()),
            _ => R::Int(0),
        }
    }
}

/// Python's `repr(float)` for the values this writer produces (already rounded to 6 places).
fn py_repr(x: f64) -> String {
    let a = x.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        // scientific form: mantissa e[+-]dd
        let s = format!("{x:e}");
        let (m, e) = s.split_once('e').expect("exponent");
        let exp: i32 = e.parse().unwrap_or(0);
        return format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Python's `round(x, n)`: the double nearest to the correctly rounded decimal.
pub fn round_to(x: f64, n: usize) -> f64 {
    format!("{x:.n$}").parse().unwrap_or(x)
}

pub fn fnum(x: f64) -> String {
    let x = round_to(x, 6);
    if x == 0.0 {
        return "0.0".to_string();
    }
    if x == x.trunc() && x.abs() < 1e15 {
        return format!("{}.0", x as i64);
    }
    py_repr(x)
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn is_scalar(v: &R) -> bool {
    matches!(v, R::Str(_) | R::Int(_) | R::Float(_) | R::Bool(_))
}

pub fn emit(v: &R, ind: usize) -> String {
    let pad = "    ".repeat(ind);
    match v {
        R::Bool(b) => b.to_string(),
        R::Int(i) => i.to_string(),
        R::Float(f) => fnum(*f),
        R::Str(s) => quote(s),
        R::Tuple(t) => format!("({})", t.iter().map(|x| emit(x, ind)).collect::<Vec<_>>().join(", ")),
        R::List(l) => {
            if l.is_empty() {
                return "[]".into();
            }
            if l.iter().all(is_scalar) && l.iter().map(|x| emit(x, 0).chars().count() + 2).sum::<usize>() < 70 {
                return format!("[{}]", l.iter().map(|x| emit(x, 0)).collect::<Vec<_>>().join(", "));
            }
            let body: String = l.iter().map(|x| format!("{pad}    {},\n", emit(x, ind + 1))).collect();
            format!("[\n{body}{pad}]")
        }
        R::S(name, fields) => {
            if fields.is_empty() {
                return name.clone();
            }
            let flat = fields.iter().map(|(k, x)| format!("{k}: {}", emit(x, ind))).collect::<Vec<_>>().join(", ");
            if flat.chars().count() < 78 && !flat.contains('\n') {
                return format!("{name}({flat})");
            }
            let body: String = fields.iter().map(|(k, x)| format!("{pad}    {k}: {},\n", emit(x, ind + 1))).collect();
            format!("{name}(\n{body}{pad})")
        }
        // not used by the character file
        R::Some(x) => format!("Some({})", emit(x, ind)),
        R::None => "None".into(),
        R::Map(m) => emit_map(m, ind),
        R::Call(name, args) => format!("{name}({})", args.iter().map(|a| emit(a, ind)).collect::<Vec<_>>().join(", ")),
    }
}

fn emit_map(m: &[(String, R)], ind: usize) -> String {
    let pad = "    ".repeat(ind);
    if m.is_empty() {
        return "{}".into();
    }
    let body: String = m.iter().map(|(k, x)| format!("{pad}    {}: {},\n", quote(k), emit(x, ind + 1))).collect();
    format!("{{\n{body}{pad}}}")
}

/// The effects file's flavour: structs are always spread over lines, options are written out.
pub fn emit_fx(v: &R, ind: usize) -> String {
    let pad = "    ".repeat(ind);
    match v {
        R::None => "None".into(),
        R::Some(x) => format!("Some({})", emit_fx(x, ind)),
        R::S(name, fields) => {
            let body: String = fields.iter().map(|(k, x)| format!("{pad}    {k}: {},\n", emit_fx(x, ind + 1))).collect();
            format!("{name}(\n{body}{pad})")
        }
        R::Map(m) => {
            if m.is_empty() {
                return "{}".into();
            }
            let body: String = m.iter().map(|(k, x)| format!("{pad}    {}: {},\n", quote(k), emit_fx(x, ind + 1))).collect();
            format!("{{\n{body}{pad}}}")
        }
        other => emit(other, ind),
    }
}

/// `snake_case` as the converter names things: split at lower-to-upper, lowercase, and replace
/// anything else with an underscore.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            out.push('_');
        }
        out.push(c);
        prev = Some(c);
    }
    out.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '_' }).collect()
}

pub fn sid(name: &str) -> String {
    if name == "Wait" {
        "idle".into()
    } else {
        snake(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_python() {
        assert_eq!(fnum(1.0), "1.0");
        assert_eq!(fnum(-0.0), "0.0");
        assert_eq!(fnum(4e-8), "0.0");
        assert_eq!(fnum(0.5), "0.5");
        assert_eq!(fnum(1.23456789), "1.234568");
        assert_eq!(fnum(0.00001), "1e-05");
        assert_eq!(fnum(2.5), "2.5");
        assert_eq!(fnum(-12.0), "-12.0");
    }

    #[test]
    fn names_and_layout() {
        assert_eq!(snake("GroundedPunch"), "grounded_punch");
        assert_eq!(snake("3Combo"), "3_combo");
        assert_eq!(snake("Air Dash!"), "air_dash_");
        assert_eq!(sid("Wait"), "idle");
        let s = R::st("A", vec![("x", R::Int(1)), ("l", R::strs(["a", "b"]))]);
        assert_eq!(emit(&s, 0), "A(x: 1, l: [\"a\", \"b\"])");
    }
}
