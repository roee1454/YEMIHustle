//! Godot 3 text scenes (`.tscn`) as the converter needs them: nodes with their properties as
//! JSON-friendly values. Ported from `tools/extract_character.py`; the decompiler prints a few
//! things oddly (`- 1`), which `convert` undoes.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Map, Value};

/// External resources of a scene: id -> (path without `res://`, type).
pub type Ext = HashMap<u32, (String, String)>;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub ty: Option<String>,
    pub parent: Option<String>,
    pub index: Option<String>,
    pub instance: Option<String>,
    pub props: Map<String, Value>,
    /// The external resources this node's `ExtResource(n)` values refer to.
    pub ext: Rc<Ext>,
    /// Set when a script was assigned in an inheriting scene: where its `res_id` is looked up.
    pub script_ext: Option<Rc<Ext>>,
    pub origin: &'static str,
    pub overrides: Vec<String>,
}

pub struct Scene {
    pub ext: Rc<Ext>,
    pub nodes: Vec<Node>,
    /// Sub-resources by id: their type and raw text (for sprite frames).
    pub subs: HashMap<u32, (String, String)>,
}

/// One animation of a `SpriteFrames` resource: name, frame count, frames per second, looping.
#[derive(Clone, Debug, PartialEq)]
pub struct Anim {
    pub name: String,
    pub frames: u32,
    pub fps: f32,
    pub looped: bool,
}

/// The animations listed in a `SpriteFrames` resource's text.
pub fn sprite_anims(text: &str) -> Vec<Anim> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    let re = re(&CELL, r#"(?s)"frames":\s*\[(.*?)\],\s*"loop":\s*(true|false),\s*"name":\s*"((?:[^"\\]|\\.)*)",\s*"speed":\s*([0-9.]+)"#);
    re.captures_iter(text)
        .map(|c| Anim {
            name: c[3].to_string(),
            frames: c[1].matches("Resource(").count() as u32,
            fps: c[4].parse().unwrap_or(5.0),
            looped: &c[2] == "true",
        })
        .collect()
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("valid pattern"))
}

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        re(&CELL, $pat)
    }};
}

/// A Godot text value as a JSON value.
pub fn convert(raw: &str, ext: Option<&Ext>) -> Value {
    let mut s = raw.trim().to_string();
    // the decompiler prints "- 1" for -1
    if let Some(rest) = s.strip_prefix('-') {
        let t = rest.trim_start();
        if t.len() < rest.len() && t.starts_with(|c: char| c.is_ascii_digit()) {
            s = format!("-{t}");
        }
    }
    match s.as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    if lazy_re!(r"^-?\d+(\.\d+)?([eE][-+]?\d+)?$").is_match(&s) {
        if s.contains('.') || s.to_ascii_lowercase().contains('e') {
            return json!(s.parse::<f64>().unwrap_or(0.0));
        }
        return s.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(s.parse::<f64>().unwrap_or(0.0)));
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        return Value::String(inner.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\"));
    }
    if s.starts_with('[') && s.ends_with(']') {
        let inner = s[1..s.len() - 1].trim();
        if inner.is_empty() {
            return json!([]);
        }
        if lazy_re!(r"^[-\d.\s,]+$").is_match(inner) {
            return Value::Array(inner.split(',').filter(|p| !p.trim().is_empty()).map(|p| convert(p, None)).collect());
        }
    }
    if let Some(c) = lazy_re!(r"^ExtResource\(\s*(\d+)\s*\)$").captures(&s) {
        let id: u32 = c[1].parse().unwrap_or(0);
        return match ext.and_then(|e| e.get(&id)) {
            Some((path, _)) => json!({ "res": path }),
            None => json!({ "res_id": id }),
        };
    }
    if let Some(c) = lazy_re!(r"^SubResource\(\s*(\d+)\s*\)$").captures(&s) {
        return json!({ "sub_id": c[1].parse::<u32>().unwrap_or(0) });
    }
    if let Some(c) = lazy_re!(r"^(Vector2|Color|Rect2|Vector3)\(\s*(.*?)\s*\)$").captures(&s) {
        let parts: Vec<Value> = c[2].split(',').map(|p| convert(p, None)).collect();
        let mut m = Map::new();
        m.insert(c[1].to_string(), Value::Array(parts));
        return Value::Object(m);
    }
    json!({ "raw": s })
}

