//! Which parts of the game's scripting API a set of scripts uses. The importer's script
//! interpreter has to provide at least the calls the official characters use; comparing a mod
//! to that set tells how much more a given mod asks for.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

macro_rules! lazy_re {
    ($pat:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        CELL.get_or_init(|| Regex::new($pat).expect("valid pattern"))
    }};
}

/// Calls on the objects a script gets: `host.` (the fighter or object it belongs to), `fixed.`
/// (the fixed-point math library) and `Utils.` (shared helpers).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Api {
    pub host: BTreeMap<String, usize>,
    pub fixed: BTreeMap<String, usize>,
    pub utils: BTreeMap<String, usize>,
    /// Functions the scripts define themselves (a `host.` call to one of these is the mod's own
    /// code, not the game's API).
    pub defined: BTreeSet<String>,
    /// Things a sandboxed interpreter must refuse: (script, what).
    pub forbidden: Vec<(String, String)>,
}

impl Api {
    pub fn from_scripts<'a>(scripts: impl Iterator<Item = (&'a str, &'a str)>) -> Api {
        let mut api = Api::default();
        for (path, text) in scripts {
            api.add(path, text);
        }
        api
    }

    pub fn add(&mut self, path: &str, text: &str) {
        // comments would count as calls
        let code: String = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        for (re, map) in [
            (lazy_re!(r"\bhost\.(\w+)\s*\("), &mut self.host),
            (lazy_re!(r"\bfixed\.(\w+)\s*\("), &mut self.fixed),
            (lazy_re!(r"\bUtils\.(\w+)\s*\("), &mut self.utils),
        ] {
            for c in re.captures_iter(&code) {
                *map.entry(c[1].to_string()).or_default() += 1;
            }
        }
        for c in lazy_re!(r"(?m)^\s*func\s+(\w+)\s*\(").captures_iter(&code) {
            self.defined.insert(c[1].to_string());
        }
        for c in lazy_re!(r"\b(File\.new|Directory\.new|OS\.\w+|HTTPRequest|Steam\.\w+|ProjectSettings\.\w+|load_resource_pack|ResourceLoader\.\w+|get_tree\(\)|get_node\()").captures_iter(&code) {
            self.forbidden.push((path.to_string(), c[1].to_string()));
        }
    }

    /// Calls this set makes that `known` does not (per group).
    pub fn beyond(&self, known: &Surface) -> BTreeMap<&'static str, BTreeSet<String>> {
        let mut out: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
        for (group, mine, theirs) in [("host", &self.host, &known.host), ("fixed", &self.fixed, &known.fixed), ("Utils", &self.utils, &known.utils)] {
            let extra: BTreeSet<String> = mine.keys().filter(|k| !theirs.contains(*k) && !self.defined.contains(*k) && !known.defined.contains(*k)).cloned().collect();
            if !extra.is_empty() {
                out.insert(group, extra);
            }
        }
        out
    }
}

/// The calls the game's own scripts make.
#[derive(Clone, Debug, Default)]
pub struct Surface {
    pub host: BTreeSet<String>,
    pub fixed: BTreeSet<String>,
    pub utils: BTreeSet<String>,
    /// Every function the game's own scripts define (the API a mod can call on its objects).
    pub defined: BTreeSet<String>,
    pub scripts: usize,
}

impl Surface {
    /// Reads every `.gd` under the folders of the library's game copy that hold gameplay code.
    pub fn from_game(game: &std::path::Path) -> Surface {
        let mut api = Api::default();
        let mut scripts = 0;
        let mut stack: Vec<std::path::PathBuf> = ["characters", "projectile", "mechanics", "obj", "framework", "lib", "ui"].iter().map(|d| game.join(d)).collect();
        stack.push(game.join("ObjectState.gd"));
        stack.push(game.join("Global.gd"));
        stack.push(game.join("game.gd"));
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()));
            } else if p.extension().is_some_and(|e| e == "gd") {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    api.add(&p.to_string_lossy(), &t);
                    scripts += 1;
                }
            }
        }
        Surface { host: api.host.into_keys().collect(), fixed: api.fixed.into_keys().collect(), utils: api.utils.into_keys().collect(), defined: api.defined, scripts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_calls_and_ignores_comments() {
        let mut a = Api::default();
        a.add("a.gd", "func f():\n\thost.apply_force(1, 2)\n\t# host.old_call()\n\tvar x = fixed.mul(a, b)\n\tFile.new()\n");
        assert_eq!(a.host.get("apply_force"), Some(&1));
        assert!(!a.host.contains_key("old_call"));
        assert_eq!(a.fixed.get("mul"), Some(&1));
        assert_eq!(a.forbidden.len(), 1);
    }
}
