//! `yemi-layout CHARACTERS_DIR [ID...] [--force]`: writes the automatic button layout
//! (`<id>_controls.ron`) and the move audit (`<id>_moves.md`) for each character (all of them
//! when no id is given). Hand-edited controls files are kept unless `--force`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let force = args.iter().any(|a| a == "--force");
    let mut rest = args.iter().filter(|a| !a.starts_with("--"));
    let Some(dir) = rest.next().map(std::path::PathBuf::from) else {
        eprintln!("usage: yemi-layout CHARACTERS_DIR [ID...] [--force]");
        std::process::exit(2);
    };
    let mut ids: Vec<String> = rest.cloned().collect();
    if ids.is_empty() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(id) = n.strip_suffix(".ron").filter(|s| !s.contains("_controls") && !s.contains("_fx")) {
                ids.push(id.to_string());
            }
        }
        ids.sort();
    }
    for id in ids {
        match sim::layout::write_for(&dir, &id, force) {
            Ok(l) => println!(
                "{id:<16} {:>3} moves: {:>3} choices ({} in reserve), {:>2} follow-ups, {:>2} movement, {:>2} system, {:>2} internal",
                l.entries.len(),
                l.count("choice"),
                l.reserve().count(),
                l.count("follow-up"),
                l.count("movement"),
                l.count("system"),
                l.count("internal")
            ),
            Err(e) => eprintln!("{id}: {e}"),
        }
    }
}
