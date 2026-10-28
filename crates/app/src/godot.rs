//! A reader for Godot 3 text scenes and resources (`.tscn` / `.tres`), enough to load the
//! original game's effect scenes: values, external and internal resources, and the node tree
//! (with scenes that inherit from other scenes).

use std::collections::HashMap;

/// A property value as written in the file.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Vec2(f32, f32),
    Color([f32; 4]),
    Ext(u32),
    Sub(u32),
    Arr(Vec<V>),
    Dict(Vec<(V, V)>),
    /// Any other constructor, e.g. `PoolRealArray( ... )`, `Rect2( ... )`.
    Call(String, Vec<V>),
}

impl V {
    pub fn num(&self) -> Option<f32> {
        match self {
            V::Num(n) => Some(*n as f32),
            V::Bool(b) => Some(*b as u8 as f32),
            _ => None,
        }
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            V::Bool(b) => Some(*b),
            V::Num(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    pub fn vec2(&self) -> Option<(f32, f32)> {
        match self {
            V::Vec2(x, y) => Some((*x, *y)),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            V::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Numbers inside an array or a `Pool*Array( ... )`.
    pub fn numbers(&self) -> Vec<f32> {
        let items = match self {
            V::Arr(a) | V::Call(_, a) => a,
            _ => return vec![],
        };
        items.iter().filter_map(|v| v.num()).collect()
    }

    pub fn items(&self) -> &[V] {
        match self {
            V::Arr(a) | V::Call(_, a) => a,
            _ => &[],
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        match self {
            V::Dict(d) => d.iter().find(|(k, _)| k.str() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
}

pub type Props = Vec<(String, V)>;

pub fn prop<'a>(props: &'a Props, key: &str) -> Option<&'a V> {
    props.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub ty: Option<String>,
    /// Path of the parent relative to the scene root (`.` is the root itself); None for the root.
    pub parent: Option<String>,
    pub instance: Option<u32>,
    pub props: Props,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    /// id -> (path without `res://`, type)
    pub ext: HashMap<u32, (String, String)>,
    /// id -> (type, properties)
    pub sub: HashMap<u32, (String, Props)>,
    pub nodes: Vec<Node>,
    /// The `[resource]` section of a `.tres`.
    pub resource: Props,
}

impl Doc {
    pub fn ext_path(&self, v: &V) -> Option<&str> {
        match v {
            V::Ext(id) => self.ext.get(id).map(|e| e.0.as_str()),
            _ => None,
        }
    }

    pub fn sub_res(&self, v: &V) -> Option<&(String, Props)> {
        match v {
            V::Sub(id) => self.sub.get(id),
            _ => None,
        }
    }

    /// Path of a node relative to the root (`.` for the root).
    pub fn path_of(&self, n: &Node) -> String {
        match n.parent.as_deref() {
            None => ".".into(),
            Some(".") => n.name.clone(),
            Some(p) => format!("{p}/{}", n.name),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.peek() == c {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_' || self.s[self.i] == b'/') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }

    fn string(&mut self) -> String {
        // at the opening quote
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() && self.s[self.i] != b'"' {
            if self.s[self.i] == b'\\' && self.i + 1 < self.s.len() {
                self.i += 1;
                out.push(match self.s[self.i] {
                    b'n' => b'\n',
                    b't' => b'\t',
                    c => c,
                });
            } else {
                out.push(self.s[self.i]);
            }
            self.i += 1;
        }
        self.i += 1;
        String::from_utf8_lossy(&out).into_owned()
    }

    fn value(&mut self) -> V {
        self.ws();
        match self.peek() {
            b'"' => V::Str(self.string()),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    if self.eat(b']') {
                        break;
                    }
                    items.push(self.value());
                    self.eat(b',');
                    if self.i >= self.s.len() {
                        break;
                    }
                }
                V::Arr(items)
            }
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    if self.eat(b'}') {
                        break;
                    }
                    let k = self.value();
                    self.eat(b':');
                    let v = self.value();
                    items.push((k, v));
                    self.eat(b',');
                    if self.i >= self.s.len() {
                        break;
                    }
                }
                V::Dict(items)
            }
            c if c == b'-' || c == b'.' || c.is_ascii_digit() => {
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || b".eE+-".contains(&self.s[self.i])) {
                    self.i += 1;
                }
                V::Num(std::str::from_utf8(&self.s[st..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0))
            }
            _ => {
                let id = self.ident();
                match id.as_str() {
                    "true" => return V::Bool(true),
                    "false" => return V::Bool(false),
                    "null" | "" => {
                        if id.is_empty() {
                            self.i += 1; // skip an unexpected byte
                        }
                        return V::Nil;
                    }
                    _ => {}
                }
                if !self.eat(b'(') {
                    return V::Str(id);
                }
                let mut args = Vec::new();
                loop {
                    if self.eat(b')') {
                        break;
                    }
                    args.push(self.value());
                    self.eat(b',');
                    if self.i >= self.s.len() {
                        break;
                    }
                }
                let n = |k: usize| args.get(k).and_then(|v| v.num()).unwrap_or(0.0);
                match id.as_str() {
                    "Vector2" => V::Vec2(n(0), n(1)),
                    "Color" => V::Color([n(0), n(1), n(2), if args.len() > 3 { n(3) } else { 1.0 }]),
                    "ExtResource" => V::Ext(n(0) as u32),
                    "SubResource" => V::Sub(n(0) as u32),
                    _ => V::Call(id, args),
                }
            }
        }
    }

    /// `[tag key=value ...]`
    fn header(&mut self) -> (String, Props) {
        self.i += 1;
        let tag = self.ident();
        let mut attrs = Vec::new();
        loop {
            self.ws();
            if self.peek() == b']' || self.i >= self.s.len() {
                self.i += 1;
                break;
            }
            let k = self.ident();
            if k.is_empty() {
                self.i += 1;
                continue;
            }
            if self.eat(b'=') {
                let v = self.value();
                attrs.push((k, v));
            }
        }
        (tag, attrs)
    }
}

pub fn parse(text: &str) -> Doc {
    let mut p = P { s: text.as_bytes(), i: 0 };
    let mut doc = Doc::default();
    // the section properties are being read into
    enum Cur {
        None,
        Sub(u32),
        Node(usize),
        Resource,
    }
    let mut cur = Cur::None;
    loop {
        p.ws();
        if p.i >= p.s.len() {
            break;
        }
        if p.peek() == b'[' {
            let (tag, attrs) = p.header();
            let a = |k: &str| prop(&attrs, k).cloned();
            match tag.as_str() {
                "ext_resource" => {
                    let path = a("path").and_then(|v| v.str().map(|s| s.trim_start_matches("res://").to_string())).unwrap_or_default();
                    let ty = a("type").and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
                    let id = a("id").and_then(|v| v.num()).unwrap_or(0.0) as u32;
                    doc.ext.insert(id, (path, ty));
                    cur = Cur::None;
                }
                "sub_resource" => {
                    let ty = a("type").and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
                    let id = a("id").and_then(|v| v.num()).unwrap_or(0.0) as u32;
                    doc.sub.insert(id, (ty, vec![]));
                    cur = Cur::Sub(id);
                }
                "node" => {
                    doc.nodes.push(Node {
                        name: a("name").and_then(|v| v.str().map(str::to_string)).unwrap_or_default(),
                        ty: a("type").and_then(|v| v.str().map(str::to_string)),
                        parent: a("parent").and_then(|v| v.str().map(str::to_string)),
                        instance: match a("instance") {
                            Some(V::Ext(id)) => Some(id),
                            _ => None,
                        },
                        props: vec![],
                    });
                    cur = Cur::Node(doc.nodes.len() - 1);
                }
                "resource" => cur = Cur::Resource,
                _ => cur = Cur::None,
            }
            continue;
        }
        // key = value
        let st = p.i;
        while p.i < p.s.len() && p.s[p.i] != b'=' && p.s[p.i] != b'\n' {
            p.i += 1;
        }
        let key = String::from_utf8_lossy(&p.s[st..p.i]).trim().to_string();
        if p.peek() != b'=' {
            continue;
        }
        p.i += 1;
        let v = p.value();
        match cur {
            Cur::Sub(id) => doc.sub.get_mut(&id).expect("section").1.push((key, v)),
            Cur::Node(n) => doc.nodes[n].props.push((key, v)),
            Cur::Resource => doc.resource.push((key, v)),
            Cur::None => {}
        }
    }
    doc
}

// ---------------------------------------------------------------------------------------------
// Curves and gradients
// ---------------------------------------------------------------------------------------------

/// A Godot `Curve`: points with tangents, sampled like Godot 3 (cubic Bezier in y between
/// points, linear in x).
#[derive(Clone, Debug, Default)]
pub struct Curve {
    /// (x, y, left tangent, right tangent)
    points: Vec<(f32, f32, f32, f32)>,
}

impl Curve {
    pub fn from_props(props: &Props) -> Self {
        let mut points = Vec::new();
        if let Some(V::Arr(d)) = prop(props, "_data") {
            for c in d.chunks(5) {
                if let (Some(V::Vec2(x, y)), Some(l), Some(r)) = (c.first(), c.get(1).and_then(|v| v.num()), c.get(2).and_then(|v| v.num())) {
                    points.push((*x, *y, l, r));
                }
            }
        }
        Curve { points }
    }

    pub fn sample(&self, x: f32) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => 1.0,
            1 => p[0].1,
            _ => {
                if x <= p[0].0 {
                    return p[0].1;
                }
                if x >= p[p.len() - 1].0 {
                    return p[p.len() - 1].1;
                }
                let i = p.windows(2).position(|w| x >= w[0].0 && x <= w[1].0).unwrap_or(0);
                let (a, b) = (p[i], p[i + 1]);
                let d = b.0 - a.0;
                if d <= 0.0 {
                    return b.1;
                }
                let t = (x - a.0) / d;
                let (y0, y1, y2, y3) = (a.1, a.1 + d * a.3 / 3.0, b.1 - d * b.2 / 3.0, b.1);
                let mt = 1.0 - t;
                y0 * mt * mt * mt + 3.0 * y1 * mt * mt * t + 3.0 * y2 * mt * t * t + y3 * t * t * t
            }
        }
    }
}

/// A Godot `Gradient`: colors at offsets, linearly interpolated.
#[derive(Clone, Debug, Default)]
pub struct Gradient {
    stops: Vec<(f32, [f32; 4])>,
}

impl Gradient {
    pub fn from_props(props: &Props) -> Self {
        let offsets = prop(props, "offsets").map(|v| v.numbers()).unwrap_or_default();
        let colors = prop(props, "colors").map(|v| v.numbers()).unwrap_or_default();
        let stops = offsets.iter().enumerate().filter_map(|(i, o)| colors.get(i * 4..i * 4 + 4).map(|c| (*o, [c[0], c[1], c[2], c[3]]))).collect();
        Gradient { stops }
    }

    pub fn sample(&self, x: f32) -> [f32; 4] {
        let s = &self.stops;
        if s.is_empty() {
            return [1.0; 4];
        }
        if x <= s[0].0 {
            return s[0].1;
        }
        for w in s.windows(2) {
            if x <= w[1].0 {
                let t = if w[1].0 > w[0].0 { (x - w[0].0) / (w[1].0 - w[0].0) } else { 1.0 };
                return std::array::from_fn(|k| w[0].1[k] + (w[1].1[k] - w[0].1[k]) * t);
            }
        }
        s[s.len() - 1].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"[gd_scene load_steps=3 format=2]

[ext_resource path="res://fx/ParticleEffect.tscn" type="PackedScene" id=1]
[ext_resource path="res://fx/hit1.png" type="Texture" id=4]

[sub_resource type="SpriteFrames" id=5]
animations = [ {
"frames": [ ExtResource( 4 ), ExtResource( 4 ) ],
"loop": true,
"name": "default",
"speed": 5.0
} ]

[sub_resource type="Curve" id=2]
_data = [ Vector2( 0, 0 ), 0.0, 0.0, 0, 0, Vector2( 1, 1 ), 0.0, 0.0, 0, 0 ]

[node name="ParticleEffect" instance=ExtResource( 1 )]
z_index = 100

[node name="CPUParticles2D" type="CPUParticles2D" parent="." index="1"]
amount = 32
gravity = Vector2( 0, -9.8 )
color = Color( 1, 0.5, 0, 1 )
scale_amount_curve = SubResource( 2 )
"#;

    #[test]
    fn reads_resources_and_nodes() {
        let d = parse(SCENE);
        assert_eq!(d.ext[&4].0, "fx/hit1.png");
        let root = &d.nodes[0];
        assert_eq!(root.instance, Some(1));
        assert_eq!(prop(&root.props, "z_index").and_then(V::num), Some(100.0));
        let p = &d.nodes[1];
        assert_eq!(p.ty.as_deref(), Some("CPUParticles2D"));
        assert_eq!(d.path_of(p), "CPUParticles2D");
        assert_eq!(prop(&p.props, "gravity"), Some(&V::Vec2(0.0, -9.8)));
        assert_eq!(prop(&p.props, "color"), Some(&V::Color([1.0, 0.5, 0.0, 1.0])));
        let frames = &d.sub[&5].1;
        let anim = &prop(frames, "animations").unwrap().items()[0];
        assert_eq!(anim.get("frames").unwrap().items().len(), 2);
        let curve = Curve::from_props(&d.sub[&2].1);
        assert!((curve.sample(0.5) - 0.5).abs() < 0.01);
    }
}
