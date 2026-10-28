//! Drafts a control scheme for a converted character: every move the player can start from
//! neutral gets a button and direction, follow-ups become `cancel_only` binds. A port of
//! `tools/make_controls.py`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::ron::sid;

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        CELL.get_or_init(|| Regex::new($pat).expect("valid pattern"))
    }};
}

const ENGINE_IDS: [&str; 13] = [
    "idle", "wait", "fall", "landing", "jump", "double_jump", "super_jump", "dash_forward", "dash_backward", "air_dash", "charge_dash",
    "charge_dash_forward", "dash_startup",
];
const SKIP_BUTTONS: [&str; 5] = ["Guard", "Jump", "Dash", "Taunt", "Burst"];
const DIR_ORDER: [&str; 4] = ["Down", "Forward", "Up", "Back"];
const OUT_DIR_ORDER: [&str; 5] = ["Down", "Forward", "Up", "Back", "Any"];

struct Mv {
    id: String,
    name: String,
    stance: String,
    button: String,
    dir: Option<String>,
}

pub struct Controls {
    pub text: String,
    pub neutral: usize,
    pub cancel: usize,
    /// Groups of moves left without a button slot: (stance, title, ids).
    pub unreached: Vec<(String, String, Vec<String>)>,
}

fn heavy_for(button: &str) -> &str {
    match button {
        "Super" => "Heavy",
        other => other,
    }
}

