//! `yemi-convert <game folder> <output folder> [character ...]`
//!
//! Converts the original game's characters from a decompiled game folder (the wizard's
//! `game/` library folder) into RON files. Hand-tuned control schemes in `--controls DIR`
//! replace the generated ones.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut positional, mut controls) = (Vec::new(), None);
    while let Some(a) = args.next() {
        if a == "--controls" {
            controls = args.next().map(PathBuf::from);
        } else {
            positional.push(a);
        }
    }
    if positional.len() < 2 {
        eprintln!("usage: yemi-convert <game folder> <output folder> [--controls DIR] [character ...]");
        std::process::exit(2);
    }
    let started = std::time::Instant::now();
    let results = convert::pipeline::convert_all(&PathBuf::from(&positional[0]), &PathBuf::from(&positional[1]), controls.as_deref(), &positional[2..]);
    let mut failed = false;
    for r in results {
        match r {
            Ok(d) => println!("{}: {} moves, {} projectiles", d.id, d.moves, d.projectiles),
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
    }
    println!("done in {:.2}s", started.elapsed().as_secs_f32());
    std::process::exit(failed as i32);
}
