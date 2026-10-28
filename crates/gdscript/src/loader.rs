//! Finding the scripts a script extends or refers to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::ast::Script;
use crate::parse;

pub trait Loader: Send + Sync {
    /// A script by `res://` path (the prefix may be left off).
    fn by_path(&self, path: &str) -> Option<Arc<Script>>;
    /// A script by its `class_name`.
    fn by_class(&self, name: &str) -> Option<Arc<Script>>;
    /// A script that is a singleton in the original (`Utils`).
    fn autoload(&self, name: &str) -> Option<Arc<Script>>;
    /// Whether the embedding implements this script itself (an engine base class).
    fn is_native(&self, _path: &str) -> bool {
        false
    }
}

/// Reads scripts from folders laid out like the original's `res://`: first folder that has the
/// file wins (a mod's folder before the game's).
pub struct FsLoader {
    roots: Vec<PathBuf>,
    classes: HashMap<String, String>,
    autoloads: HashMap<String, String>,
    native: Vec<String>,
    cache: RwLock<HashMap<String, Option<Arc<Script>>>>,
}

fn rel(p: &str) -> &str {
    p.strip_prefix("res://").unwrap_or(p)
}

impl FsLoader {
    pub fn new(roots: Vec<PathBuf>) -> FsLoader {
        let mut classes = HashMap::new();
        let mut autoloads = HashMap::new();
        for root in &roots {
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "gd") {
                        let Ok(text) = std::fs::read_to_string(&p) else { continue };
                        // `class_name X` sits in the first lines
                        for line in text.lines().take(12) {
                            if let Some(name) = line.strip_prefix("class_name ") {
                                let name = name.split([',', ' ']).next().unwrap_or("").trim();
                                if !name.is_empty() {
                                    classes.entry(name.to_string()).or_insert_with(|| p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
                                }
                            }
                        }
                    }
                }
            }
            if let Ok(text) = std::fs::read_to_string(root.join("project.godot")) {
                let mut in_autoload = false;
                for line in text.lines() {
                    if line.starts_with('[') {
                        in_autoload = line.trim() == "[autoload]";
                    } else if in_autoload {
                        if let Some((name, path)) = line.split_once('=') {
                            let path = path.trim().trim_matches('"').trim_start_matches('*');
                            autoloads.insert(name.trim().to_string(), rel(path).to_string());
                        }
                    }
                }
            }
        }
        FsLoader { roots, classes, autoloads, native: Vec::new(), cache: RwLock::new(HashMap::new()) }
    }

    /// Marks scripts (by `res://` path) as implemented by the embedding.
    pub fn with_native<I: IntoIterator<Item = S>, S: Into<String>>(mut self, paths: I) -> FsLoader {
        self.native.extend(paths.into_iter().map(|p| rel(&p.into()).to_string()));
        self
    }

    fn find(&self, path: &str) -> Option<PathBuf> {
        let r = rel(path);
        self.roots.iter().map(|d| d.join(r)).find(|p| Path::new(p).is_file())
    }

    /// Every script path the loader knows by class name (for coverage runs).
    pub fn class_paths(&self) -> impl Iterator<Item = &String> {
        self.classes.values()
    }
}

impl Loader for FsLoader {
    fn by_path(&self, path: &str) -> Option<Arc<Script>> {
        let key = rel(path).to_string();
        if let Some(hit) = self.cache.read().unwrap().get(&key) {
            return hit.clone();
        }
        let script = self.find(path).and_then(|p| std::fs::read_to_string(&p).ok()).and_then(|t| parse(&key, &t).ok()).map(Arc::new);
        self.cache.write().unwrap().insert(key, script.clone());
        script
    }

    fn by_class(&self, name: &str) -> Option<Arc<Script>> {
        self.classes.get(name).and_then(|p| self.by_path(p))
    }

    fn autoload(&self, name: &str) -> Option<Arc<Script>> {
        self.autoloads.get(name).and_then(|p| self.by_path(p))
    }

    fn is_native(&self, path: &str) -> bool {
        self.native.iter().any(|n| n == rel(path))
    }
}
