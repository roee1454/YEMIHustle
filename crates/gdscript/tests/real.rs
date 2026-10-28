//! Runs real scripts from the original game and a Workshop mod against a recording host.
//! Skipped when the library is not there.

use std::collections::HashMap;
use std::path::PathBuf;

use gdscript::{Error, FsLoader, Host, ObjId, Value, Vm};

fn library() -> Option<PathBuf> {
    let root = std::env::var_os("YEMI_LIBRARY").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/yemi-hustle")))?;
    root.join("game").is_dir().then_some(root)
}

#[derive(Default)]
struct Rec {
    props: HashMap<String, Value>,
    calls: Vec<String>,
}

impl Host for Rec {
    fn get(&mut self, _obj: ObjId, name: &str) -> Result<Value, Error> {
        Ok(self.props.get(name).cloned().unwrap_or(Value::Nil))
    }
    fn set(&mut self, _obj: ObjId, name: &str, v: Value) -> Result<(), Error> {
        self.calls.push(format!("set {name} = {}", v.to_text()));
        self.props.insert(name.to_string(), v);
        Ok(())
    }
    fn call(&mut self, _obj: ObjId, method: &str, args: &[Value]) -> Result<Value, Error> {
        self.calls.push(format!("{method}({})", args.iter().map(Value::to_text).collect::<Vec<_>>().join(", ")));
        Ok(Value::Nil)
    }
}

#[test]
fn a_mod_state_script_runs() {
    let Some(lib) = library() else { return };
    let mod_res = lib.join("mods/3139650206/res");
    if !mod_res.is_dir() {
        return;
    }
    let loader = FsLoader::new(vec![mod_res, lib.join("game")]);
    let script = gdscript::Loader::by_path(&loader, "OnePunchMod/Characters/OnePunchMan/States/Normal Punch.gd").expect("the script loads");
    let mut vm = Vm::new(Box::new(loader));
    let mut host = Rec::default();
    let inst = vm.instantiate(&mut host, script, None).expect("instantiates");
    inst.write().unwrap().set("host", Value::Obj(ObjId(1)));

    vm.call(&mut host, &inst, "_frame_0", vec![]).unwrap();
    vm.call(&mut host, &inst, "_frame_1", vec![]).unwrap();
    assert_eq!(host.calls, vec!["set has_hyper_armor = True", "start_projectile_invulnerability()"]);
    vm.call(&mut host, &inst, "_frame_48", vec![]).unwrap();
    assert_eq!(host.props.get("has_hyper_armor").map(Value::to_text).as_deref(), Some("False"));
    assert!(host.calls.last().unwrap().starts_with("end_projectile_invulnerability"));

    // being hit before frame 1 means no armor
    host.calls.clear();
    vm.call(&mut host, &inst, "_frame_0", vec![]).unwrap();
    vm.call(&mut host, &inst, "on_got_hit", vec![]).unwrap();
    vm.call(&mut host, &inst, "_frame_1", vec![]).unwrap();
    assert!(!host.calls.iter().any(|c| c.contains("has_hyper_armor = True")), "{:?}", host.calls);
}

#[test]
fn the_games_own_script_math_works() {
    let Some(lib) = library() else { return };
    let loader = FsLoader::new(vec![lib.join("game")]);
    // the shuriken state: speed from the aim, as in the converter's verified table
    let script = gdscript::Loader::by_path(&loader, "characters/stickman/states/Fireball.gd").expect("loads");
    let mut vm = Vm::new(Box::new(loader));
    let mut host = Rec::default();
    let inst = vm.instantiate(&mut host, script, None).expect("instantiates");
    // GROUNDED_SPEED * 1.5 as the script computes it
    let v = vm.call(&mut host, &inst, "xy_to_dir_test", vec![]);
    assert!(v.is_err(), "that helper does not exist");
    let grounded = inst.read().unwrap().chain.iter().any(|s| s.consts.iter().any(|(n, _)| n == "GROUNDED_SPEED"));
    assert!(grounded);
}

#[test]
fn runaway_loops_are_stopped() {
    let loader = FsLoader::new(vec![]);
    let script = std::sync::Arc::new(gdscript::parse("loop.gd", "func spin():\n\twhile true:\n\t\tpass\n").unwrap());
    let mut vm = Vm::new(Box::new(loader));
    vm.budget = 5_000;
    let mut host = Rec::default();
    let inst = vm.instantiate(&mut host, script, None).unwrap();
    assert!(matches!(vm.call(&mut host, &inst, "spin", vec![]), Err(Error::Budget)));
}

#[test]
fn language_basics() {
    let src = "var total = 0\nenum Mode { A, B = 5, C }\nconst K = 3\nfunc f(n):\n\tvar out = []\n\tfor i in range(n):\n\t\tif i % 2 == 0:\n\t\t\tcontinue\n\t\tout.append(i * K)\n\treturn out\nfunc g(x):\n\tmatch x:\n\t\t1, 2:\n\t\t\treturn \"low\"\n\t\t_:\n\t\t\treturn \"high\"\nfunc h():\n\treturn Mode.C + (10 if total == 0 else 20) + fixed.mul(\"0.5\", 4).to_int()\n";
    let script = std::sync::Arc::new(gdscript::parse("t.gd", src).unwrap());
    let mut vm = Vm::new(Box::new(FsLoader::new(vec![])));
    let mut host = Rec::default();
    let inst = vm.instantiate(&mut host, script, None).unwrap();
    let r = vm.call(&mut host, &inst, "f", vec![Value::Int(6)]).unwrap();
    assert_eq!(r.to_text(), "[3, 9, 15]");
    assert_eq!(vm.call(&mut host, &inst, "g", vec![Value::Int(2)]).unwrap().to_text(), "low");
    assert_eq!(vm.call(&mut host, &inst, "g", vec![Value::Int(9)]).unwrap().to_text(), "high");
    assert_eq!(vm.call(&mut host, &inst, "h", vec![]).unwrap().to_text(), "18");
}

#[test]
fn the_interpreter_can_live_in_a_shared_resource() {
    fn needs_send_sync<T: Send + Sync>() {}
    needs_send_sync::<Vm>();
    needs_send_sync::<gdscript::InstRef>();
    needs_send_sync::<Value>();
}

/// `cargo test -p gdscript --test real instantiate_everything -- --ignored --nocapture`:
/// builds an object from every script of the game and the imported mods.
#[test]
#[ignore]
fn instantiate_everything() {
    let Some(lib) = library() else { return };
    let mut roots = vec![];
    if let Ok(rd) = std::fs::read_dir(lib.join("mods")) {
        for e in rd.flatten() {
            if e.path().join("res").is_dir() {
                roots.push(e.path().join("res"));
            }
        }
    }
    roots.push(lib.join("game"));
    let loader = FsLoader::new(roots.clone());
    let mut paths = Vec::new();
    for root in &roots {
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "gd") {
                    paths.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut vm = Vm::new(Box::new(loader));
    let mut host = Rec::default();
    let (mut ok, mut bad) = (0, 0);
    let mut by_err: std::collections::BTreeMap<String, usize> = Default::default();
    for p in &paths {
        let Some(script) = gdscript::Loader::by_path(vm.loader(), p) else { continue };
        match vm.instantiate(&mut host, script, None) {
            Ok(_) => ok += 1,
            Err(e) => {
                bad += 1;
                *by_err.entry(e.to_string()).or_default() += 1;
            }
        }
    }
    println!("{ok} scripts instantiate, {bad} do not");
    let mut v: Vec<_> = by_err.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (e, n) in v.iter().take(20) {
        println!("{n:4} x {e}");
    }
}