pub fn make_controls(data: &Value, ron: &str, char_id: &str) -> Controls {
    let mut js: HashMap<String, &Value> = HashMap::new();
    for s in data["states"].as_array().into_iter().flatten() {
        js.insert(sid(s["name"].as_str().unwrap_or("")), s);
    }
    let mut moves: Vec<Mv> = Vec::new();
    for part in lazy_re!(r#"\n {16}id: ""#).split(ron).skip(1) {
        let mid = part.split('"').next().unwrap_or("").to_string();
        let head: String = part.chars().take(300).collect();
        if !lazy_re!(r"\n {16}name:").is_match(&head) {
            continue;
        }
        let stance = lazy_re!(r"\n {16}stance: (\w+)").captures(part).map(|c| c[1].to_string());
        let Some(inp) = lazy_re!(r"\n {16}input: Input\((.*?)\),\n").captures(part) else { continue };
        let button = lazy_re!(r"button: (\w+)").captures(&inp[1]).map(|c| c[1].to_string()).unwrap_or_default();
        let dir = lazy_re!(r"direction: (\w+)").captures(&inp[1]).map(|c| c[1].to_string());
        let name = lazy_re!(r#"name: "([^"]*)""#).captures(part).map(|c| c[1].to_string()).unwrap_or_default();
        moves.push(Mv { id: mid, name, stance: stance.unwrap_or_else(|| "Any".into()), button, dir });
    }

    let (mut neutral, mut cancel): (Vec<&Mv>, Vec<&Mv>) = (vec![], vec![]);
    for m in &moves {
        let Some(s) = js.get(&m.id) else { continue };
        if ENGINE_IDS.contains(&m.id.as_str()) || SKIP_BUTTONS.contains(&m.button.as_str()) {
            continue;
        }
        let k = &s["key"];
        let frm: HashSet<&str> = k["interrupt_from"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let truthy = |key: &str| match &k[key] {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            Value::String(s) => !s.is_empty(),
            _ => true,
        };
        if !matches!(k["type"].as_str(), Some("Attack" | "Special" | "Super")) {
            continue;
        }
        if frm.iter().any(|f| f.contains("Throw")) || (!truthy("selectable") && !truthy("combo_only") && frm.is_empty()) {
            continue;
        }
        if truthy("combo_only") {
            cancel.push(m);
        } else if frm.contains("Grounded") || frm.contains("Aerial") {
            neutral.push(m);
        } else if !frm.is_empty() {
            cancel.push(m);
        }
    }

    struct Group {
        name: String,
        dir: Option<String>,
        ids: Vec<String>,
    }
    let groups_for = |sset: &[&str], button: &str| -> Vec<Group> {
        let mut gs: Vec<Group> = Vec::new();
        for m in &neutral {
            if heavy_for(&m.button) != button || !sset.contains(&m.stance.as_str()) {
                continue;
            }
            match gs.iter_mut().find(|g| g.name == m.name && g.dir == m.dir) {
                Some(g) => g.ids.push(m.id.clone()),
                None => gs.push(Group { name: m.name.clone(), dir: m.dir.clone(), ids: vec![m.id.clone()] }),
            }
        }
        gs
    };

    type Bind = (String, String, String, Vec<String>, bool);
    let mut binds: Vec<Bind> = Vec::new();
    let mut unreached = Vec::new();
    for (stance, sset) in [("Ground", ["Ground", "Any"]), ("Air", ["Air", "Any"])] {
        let mut taken: HashMap<(String, String), Vec<String>> = HashMap::new();
        let mut pending: Vec<(String, Group)> = Vec::new();
        for button in ["Light", "Special", "Heavy", "Grab"] {
            let mut gs = groups_for(&sset, button);
            let mut rest = Vec::new();
            for g in gs.drain(..) {
                match g.dir.as_deref() {
                    Some(d @ ("Forward" | "Back" | "Up" | "Down")) => taken.entry((button.to_string(), d.to_string())).or_default().extend(g.ids),
                    _ => rest.push(g),
                }
            }
            let mut free: Vec<&str> = DIR_ORDER.iter().copied().filter(|d| !taken.contains_key(&(button.to_string(), d.to_string()))).collect();
            for g in rest {
                if !taken.keys().any(|(b, d)| b == button && d == "Any") {
                    taken.insert((button.to_string(), "Any".into()), g.ids.clone());
                } else if !free.is_empty() {
                    taken.insert((button.to_string(), free.remove(0).to_string()), g.ids.clone());
                } else {
                    pending.push((button.to_string(), g));
                }
            }
        }
        // overflow: free slots on Heavy, then Grab
        for (_, g) in pending {
            let mut placed = false;
            'outer: for b2 in ["Heavy", "Grab", "Special", "Light"] {
                for d in ["Any", "Down", "Forward", "Up", "Back"] {
                    let key = (b2.to_string(), d.to_string());
                    if !taken.contains_key(&key) {
                        taken.insert(key, g.ids.clone());
                        placed = true;
                        break 'outer;
                    }
                }
            }
            if !placed {
                unreached.push((stance.to_string(), g.name.clone(), g.ids.clone()));
            }
        }
        for button in ["Light", "Heavy", "Special", "Grab"] {
            for d in ["Down", "Forward", "Up", "Back", "Any"] {
                if let Some(ids) = taken.get(&(button.to_string(), d.to_string())) {
                    binds.push((stance.to_string(), button.to_string(), d.to_string(), ids.clone(), false));
                }
            }
        }
    }

    // cancel-only binds: no slot limit, the cancel's tag filter picks the move
    let mut cb: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    for m in &cancel {
        for (stance, sset) in [("Ground", ["Ground", "Any"]), ("Air", ["Air", "Any"])] {
            if sset.contains(&m.stance.as_str()) {
                cb.entry((stance.to_string(), heavy_for(&m.button).to_string(), m.dir.clone().unwrap_or_else(|| "Any".into()))).or_default().push(m.id.clone());
            }
        }
    }
    for stance in ["Ground", "Air"] {
        for button in ["Light", "Heavy", "Special", "Grab"] {
            for d in OUT_DIR_ORDER {
                if let Some(ids) = cb.get(&(stance.to_string(), button.to_string(), d.to_string())) {
                    binds.push((stance.to_string(), button.to_string(), d.to_string(), ids.clone(), true));
                }
            }
        }
    }

    let names: HashMap<&str, &str> = moves.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect();
    let mut lines: Vec<String> = vec![
        format!("// Control scheme for the converted {char_id} (PRIVATE: names moves from the original game)."),
        "// DRAFT generated by the yemi-convert crate: edit freely, press F5 in the game to reload.".into(),
        "// Light = attacks, Special = specials, Heavy = supers (and overflow), Grab = grabs.".into(),
        "// The first bind of a button+direction that has a usable move wins. Directions are relative to facing.".into(),
        "// `cancel_only` binds are follow-ups reachable only through a cancel window.".into(),
        "ControlScheme(".into(),
        format!("    character: \"{char_id}\","),
        "    binds: [".into(),
    ];
    for (stance, button, d, ids, co) in &binds {
        let mut parts = vec![format!("button: {button}")];
        if d != "Any" {
            parts.push(format!("dir: {d}"));
        }
        parts.push(format!("stance: {stance}"));
        if *co {
            parts.push("cancel_only: true".into());
        }
        parts.push(format!("moves: [{}]", ids.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ")));
        let titles: Vec<&str> = ids.iter().take(3).map(|i| names.get(i.as_str()).copied().unwrap_or("")).collect();
        lines.push(format!("        Bind({}),  // {}", parts.join(", "), titles.join(", ")));
    }
    lines.extend(["    ],".to_string(), ")".to_string(), String::new()]);
    Controls { text: lines.join("\n"), neutral: neutral.len(), cancel: cancel.len(), unreached }
}
