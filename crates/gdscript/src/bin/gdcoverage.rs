//! `gdcoverage FOLDER...`: parses every `.gd` file below the folders and lists what failed.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() {
    let mut stack: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let (mut ok, mut failed) = (0usize, 0usize);
    let mut by_msg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()));
        } else if p.extension().is_some_and(|e| e == "gd") {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            match gdscript::parse(&p.to_string_lossy(), &text) {
                Ok(_) => ok += 1,
                Err(e) => {
                    failed += 1;
                    let msg = match &e {
                        gdscript::Error::Parse { msg, .. } => msg.clone(),
                        other => other.to_string(),
                    };
                    by_msg.entry(msg).or_default().push(format!("{}: {e}", p.display()));
                }
            }
        }
    }
    println!("{ok} parsed, {failed} failed");
    let mut groups: Vec<_> = by_msg.into_iter().collect();
    groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (msg, files) in groups.iter().take(25) {
        println!("\n{} x {msg}", files.len());
        for f in files.iter().take(3) {
            println!("   {f}");
        }
    }
}
