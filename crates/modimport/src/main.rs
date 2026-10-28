//! `yemi-modscan WORKSHOP_DIR [--game GAME_DIR] [--json]`: lists the Workshop mods and what is
//! in them. `--game` is the library's game folder (to compare scripts with the official ones).

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut dir, mut game, mut json) = (None, None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--json" => json = true,
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: yemi-modscan WORKSHOP_DIR [--game GAME_DIR] [--json]");
        std::process::exit(2);
    };
    let mut mods = Vec::new();
    for r in modimport::scan_workshop(&dir) {
        match r {
            Ok(m) => mods.push(m),
            Err(e) => eprintln!("skipped: {e}"),
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&mods).expect("serializable"));
        return;
    }
    let official = game.map(|g| modimport::Surface::from_game(&g));
    print!("{}", modimport::report::render(&mods, official.as_ref()));
}
