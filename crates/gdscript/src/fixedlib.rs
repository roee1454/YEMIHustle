//! The original's `fixed` library: arithmetic on decimal strings in 16.16 fixed point.
//! Multiplication floors and division truncates toward zero (measured on the original's native
//! library, see `crates/sim/src/fixed.rs`, which this agrees with).

use crate::value::Value;
use crate::Error;

const ONE: i64 = 1 << 16;

/// A number in any form the library accepts (decimal text, int, float) as a raw 16.16 value.
pub fn raw(v: &Value) -> Result<i64, Error> {
    match v {
        Value::Int(i) => Ok(i << 16),
        Value::Bool(b) => Ok((*b as i64) << 16),
        Value::Float(f) => Ok((f * ONE as f64).round() as i64),
        Value::Str(s) => {
            let s = s.trim();
            let (neg, body) = match s.strip_prefix('-') {
                Some(r) => (true, r.trim_start()),
                None => (false, s.strip_prefix('+').unwrap_or(s)),
            };
            let (int, frac) = body.split_once('.').unwrap_or((body, ""));
            let int: i64 = if int.is_empty() { 0 } else { int.parse().map_err(|_| Error::runtime(format!("not a number: {s:?}")))? };
            let digits: String = frac.chars().take(10).collect();
            let frac_val: f64 = if digits.is_empty() { 0.0 } else { format!("0.{digits}").parse().map_err(|_| Error::runtime(format!("not a number: {s:?}")))? };
            let raw = (int << 16) + (frac_val * ONE as f64).round() as i64;
            Ok(if neg { -raw } else { raw })
        }
        other => Err(Error::runtime(format!("fixed: expected a number, got {}", other.type_name()))),
    }
}

