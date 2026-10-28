//! What the converter needs to know about the decompiled GDScript files: their `extends`
//! chain, `export var` defaults, constants and which functions exist. Ported from
//! `tools/extract_character.py` (and the small helpers in `convert_character.py`).

use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::tscn::convert;

pub const ENUMS: [(&str, &[&str]); 5] = [
    ("ActionType", &["Movement", "Attack", "Special", "Super", "Defense", "Hurt"]),
    ("AirType", &["Grounded", "Aerial", "Both"]),
    ("BusyInterrupt", &["Normal", "Hurt", "None"]),
    ("HitboxType", &["Normal", "Flip", "ThrowHit", "OffensiveBurst", "Burst", "NoHitstun", "Detect"]),
    ("HitHeight", &["High", "Mid", "Low"]),
];

pub const PROP_ENUM: [(&str, &str); 5] = [
    ("type", "ActionType"),
    ("air_type", "AirType"),
    ("busy_interrupt_type", "BusyInterrupt"),
    ("hitbox_type", "HitboxType"),
    ("hit_height", "HitHeight"),
];

pub fn enum_members(name: &str) -> &'static [&'static str] {
    ENUMS.iter().find(|(n, _)| *n == name).map(|(_, m)| *m).unwrap_or(&[])
}

#[derive(Clone, Debug, Default)]
pub struct ScriptInfo {
    pub extends: Option<String>,
    pub exports: Map<String, Value>,
    pub consts: Map<String, Value>,
    pub funcs: Vec<String>,
    pub frame_hooks: Vec<i64>,
    pub sets_anim_length: bool,
    pub sets_iasa: bool,
    pub sets_hitbox_fields: bool,
    pub applies_forces: bool,
}

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        CELL.get_or_init(|| Regex::new($pat).expect("valid pattern"))
    }};
}

/// True if `pat` matches and the match is not followed by `=` (Python's `(?!=)`; the regex
/// crate has no lookahead). `pat` must end in `=`.
fn assigns(text: &str, re: &Regex) -> bool {
    re.find_iter(text).any(|m| text[m.end()..].chars().next() != Some('='))
}

pub fn parse(text: &str) -> ScriptInfo {
    let mut info = ScriptInfo::default();
    if let Some(c) = lazy_re!(r#"(?m)^extends\s+("[^"]+"|[\w.]+)"#).captures(text) {
        info.extends = Some(c[1].trim_matches('"').to_string());
    }
    let export = lazy_re!(r"(?m)^export\s*(?:\(([^)]*)\))?\s*var\s+(\w+)\s*(?::\s*\w+)?\s*(?:=\s*(.+))?$");
    for c in export.captures_iter(text) {
        let var = c[2].to_string();
        if var.starts_with("_c_") {
            continue;
        }
        let Some(default) = c.get(3) else {
            info.exports.insert(var, Value::Null);
            continue;
        };
        let default = default.as_str().trim();
        let value = match lazy_re!(r"^(\w+)\.(\w+)$").captures(default) {
            Some(em) if enum_members(&em[1]).contains(&&em[2]) => Value::String(em[2].to_string()),
            _ => convert(default, None),
        };
        info.exports.insert(var, value);
    }
    for c in lazy_re!(r"(?m)^const\s+(\w+)\s*=\s*(.+)$").captures_iter(text) {
        info.consts.insert(c[1].to_string(), convert(&c[2], None));
    }
    let frame = lazy_re!(r"^_frame_(\d+)(_shared)?$");
    for c in lazy_re!(r"(?m)^func\s+(\w+)\(").captures_iter(text) {
        let name = c[1].to_string();
        if let Some(f) = frame.captures(&name) {
            info.frame_hooks.push(f[1].parse().unwrap_or(0));
        }
        info.funcs.push(name);
    }
    info.frame_hooks.sort_unstable();
    info.frame_hooks.dedup();
    info.sets_anim_length = assigns(text, lazy_re!(r"\banim_length\s*="));
    info.sets_iasa = assigns(text, lazy_re!(r"\biasa_at\s*="));
    info.sets_hitbox_fields = assigns(text, lazy_re!(r"\.(start_tick|active_ticks|damage|knockback|hitstun_ticks|dir_x|dir_y)\s*="));
    info.applies_forces = lazy_re!(r"\bapply_force\w*\(|\bset_vel\(|\bmove_directly\w*\(").is_match(text);
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exports_and_assignments() {
        let src = "extends \"res://characters/states/CharState.gd\"\nexport var damage = 35\nexport (int, \"A\", \"B\") var kind = HitboxType.Burst\nexport var nothing\nconst X = \"1.5\"\nfunc _frame_4_shared():\n\tanim_length = 9\n\tif iasa_at == 3:\n\t\tpass\n";
        let i = parse(src);
        assert_eq!(i.extends.as_deref(), Some("res://characters/states/CharState.gd"));
        assert_eq!(i.exports["damage"], 35);
        assert_eq!(i.exports["kind"], "Burst");
        assert!(i.exports["nothing"].is_null());
        assert_eq!(i.consts["X"], "1.5");
        assert_eq!(i.frame_hooks, vec![4]);
        assert!(i.sets_anim_length, "an assignment");
        assert!(!i.sets_iasa, "a comparison is not an assignment");
    }
}
