//! A readable summary of what a scan found.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::{extends_histogram, Kind, ModInfo, Surface};

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

pub fn render(mods: &[ModInfo], official: Option<&Surface>) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Workshop scan\n");
    let _ = writeln!(s, "| item | kind | fighters | scripts | projectile scenes | size |");
    let _ = writeln!(s, "|---|---|---|---|---|---|");
    for m in mods {
        let name = if m.title.is_empty() { &m.meta.friendly_name } else { &m.title };
        let kind = match m.kind {
            Kind::Character => "character",
            Kind::Tool => "tool / UI",
            Kind::Style => "style",
            Kind::Unknown => "unknown",
        };
        let fighters = m.characters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
        let _ = writeln!(s, "| {name} ({}) | {kind} | {fighters} | {} | {} | {} |", m.id, m.counts.scripts, m.projectile_scenes.len(), mb(m.counts.bytes));
    }

    let mut wanted: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // call -> (mods, uses)
    for m in mods.iter().filter(|m| m.kind == Kind::Character) {
        let _ = writeln!(s, "\n## {} ({})\n", if m.title.is_empty() { &m.meta.friendly_name } else { &m.title }, m.id);
        let _ = writeln!(s, "- author {}, version {}, requires {:?}", m.meta.author, m.meta.version, m.meta.requires);
        for c in &m.characters {
            let _ = writeln!(s, "- fighter **{}** from `{}`", c.name, c.scene);
        }
        let c = &m.counts;
        let _ = writeln!(s, "- {} files: {} images, {} sounds, {} scenes, {} scripts, {} already imported", c.files, c.images, c.sounds, c.scenes, c.scripts, c.imported);
        let ext: Vec<String> = extends_histogram(m).into_iter().take(6).map(|(k, n)| format!("{k} x{n}")).collect();
        let _ = writeln!(s, "- scripts extend: {}", ext.join(", "));
        if !m.projectile_scenes.is_empty() {
            let _ = writeln!(s, "- projectile scenes: {}", m.projectile_scenes.iter().map(|p| p.rsplit('/').next().unwrap_or(p)).collect::<Vec<_>>().join(", "));
        }
        let calls: usize = m.api.host.values().chain(m.api.fixed.values()).chain(m.api.utils.values()).sum();
        let _ = writeln!(s, "- {} distinct host calls, {} fixed, {} Utils ({} call sites)", m.api.host.len(), m.api.fixed.len(), m.api.utils.len(), calls);
        if let Some(o) = official {
            let beyond = m.api.beyond(o);
            if beyond.is_empty() {
                let _ = writeln!(s, "- everything it calls is something the official characters call too");
            }
            for (group, set) in &beyond {
                let _ = writeln!(s, "- beyond the official scripts ({group}): {}", set.iter().cloned().collect::<Vec<_>>().join(", "));
                for name in set {
                    let uses = match *group {
                        "host" => m.api.host.get(name),
                        "fixed" => m.api.fixed.get(name),
                        _ => m.api.utils.get(name),
                    };
                    let e = wanted.entry(format!("{group}.{name}")).or_default();
                    e.0 += 1;
                    e.1 += uses.copied().unwrap_or(0);
                }
            }
        }
        if !m.api.forbidden.is_empty() {
            let mut f: BTreeMap<&str, usize> = BTreeMap::new();
            for (_, what) in &m.api.forbidden {
                *f.entry(what.as_str()).or_default() += 1;
            }
            let _ = writeln!(s, "- would be refused by a sandbox: {}", f.iter().map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", "));
        }
    }
    if let Some(o) = official {
        let _ = writeln!(s, "\n## What the official scripts cover\n");
        let _ = writeln!(s, "{} gameplay scripts use {} distinct host calls, {} fixed, {} Utils.", o.scripts, o.host.len(), o.fixed.len(), o.utils.len());
    }
    if !wanted.is_empty() {
        let mut v: Vec<_> = wanted.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(b.1 .1.cmp(&a.1 .1)).then(a.0.cmp(&b.0)));
        let _ = writeln!(s, "\n## Calls the interpreter needs beyond the official characters\n");
        let _ = writeln!(s, "| call | mods | uses |\n|---|---|---|");
        for (k, (mods, uses)) in v {
            let _ = writeln!(s, "| {k} | {mods} | {uses} |");
        }
    }
    s
}