/// The library's text form: whole numbers plain, otherwise up to five decimals.
pub fn text(raw: i64) -> Value {
    if raw % ONE == 0 {
        return Value::str(&(raw / ONE).to_string());
    }
    let neg = raw < 0;
    let a = raw.unsigned_abs() as f64 / ONE as f64;
    let mut s = format!("{a:.5}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    Value::str(&if neg { format!("-{s}") } else { s })
}

fn mul(a: i64, b: i64) -> i64 {
    (a * b).div_euclid(ONE)
}

fn div(a: i64, b: i64) -> Result<i64, Error> {
    if b == 0 {
        return Err(Error::runtime("fixed: division by zero"));
    }
    Ok((a * ONE) / b)
}

fn sqrt(a: i64) -> i64 {
    if a <= 0 {
        return 0;
    }
    // integer square root of a * 2^16
    let n = (a as u128) << 16;
    let mut x = (n as f64).sqrt() as u128;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x as i64
}

fn vec_len(x: i64, y: i64) -> i64 {
    sqrt(mul(x, x) + mul(y, y))
}

fn vec_val(x: i64, y: i64) -> Value {
    Value::dict(vec![(Value::str("x"), text(x)), (Value::str("y"), text(y))])
}

fn num(args: &[Value], i: usize) -> Result<i64, Error> {
    raw(args.get(i).ok_or_else(|| Error::runtime("fixed: missing argument"))?)
}

fn round_to_int(raw: i64) -> i64 {
    if raw >= 0 {
        (raw + ONE / 2) >> 16
    } else {
        -((-raw + ONE / 2) >> 16)
    }
}

/// `fixed.<name>(args)`.
pub fn call(name: &str, args: &[Value]) -> Result<Value, Error> {
    let b = |v: bool| Ok(Value::Bool(v));
    match name {
        "add" => Ok(text(num(args, 0)? + num(args, 1)?)),
        "sub" => Ok(text(num(args, 0)? - num(args, 1)?)),
        "mul" => Ok(text(mul(num(args, 0)?, num(args, 1)?))),
        "div" => Ok(text(div(num(args, 0)?, num(args, 1)?)?)),
        "abs" => Ok(text(num(args, 0)?.abs())),
        "neg" => Ok(text(-num(args, 0)?)),
        "sign" => Ok(Value::Int(num(args, 0)?.signum())),
        "eq" => b(num(args, 0)? == num(args, 1)?),
        "gt" => b(num(args, 0)? > num(args, 1)?),
        "lt" => b(num(args, 0)? < num(args, 1)?),
        "ge" | "gte" => b(num(args, 0)? >= num(args, 1)?),
        "le" | "lte" => b(num(args, 0)? <= num(args, 1)?),
        "round" => Ok(Value::Int(round_to_int(num(args, 0)?))),
        "floor" => Ok(Value::Int(num(args, 0)?.div_euclid(ONE))),
        "ceil" => Ok(Value::Int(-((-num(args, 0)?).div_euclid(ONE)))),
        "sqrt" => Ok(text(sqrt(num(args, 0)?))),
        "powu" => {
            let base = num(args, 0)?;
            let n = args.get(1).and_then(Value::as_i64).unwrap_or(0).max(0);
            let mut r = ONE;
            for _ in 0..n {
                r = mul(r, base);
            }
            Ok(text(r))
        }
        "lerp_string" => {
            let (a, c, t) = (num(args, 0)?, num(args, 1)?, num(args, 2)?);
            Ok(text(a + mul(c - a, t)))
        }
        "vec_add" => Ok(vec_val(num(args, 0)? + num(args, 2)?, num(args, 1)? + num(args, 3)?)),
        "vec_sub" => Ok(vec_val(num(args, 0)? - num(args, 2)?, num(args, 1)? - num(args, 3)?)),
        "vec_mul" => {
            let s = num(args, 2)?;
            Ok(vec_val(mul(num(args, 0)?, s), mul(num(args, 1)?, s)))
        }
        "vec_div" => {
            let s = num(args, 2)?;
            Ok(vec_val(div(num(args, 0)?, s)?, div(num(args, 1)?, s)?))
        }
        "vec_len" => Ok(text(vec_len(num(args, 0)?, num(args, 1)?))),
        // radians; through floats (the native library's own rounding is not known)
        "vec_to_angle" => Ok(text(((num(args, 1)? as f64).atan2(num(args, 0)? as f64) * ONE as f64).round() as i64)),
        "vec_dist" => Ok(text(vec_len(num(args, 0)? - num(args, 2)?, num(args, 1)? - num(args, 3)?))),
        "normalized_vec" | "normalized_vec_times" => {
            let (x, y) = (num(args, 0)?, num(args, 1)?);
            let l = vec_len(x, y);
            if l == 0 {
                return Ok(vec_val(0, 0));
            }
            let (nx, ny) = (div(x, l)?, div(y, l)?);
            if name == "normalized_vec" {
                return Ok(vec_val(nx, ny));
            }
            let s = num(args, 2)?;
            Ok(vec_val(mul(nx, s), mul(ny, s)))
        }
        _ => Err(Error::Unsupported(format!("fixed.{name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::str(v)
    }

    #[test]
    fn arithmetic_matches_the_originals_text_form() {
        assert_eq!(call("mul", &[s("0.5"), s("3")]).unwrap().to_text(), "1.5");
        assert_eq!(call("mul", &[s("19"), s("1")]).unwrap().to_text(), "19");
        assert_eq!(call("sub", &[s("1.0"), s("0.25")]).unwrap().to_text(), "0.75");
        assert_eq!(call("round", &[s("2.5")]).unwrap().as_i64(), Some(3));
        assert_eq!(call("round", &[s("-2.5")]).unwrap().as_i64(), Some(-3));
        assert!(call("gt", &[s("1"), s("0.99")]).unwrap().truthy());
    }

    #[test]
    fn vectors() {
        let v = call("normalized_vec_times", &[s("3"), s("4"), s("10")]).unwrap();
        let Value::Dict(d) = v else { panic!() };
        let d = d.read().unwrap();
        // fixed-point rounding may leave the last digit one off
        let near = |v: &Value, want: f64| (v.as_f64().or_else(|| v.to_text().parse().ok()).unwrap() - want).abs() < 0.001;
        assert!(near(&d[0].1, 6.0), "{}", d[0].1.to_text());
        assert!(near(&d[1].1, 8.0), "{}", d[1].1.to_text());
        assert_eq!(call("vec_len", &[s("3"), s("4")]).unwrap().to_text(), "5");
    }
}
