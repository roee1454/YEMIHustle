//! `yemi-modimport ARCHIVE_OR_ITEM_FOLDER LIBRARY_ROOT`: unpacks a mod into the library and
//! converts its fighters.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(item), Some(library)) = (args.next().map(PathBuf::from), args.next().map(PathBuf::from)) else {
        eprintln!("usage: yemi-modimport ITEM_FOLDER LIBRARY_ROOT");
        std::process::exit(2);
    };
    let m = match modimport::inspect_item(&item) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let start = std::time::Instant::now();
    match modimport::install::install(&m, &library) {
        Ok(results) => {
            for r in results {
                match r {
                    Ok(i) => println!("imported {} ({}): {} moves, {} projectiles", i.name, i.id, i.moves, i.projectiles),
                    Err(e) => println!("failed: {e}"),
                }
            }
            println!("done in {:.1} s", start.elapsed().as_secs_f32());
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