/// True if quotes and brackets are balanced (for values that span lines).
fn balanced(text: &str) -> bool {
    let (mut in_str, mut esc, mut depth) = (false, false, 0i32);
    for ch in text.chars() {
        if esc {
            esc = false;
            continue;
        }
        if ch == '\\' {
            esc = true;
            continue;
        }
        if ch == '"' {
            in_str = !in_str;
            continue;
        }
        if !in_str {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
    }
    !in_str && depth <= 0
}

fn header_attrs(header: &str) -> HashMap<String, String> {
    let r = lazy_re!(r#"(\w+)=("(?:[^"\\]|\\.)*"|ExtResource\(\s*\d+\s*\)|SubResource\(\s*\d+\s*\)|[^\s\]]+)"#);
    r.captures_iter(header).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').to_string()
}

/// Reads a text file the way Python's text mode does (universal newlines).
pub fn read_text(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n"))
}

pub fn parse(text: &str) -> Scene {
    let mut ext: Ext = HashMap::new();
    // nodes are built with a placeholder ext and given the final table at the end, because
    // `convert` needs the table as it grows (resources are declared before nodes)
    struct Raw {
        name: String,
        ty: Option<String>,
        parent: Option<String>,
        index: Option<String>,
        instance: Option<String>,
        props: Map<String, Value>,
    }
    let mut raws: Vec<Raw> = Vec::new();
    let mut subs: HashMap<u32, (String, String)> = HashMap::new();
    let mut sub_open: Option<u32> = None;
    enum Cur {
        None,
        Sub,
        Node,
    }
    let mut cur = Cur::None;
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.starts_with('[') {
            sub_open = None;
            let Some(hm) = lazy_re!(r"^\[(\w+)\s*(.*)\]\s*$").captures(line) else { continue };
            let attrs = header_attrs(&hm[2]);
            match &hm[1] {
                "ext_resource" => {
                    let id: u32 = attrs.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let path = attrs.get("path").map(|p| unquote(p).replace("res://", "")).unwrap_or_default();
                    let ty = attrs.get("type").map(|t| unquote(t)).unwrap_or_default();
                    ext.insert(id, (path, ty));
                    cur = Cur::None;
                }
                "sub_resource" => {
                    cur = Cur::Sub;
                    let id: u32 = attrs.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
                    subs.insert(id, (attrs.get("type").map(|t| unquote(t)).unwrap_or_default(), String::new()));
                    sub_open = Some(id);
                }
                "node" => {
                    let opt = |k: &str| attrs.get(k).map(|v| unquote(v)).filter(|v| !v.is_empty());
                    raws.push(Raw {
                        name: attrs.get("name").map(|n| unquote(n)).unwrap_or_default(),
                        ty: opt("type"),
                        parent: opt("parent"),
                        index: opt("index"),
                        instance: attrs.get("instance").cloned(),
                        props: Map::new(),
                    });
                    cur = Cur::Node;
                }
                _ => cur = Cur::None,
            }
            continue;
        }
        if matches!(cur, Cur::None) || line.trim().is_empty() || line.starts_with(';') {
            continue;
        }
        let Some(m) = lazy_re!(r"^([A-Za-z_][\w/:]*)\s*=\s*(.*)$").captures(line) else { continue };
        let (key, mut val) = (m[1].to_string(), m[2].to_string());
        while !balanced(&val) && i < lines.len() {
            val.push('\n');
            val.push_str(lines[i]);
            i += 1;
        }
        if let (Cur::Sub, Some(id)) = (&cur, sub_open) {
            if let Some(e) = subs.get_mut(&id) {
                e.1.push_str(&format!("{key} = {val}\n"));
            }
        }
        if matches!(cur, Cur::Node) {
            let n = raws.last_mut().expect("a node is open");
            n.props.insert(key, convert(&val, Some(&ext)));
        }
    }
    let ext = Rc::new(ext);
    let nodes = raws
        .into_iter()
        .map(|r| Node {
            name: r.name,
            ty: r.ty,
            parent: r.parent,
            index: r.index,
            instance: r.instance,
            props: r.props,
            ext: ext.clone(),
            script_ext: None,
            origin: "character",
            overrides: vec![],
        })
        .collect();
    Scene { ext, nodes, subs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_convert_like_the_python_tool() {
        assert_eq!(convert("- 1", None), json!(-1), "the decompiler's spaced minus");
        assert_eq!(convert("35", None), json!(35));
        assert_eq!(convert("1.5", None), json!(1.5));
        assert_eq!(convert("1e3", None), json!(1000.0));
        assert_eq!(convert("true", None), json!(true));
        assert_eq!(convert("\"a\\\"b\\nc\"", None), json!("a\"b\nc"));
        assert_eq!(convert("[ 1, 2.5 ]", None), json!([1, 2.5]));
        assert_eq!(convert("Vector2( 3, - 4 )", None), json!({"Vector2": [3, -4]}));
        assert_eq!(convert("ExtResource( 7 )", None), json!({"res_id": 7}));
        assert_eq!(convert("preload(\"x\")", None), json!({"raw": "preload(\"x\")"}));
    }

    #[test]
    fn scenes_read_nodes_and_multiline_values() {
        let text = "[ext_resource path=\"res://a/b.gd\" type=\"Script\" id=3]\n\n[node name=\"Root\" type=\"Node2D\"]\nscript = ExtResource( 3 )\nitems = [ \"a\",\r\n\"b\" ]\nx = 5\n\n[node name=\"Kid\" parent=\".\" index=\"2\"]\ny = - 1\n";
        let s = parse(&text.replace("\r\n", "\n"));
        assert_eq!(s.nodes.len(), 2);
        assert_eq!(s.nodes[0].props["script"], json!({"res": "a/b.gd"}));
        assert_eq!(s.nodes[0].props["items"], json!({"raw": "[ \"a\",\n\"b\" ]"}));
        assert_eq!(s.nodes[1].parent.as_deref(), Some("."));
        assert_eq!(s.nodes[1].index.as_deref(), Some("2"));
        assert_eq!(s.nodes[1].props["y"], json!(-1));
    }
}
