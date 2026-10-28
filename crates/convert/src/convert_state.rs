// Included by convert.rs: converting states, projectiles and assembling the outputs.

struct Entry {
    state: String,
    id: String,
    origin: String,
    behavior: Option<String>,
    behavior_reasons: Vec<String>,
    losses: Vec<String>,
    notes: Vec<String>,
    dropped_turn: Vec<String>,
    engine_rules: Vec<String>,
    script_params: Vec<String>,
    presentation_dropped: Vec<String>,
    input: bool,
    no_input_reason: Option<String>,
    hitboxes: usize,
    script_lines: usize,
    params: Vec<String>,
    unbound_params: Vec<String>,
    throw: bool,
    spawn_projectiles: Vec<String>,
    partial: bool,
}

impl Entry {
    fn data(&self) -> &'static str {
        if self.partial { "partial" } else { "complete" }
    }

    fn to_json(&self) -> Value {
        json!({
            "state": self.state, "id": self.id, "origin": self.origin,
            "data": self.data(),
            "needs_behavior": self.behavior.is_some(),
            "behavior": self.behavior,
            "behavior_reasons": self.behavior_reasons,
            "losses": self.losses, "notes": self.notes,
            "dropped_turn_based": self.dropped_turn, "engine_rules": self.engine_rules,
            "script_params": self.script_params, "presentation_dropped": self.presentation_dropped,
            "input": if self.input { json!("placeholder") } else { Value::Null },
            "no_input_reason": self.no_input_reason,
            "hitboxes": self.hitboxes, "script_lines": self.script_lines,
            "params": self.params, "unbound_params": self.unbound_params,
            "throw": self.throw,
            "spawn_projectiles": self.spawn_projectiles,
        })
    }
}

impl Conv<'_> {
    fn convert_state(&mut self, s: &Value) -> (R, Entry) {
        let k = at(s, "key");
        let ex: Map<String, Value> = at(s, "explicit_overrides").as_object().cloned().unwrap_or_default();
        let sname = at(s, "name").as_str().unwrap_or("").to_string();
        let (mut losses, mut notes): (Vec<String>, Vec<String>) = (vec![], vec![]);
        let sa = self.analyze_state_scripts(s);
        let sid_ = sid(&sname);
        let all_boxes: Vec<Value> = at(s, "hitboxes").as_array().cloned().unwrap_or_default();
        let real: Vec<&Value> = all_boxes.iter().filter(|h| at(h, "kind").as_str() == Some("hitbox")).collect();
        let endless = truthy(k.get("endless"));
        let anim_len = if is_int(k.get("anim_length")) { to_int(k.get("anim_length")) } else { 1 };

        // --- timeline (computed from clamped windows)
        let wins: Vec<(i64, i64)> = all_boxes
            .iter()
            .map(|h| {
                let hk = at(h, "key");
                let st = to_int(Some(hk.get("start_tick").unwrap_or(&json!(0))));
                let ac = to_int(Some(hk.get("active_ticks").unwrap_or(&json!(0))));
                let wf = 1.max(st + 1);
                (wf, wf.max(st + ac))
            })
            .collect();
        let (startup, active) = if wins.is_empty() {
            (0, 0)
        } else {
            let st = wins.iter().map(|w| w.0).min().unwrap_or(1) - 1;
            (st, wins.iter().map(|w| w.1).max().unwrap_or(0) - st)
        };
        let mut end_kind = "Fixed";
        let recovery;
        if endless {
            recovery = 0;
            end_kind = if matches!(string(k.get("air_type")), Some("Aerial" | "Both")) && sa.grounded_check { "OnLanding" } else { "Script" };
        } else {
            recovery = 0.max(anim_len - startup - active);
            if startup + active > anim_len {
                losses.push(format!("hitboxes extend past anim_length ({anim_len}); recovery set to 0"));
            }
        }
        let mut tl = vec![("startup", R::Int(startup)), ("active", R::Int(active)), ("recovery", R::Int(recovery))];
        if end_kind != "Fixed" {
            tl.push(("end", R::s(end_kind)));
        }
        let listed = startup + active + recovery;

        // --- hitboxes
        let mut id_set = HashSet::new();
        self.hit_unmapped.clear();
        self.clamped.clear();
        let mut boxes = Vec::new();
        for h in &all_boxes {
            boxes.push(self.convert_hitbox(h, Some(listed), endless, &mut losses, all_boxes.len(), &mut id_set));
        }
        if !self.clamped.is_empty() {
            losses.push(format!("{} hitbox(es) have a start_tick set by script (scene value is negative); windows clamped to frame 1", self.clamped.len()));
        }
        if !self.hit_unmapped.is_empty() {
            let keys: Vec<String> = self.hit_unmapped.iter().cloned().collect();
            losses.push(format!("hitbox properties not expressed: {}", keys.join(", ")));
            for key in keys {
                self.note_prop("hitbox", &key, "gap", &sname);
            }
        }

        // --- hurt windows
        let mut hurt = Vec::new();
        for hs in at(s, "hurtbox_states").as_array().cloned().unwrap_or_default() {
            let p = at(&hs, "props");
            let st = to_int(Some(p.get("start_tick").unwrap_or(&json!(1))));
            let atk = to_int(Some(p.get("active_ticks").unwrap_or(&json!(1))));
            let frm = st + 1;
            let to = if truthy(p.get("endless")) { if listed != 0 { listed } else { frm } } else { st + atk };
            let to = to.max(frm);
            let dim = |key: &str, d: i64| R::Int(if p.get(key).is_some() { to_int(p.get(key)).abs() } else { d });
            hurt.push(R::st(
                "HurtWindow",
                vec![
                    ("window", span(frm, to)),
                    ("rect", R::st("Rect", vec![("x", R::Int(to_int(p.get("x")))), ("y", R::Int(to_int(p.get("y")))), ("half_w", dim("width", 20)), ("half_h", dim("height", 20))])),
                ],
            ));
        }

        // --- physics
        // a scripted move's own script applies what it calls, live
        let from_script = |name: &str| !self.scripted && sa.physics.contains(name);
        let friction = truthy(k.get("apply_fric")) || from_script("fric");
        let gravity = truthy(k.get("apply_grav")) || from_script("grav");
        let forces = truthy(k.get("apply_forces")) || from_script("forces") || from_script("forces_no_limit");
        let od = self.script_defaults("ObjectState.gd");
        let ov = |key: &str| ex.get(key).or_else(|| od.get(key));
        let mut ph: Vec<(&str, R)> = vec![];
        for (n, on) in [("friction", friction), ("gravity", gravity), ("forces", forces)] {
            if on {
                ph.push((n, R::Bool(true)));
            }
        }
        if truthy(ex.get("apply_forces_no_limit")) {
            ph.push(("uncapped_forces", R::Bool(true)));
        }
        if truthy(ex.get("apply_custom_grav")) {
            ph.push(("gravity_override", R::st("GravityOverride", vec![("gravity", f(num(ov("custom_grav"), 0.0))), ("fall_speed", f(num(ov("custom_grav_max_fall_speed"), 0.0)))])));
        }
        if truthy(ex.get("apply_custom_limits")) {
            ph.push(("speed_caps", R::st("SpeedCaps", vec![("ground", f(num(ov("custom_max_ground_speed"), 0.0))), ("air", f(num(ov("custom_max_air_speed"), 0.0)))])));
        }
        if truthy(ex.get("apply_custom_x_fric")) {
            ph.push(("friction_x", f(num(ov("custom_x_fric"), 0.0))));
        }
        if truthy(ex.get("apply_custom_y_fric")) {
            ph.push(("friction_y", f(num(ov("custom_y_fric"), 0.0))));
        }
        let physics = if ph.is_empty() { None } else { Some(R::st("Physics", ph)) };

        // --- motion
        let mut motion = Vec::new();
        let event = |at_: i64, kind: R| R::st("MotionEvent", vec![("at", R::Int(at_)), ("kind", kind)]);
        if truthy(ex.get("reset_momentum")) {
            motion.push(event(1, R::s("StopMomentum")));
        } else {
            if truthy(ex.get("reset_x_momentum")) {
                motion.push(event(1, R::s("StopHorizontal")));
            }
            if truthy(ex.get("reset_y_momentum")) {
                motion.push(event(1, R::s("StopVertical")));
            }
        }
        let impulse = |dx: f64, dy: f64, spd: f64| -> Option<R> {
            let ln = (dx * dx + dy * dy).powf(0.5);
            (ln != 0.0).then(|| R::st("Impulse", vec![("forward", f(round_to(dx / ln * spd, 4))), ("up", f(round_to(-dy / ln * spd, 4)))]))
        };
        let spd = num(k.get("enter_force_speed"), 0.0);
        if spd != 0.0 {
            match impulse(num(ex.get("enter_force_dir_x"), 0.0), num(ex.get("enter_force_dir_y"), 0.0), spd) {
                Some(i) => motion.push(event(1, i)),
                None => losses.push("enter force has speed but no direction".into()),
            }
        }
        let fspd = num(k.get("force_speed"), 0.0);
        if fspd != 0.0 {
            match impulse(num(ex.get("force_dir_x"), 0.0), num(ex.get("force_dir_y"), 0.0), fspd) {
                Some(i) => motion.push(event(to_int(Some(k.get("force_tick").unwrap_or(&json!(0)))) + 1, i)),
                None => losses.push("timed force has speed but no direction".into()),
            }
        }

        // --- projectile spawn from the scene
        let mut spawns: Vec<(String, R)> = Vec::new();
        let mut script_spawned: Vec<String> = Vec::new();
        if let Some(ps) = string(k.get("projectile_scene")).filter(|p| !p.is_empty()) {
            let file = ps.rsplit('/').next().unwrap_or(ps);
            let pid = snake(file.rsplit_once('.').map(|x| x.0).unwrap_or(file));
            let tick = to_int(Some(k.get("projectile_tick").unwrap_or(&json!(1))));
            if tick < 0 {
                script_spawned.push(pid.clone());
                losses.push(format!("projectile {pid} is spawned by script at a time the data does not hold"));
            } else {
                let off = R::tup2(R::Int(to_int(Some(k.get("projectile_pos_x").unwrap_or(&json!(0))))), R::Int(to_int(Some(k.get("projectile_pos_y").unwrap_or(&json!(0))))));
                spawns.push((pid.clone(), R::st("Spawn", vec![("at", R::Int(tick + 1)), ("projectile", R::Str(pid)), ("offset", off)])));
            }
        }

        // projectiles a script spawns from an exported scene (`spawn_object(projectile, projectile_x, ...)`)
        if spawns.is_empty() && !self.scripted {
            if let Some(res) = ex.get("projectile").and_then(|v| v.get("res")).and_then(Value::as_str) {
                let script = string(s.get("script")).unwrap_or("").to_string();
                let text = self.read(&script);
                if let Some(frame) = spawn_frame(&text) {
                    let defaults = self.script_defaults(&script);
                    let file = res.rsplit('/').next().unwrap_or(res);
                    let pid = snake(file.rsplit_once('.').map(|x| x.0).unwrap_or(file));
                    let off = |key: &str| to_int(ex.get(key).or_else(|| defaults.get(key)));
                    let offset = R::tup2(R::Int(off("projectile_x")), R::Int(off("projectile_y")));
                    spawns.push((pid.clone(), R::st("Spawn", vec![("at", R::Int(frame + 1)), ("projectile", R::Str(pid)), ("offset", offset)])));
                    notes.push("projectile spawn read from the script's frame hook; its aim and speed modifiers are not applied".into());
                }
            }
        }

        // --- cancels
        let mut cancels = Vec::new();
        let into_tags = self.tags_of(k.get("interrupt_into").unwrap_or(&NULL));
        let cancel = |into: Vec<String>, when: &str, from: i64, to: i64| R::st("Cancel", vec![("into", R::strs(into)), ("when", R::s(when)), ("window", span(from, to))]);
        if !endless && !into_tags.is_empty() {
            let iasa = k.get("iasa_at").cloned().unwrap_or(json!(-1));
            let iasa_tick = if is_int(Some(&iasa)) && num(Some(&iasa), 0.0) < 0.0 { Some(anim_len + to_int(Some(&iasa))) } else if is_int(Some(&iasa)) { Some(to_int(Some(&iasa))) } else { None };
            if let Some(t) = iasa_tick {
                if t + 1 < listed {
                    cancels.push(cancel(into_tags.clone(), "Always", t + 1, listed));
                }
            }
            for t in k.get("interrupt_frames").and_then(Value::as_array).cloned().unwrap_or_default() {
                if is_int(Some(&t)) && to_int(Some(&t)) + 1 <= listed {
                    cancels.push(cancel(into_tags.clone(), "Always", to_int(Some(&t)) + 1, to_int(Some(&t)) + 1));
                }
            }
        }
        let hit_into = self.tags_of(k.get("hit_cancel_into").unwrap_or(&NULL));
        if endless && (!hit_into.is_empty() || !into_tags.is_empty()) && !real.is_empty() {
            losses.push("cancel windows end at the last listed frame; the format cannot say 'until the move ends' for cancels".into());
        }
        let flag = |h: &Value, key: &str| at(h, "key").get(key).map(|v| truthy(Some(v))).unwrap_or(true);
        let cancellable = real.iter().any(|h| flag(h, "cancellable"));
        let when_hit = if real.iter().any(|h| flag(h, "block_cancel_allowed")) { "OnHitOrBlock" } else { "OnHit" };
        if !hit_into.is_empty() && !real.is_empty() && cancellable {
            let first_active = startup + 1;
            let last = if listed != 0 { listed } else { first_active };
            cancels.push(cancel(hit_into, when_hit, first_active, last));
            cancels.push(cancel(
                ["offensive_burst", "instant_cancel", "grounded_defense", "aerial_defense", "grounded_grab", "aerial_grab"].iter().map(|s| s.to_string()).collect(),
                when_hit,
                first_active,
                last,
            ));
        }

        // --- input, availability, cost
        let (inp, why_no_input) = self.guess_input(s);
        let mut avail = vec![];
        if truthy(k.get("combo_only")) {
            avail.push(("combo_only", R::Bool(true)));
        }
        if truthy(k.get("neutral_only")) {
            avail.push(("neutral_only", R::Bool(true)));
        }
        let mut cost = vec![];
        if truthy(k.get("super_level_")) {
            cost.push(("super_bars", json_to_r(k.get("super_level_"))));
        }
        if truthy(k.get("uses_air_movement")) {
            cost.push(("air_action", R::Bool(true)));
        }

        // --- player parameters and throw choreography
        let (params, params_dropped, pnotes, datas) = self.build_params(s);
        notes.extend(pnotes);
        if !params_dropped.is_empty() {
            losses.push(format!("parameter widgets with no mapping: {}", params_dropped.join(", ")));
        }
        let pname = |p: &R| match p.get("name") {
            Some(R::Str(n)) => n.clone(),
            _ => String::new(),
        };
        let unbound: Vec<String> = params.iter().filter(|p| p.get("binding").is_none()).map(pname).collect();
        let throw = self.build_throw(s, k, &ex, anim_len, &mut losses, &mut notes);

        // --- script-derived losses and behavior
        let mut behavior: Option<String> = None;
        let mut reasons: Vec<String> = Vec::new();
        if sa.unknown > 0 {
            let calls: Vec<&str> = sa.calls.iter().take(8).map(String::as_str).collect();
            reasons.push(format!("{} script statements ({} lines); calls {}", sa.unknown, sa.loc, if calls.is_empty() { "none".to_string() } else { calls.join(", ") }));
        }
        if !sa.lib.is_empty() {
            reasons.push(format!("library behavior {}", sa.lib.iter().map(|p| lib_stem(p)).collect::<Vec<_>>().join(", ")));
        }
        if sa.spawns && spawns.is_empty() {
            reasons.push("spawns objects from script".into());
        }
        if sa.invuln {
            reasons.push("invulnerability set from script".into());
        }
        if endless {
            reasons.push(if end_kind == "Script" { "end condition is script-driven" } else { "ends on landing (verify)" }.into());
        }
        if sa.sets_hitbox {
            reasons.push("script edits hitbox data".into());
        }
        let needs_behavior = sa.unknown > 0 || (sa.spawns && spawns.is_empty()) || sa.invuln || !sa.lib.is_empty() || end_kind == "Script" || sa.sets_hitbox || !script_spawned.is_empty();
        if !script_spawned.is_empty() {
            reasons.push(format!("spawns {} from script", script_spawned.join(", ")));
        }
        let own_name = format!("{}.{sid_}", self.char_id);
        if needs_behavior {
            behavior = Some(if !sa.own.is_empty() && sa.unknown > 0 {
                own_name.clone()
            } else if !sa.lib.is_empty() {
                format!("lib.{}", snake(&lib_stem(&sa.lib[0])))
            } else {
                own_name.clone()
            });
        }
        if sa.sets_hitbox {
            losses.push("hitbox fields are edited by script; the converted values are the scene defaults".into());
        }
        if sa.sets_length {
            losses.push("anim_length is set by script; the converted timeline uses the scene value".into());
        }
        if sa.sets_iasa {
            losses.push("cancel timing (iasa) is set by script; the converted cancels use the scene value".into());
        }
        if !sa.frame_hooks.is_empty() {
            let hooks: Vec<String> = sa.frame_hooks.iter().take(10).map(|h| h.to_string()).collect();
            notes.push(format!("per-tick hooks at [{}]", hooks.join(", ")));
        }
        let mut by_class: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        let keys: Vec<String> = clean_keys(&ex)
            .into_iter()
            .filter(|key| !STATE_CONSUMED.contains(&key.as_str()) && !STATE_PRESENTATION.contains(&key.as_str()) && !(throw.is_some() && THROW_CONSUMED.contains(&key.as_str())))
            .collect();
        for key in keys {
            let cls = state_class(&key).unwrap_or(if sa.exports.contains(&key) { "script" } else { "unknown" });
            by_class.entry(cls).or_default().push(key.clone());
            self.note_prop("state", &key, cls, &sname);
        }
        let bc = |c: &str| by_class.get(c).cloned().unwrap_or_default();
        if !bc("gap").is_empty() {
            losses.push(format!("data the format cannot hold: {}", bc("gap").join(", ")));
        }
        if !bc("unknown").is_empty() {
            losses.push(format!("unrecognised scene properties: {}", bc("unknown").join(", ")));
        }
        if !bc("script").is_empty() && behavior.is_none() {
            behavior = Some(own_name.clone());
            reasons.push(format!("scene parameters for script code: {}", bc("script").join(", ")));
        }
        let others: Vec<String> = at(s, "other_children").as_array().map(|a| a.iter().filter_map(|c| at(c, "name").as_str().map(str::to_string)).collect()).unwrap_or_default();
        if !others.is_empty() {
            losses.push(format!("extra child nodes not converted: {}", others.join(", ")));
        }
        let hc = at(s, "host_command_nodes").as_array().map(Vec::len).unwrap_or(0);
        if hc > 0 {
            losses.push(format!("{hc} host-command nodes not converted"));
        }
        let pres_dropped: Vec<String> = clean_keys(&ex).into_iter().filter(|k| STATE_PRESENTATION.contains(&k.as_str()) && k != "button_texture" && k != "flip_icon").collect();
        if !pres_dropped.is_empty() {
            notes.push(format!("presentation not converted: {}", pres_dropped.join(", ")));
        }

        let allowed_src = string(k.get("allowed_stances_string")).filter(|s| !s.is_empty()).unwrap_or("Normal");
        let allowed: Vec<String> = allowed_src.split('\n').filter(|x| !x.trim().is_empty()).map(snake).collect();
        let modes_for_move: Vec<String> = if allowed.is_empty() || allowed.iter().any(|a| a == "all") { vec![] } else { allowed };
        let set_mode = string(ex.get("change_stance_to")).filter(|s| !s.is_empty()).map(snake);
        self.mode_use.extend(modes_for_move.iter().cloned());
        if let Some(m) = &set_mode {
            self.mode_use.insert(m.clone());
        }

        let title = string(k.get("title")).filter(|s| !s.is_empty()).unwrap_or(&sname).to_string();
        let stance = match string(k.get("air_type")) {
            Some("Grounded") => "Ground",
            Some("Aerial") => "Air",
            _ => "Any",
        };
        let mut fields: Vec<(&str, R)> = vec![
            ("id", R::Str(sid_.clone())),
            ("name", R::Str(title)),
            ("tags", R::strs(self.tags_of(k.get("interrupt_from").unwrap_or(&NULL)))),
            ("stance", R::s(stance)),
        ];
        if !modes_for_move.is_empty() {
            fields.push(("modes", R::strs(modes_for_move)));
        }
        if let Some(m) = set_mode {
            fields.push(("set_mode", R::Str(m)));
        }
        if let Some(i) = inp.clone() {
            fields.push(("input", i));
        }
        if !params.is_empty() {
            fields.push(("params", R::List(params.clone())));
        }
        if !avail.is_empty() {
            fields.push(("availability", R::st("Availability", avail)));
        }
        fields.push(("timeline", R::st("Timeline", tl)));
        let box_count = boxes.len();
        if !boxes.is_empty() {
            fields.push(("hitboxes", R::List(boxes)));
        }
        let has_throw = throw.is_some();
        if let Some(t) = throw {
            fields.push(("throw", t));
        }
        if !hurt.is_empty() {
            fields.push(("hurt", R::List(hurt)));
        }
        if let Some(p) = physics {
            fields.push(("physics", p));
        }
        if !motion.is_empty() {
            fields.push(("motion", R::List(motion)));
        }
        if !spawns.is_empty() {
            fields.push(("spawns", R::List(spawns.iter().map(|(_, sp)| sp.clone()).collect())));
        }
        if !cancels.is_empty() {
            fields.push(("cancels", R::List(cancels)));
        }
        let fallback = string(k.get("fallback_state")).filter(|s| !s.is_empty()).unwrap_or("Wait");
        let fb = sid(fallback);
        if fb != "idle" {
            fields.push(("finish_into", R::Str(fb.clone())));
        }
        if !self.conv_ids.contains(&fb) && !self.lib_ids.contains(&fb) && fb != "idle" {
            losses.push(format!("finish state {fallback} is neither converted nor in the base library"));
        }
        if !cost.is_empty() {
            fields.push(("cost", R::st("Cost", cost)));
        }
        if self.scripted {
            behavior = None;
            if let Some(path) = string(s.get("script")).filter(|p| !p.is_empty()) {
                let mut sc = state_script(path, &sname, &ex);
                let (data, stick) = script_data(&datas);
                if let Some(d) = data {
                    sc.put("data", d);
                }
                if let Some(st) = stick {
                    sc.put("stick", R::Some(Box::new(st)));
                }
                fields.push(("script", sc));
            }
        }
        if let Some(b) = &behavior {
            fields.push(("behavior", R::Str(b.clone())));
        }
        let anim = string(k.get("sprite_animation")).filter(|s| !s.is_empty()).unwrap_or(&sname);
        fields.push(("presentation", R::st("Presentation", vec![("anim", R::Str(snake(anim)))])));
        let mv = R::st("Move", fields);

        let entry = Entry {
            state: sname,
            id: sid_,
            origin: at(s, "origin").as_str().unwrap_or("").to_string(),
            behavior,
            behavior_reasons: reasons,
            partial: !losses.is_empty(),
            losses,
            notes,
            dropped_turn: bc("turn"),
            engine_rules: bc("engine"),
            script_params: bc("script"),
            presentation_dropped: bc("presentation"),
            input: inp.is_some(),
            no_input_reason: why_no_input,
            hitboxes: box_count,
            script_lines: sa.loc,
            params: params.iter().map(pname).collect(),
            unbound_params: unbound,
            throw: has_throw,
            spawn_projectiles: spawns.iter().map(|(p, _)| p.clone()).collect(),
        };
        (mv, entry)
    }

    fn convert_projectiles(&mut self) -> (Vec<R>, Vec<Value>) {
        let (mut out, mut rep) = (Vec::new(), Vec::new());
        let projectiles = at(self.data, "projectiles").as_array().cloned().unwrap_or_default();
        for p in &projectiles {
            let scene = at(p, "scene").as_str().unwrap_or("");
            let file = scene.rsplit('/').next().unwrap_or(scene);
            let pid = snake(file.rsplit_once('.').map(|x| x.0).unwrap_or(file));
            let rs = string(p.get("root_script")).unwrap_or("").to_string();
            if p.get("error").is_some() {
                rep.push(json!({"id": pid, "status": "parse error"}));
                continue;
            }
            if rs.ends_with("ParticleEffect.gd") {
                rep.push(json!({"id": pid, "status": "visual effect only, skipped"}));
                continue;
            }
            let states_raw = at(p, "states").as_array().cloned().unwrap_or_default();
            if self.scripted && states_raw.is_empty() && !scene.contains("/projectiles/") {
                // a scene the scripts refer to that is not an object (a UI piece, an effect)
                continue;
            }
            let pid = {
                let mut id = pid;
                let base = id.clone();
                let mut k = 2;
                while out.iter().any(|o: &R| matches!(o.get("id"), Some(R::Str(x)) if *x == id)) {
                    id = format!("{base}_{k}");
                    k += 1;
                }
                id
            };
            let mut losses: Vec<String> = Vec::new();
            let rprops = at(p, "root_properties");
            let grav = rprops.get("gravity").filter(|g| !g.is_null());
            if grav.is_none() {
                losses.push("gravity not set in scene (script/engine default applies)".into());
            }
            let states = at(p, "states").as_array().cloned().unwrap_or_default();
            let with_boxes: Vec<&Value> = states.iter().filter(|st| at(st, "hitboxes").as_array().is_some_and(|b| !b.is_empty())).collect();
            let main: Option<&Value> = states.iter().find(|st| at(st, "name").as_str() == Some("Default") && at(st, "hitboxes").as_array().is_some_and(|b| !b.is_empty())).or_else(|| with_boxes.first().copied());
            let other: Vec<&str> = with_boxes.iter().filter(|st| !main.is_some_and(|m| std::ptr::eq(m, **st))).filter_map(|st| at(st, "name").as_str()).collect();
            if !other.is_empty() {
                losses.push(format!("hitboxes in other projectile states not expressed: {}", other.join(", ")));
            }
            if states.len() > 1 {
                losses.push(format!("projectile has multiple states: {}", states.iter().filter_map(|st| at(st, "name").as_str()).collect::<Vec<_>>().join(", ")));
            }
            let mut lifetime = 0;
            let first_explicit = states.first().map(|st| at(st, "explicit"));
            match first_explicit {
                Some(e) if is_int(e.get("anim_length")) && !truthy(e.get("endless")) => lifetime = to_int(e.get("anim_length")),
                Some(e) if is_int(e.get("lifetime")) && (1..5000).contains(&to_int(e.get("lifetime"))) => lifetime = to_int(e.get("lifetime")),
                _ => losses.push("lifetime is not in the scene (script or endless state)".into()),
            }
            // straight-line travel from the default state's `move_x` / `move_speed`
            let attacks = main.is_some();
            let velocity = match first_explicit {
                _ if !attacks => None,
                Some(e) if e.get("move_speed").is_some() => Some((num(e.get("move_speed"), 0.0), 0.0)),
                Some(e) if e.get("move_x").is_some() || e.get("move_y").is_some() => Some((num(e.get("move_x"), 0.0), num(e.get("move_y"), 0.0))),
                _ => None,
            };
            if velocity.is_none() {
                losses.push("initial velocity is set by the spawning move or the script".into());
            }
            let (vx, vy) = velocity.unwrap_or((0.0, 0.0));
            let mut boxes = Vec::new();
            let mut ids = HashSet::new();
            if let Some(m) = main {
                let raw = at(m, "hitboxes").as_array().cloned().unwrap_or_default();
                for h in &raw {
                    boxes.push(self.convert_hitbox(h, None, true, &mut losses, raw.len(), &mut ids));
                }
            }
            let mut fields: Vec<(&str, R)> = vec![("id", R::Str(pid.clone())), ("velocity", R::tup2(f(vx), f(vy)))];
            if let Some(g) = grav {
                fields.push(("gravity", f(num(Some(g), 0.0))));
            }
            fields.push(("lifetime", R::Int(lifetime)));
            let nboxes = boxes.len();
            fields.push(("hitboxes", R::List(boxes)));
            let mut behavior = None;
            if self.scripted && !states_raw.is_empty() {
                // the object's own scripts run in the interpreter: its scene, script and states
                let rp = at(p, "root_properties").as_object().cloned().unwrap_or_default();
                let root_script = if rs.is_empty() { "projectile/BaseProjectile.gd" } else { rs.as_str() };
                fields.push(("scene", R::Some(Box::new(R::str(scene)))));
                fields.push(("script", R::Some(Box::new(state_script(root_script, ".", &rp)))));
                let mut sts = Vec::new();
                for st in &states_raw {
                    let name = at(st, "name").as_str().unwrap_or("");
                    let ex: Map<String, Value> = at(st, "explicit").as_object().cloned().unwrap_or_default();
                    let script = string(st.get("script")).filter(|x| !x.is_empty()).unwrap_or("ObjectState.gd").to_string();
                    let raw = at(st, "hitboxes").as_array().cloned().unwrap_or_default();
                    let mut sids = HashSet::new();
                    let mut sl = Vec::new();
                    let boxes: Vec<R> = raw.iter().map(|h| self.convert_hitbox(h, None, true, &mut sl, raw.len(), &mut sids)).collect();
                    let mut f2 = vec![("node", R::str(name)), ("script", state_script(&script, name, &ex))];
                    if !boxes.is_empty() {
                        f2.push(("hitboxes", R::List(boxes)));
                    }
                    sts.push(R::st("ObjectState", f2));
                }
                fields.push(("states", R::List(sts)));
                out.push(R::st("Projectile", fields));
                rep.push(json!({"id": pid, "status": "scripted", "states": states_raw.len()}));
                continue;
            }
            if !rs.is_empty() && !rs.ends_with("BaseProjectile.gd") {
                let b = format!("{}.{pid}", self.char_id);
                fields.push(("behavior", R::Str(b.clone())));
                behavior = Some(b);
                let loc = self.analyze_script(&rs).loc;
                let base = rs.rsplit('/').next().unwrap_or(&rs);
                losses.push(format!("custom script {base} ({loc} lines)"));
            }
            out.push(R::st("Projectile", fields));
            rep.push(json!({"id": pid, "status": if losses.is_empty() { "converted" } else { "partial" }, "losses": losses, "hitboxes": nboxes, "behavior": behavior}));
        }
        (out, rep)
    }
}

/// The frame whose `_frame_N` hook spawns the exported `projectile`.
fn spawn_frame(text: &str) -> Option<i64> {
    let hooks = lazy_re!(r"(?m)^func _frame_(\d+)\(\):");
    let starts: Vec<(usize, i64)> = hooks.captures_iter(text).map(|c| (c.get(0).expect("match").start(), c[1].parse().unwrap_or(0))).collect();
    for (i, (start, n)) in starts.iter().enumerate() {
        let rest = &text[*start..];
        let end = lazy_re!(r"(?m)^func ").find_at(rest, 1).map(|m| m.start()).unwrap_or(rest.len());
        if rest[..end].contains("spawn_object(projectile") {
            let _ = i;
            return Some(*n);
        }
    }
    None
}

/// What a conversion produced, ready to write to disk.
pub struct Converted {
    pub ron: String,
    pub fx: String,
    pub report_json: Value,
    pub report_md: String,
    pub moves: usize,
    pub projectiles: usize,
    pub summary: Value,
}

const ASSUMPTIONS: [&str; 9] = [
    "Frames are 1-based. A hitbox with start_tick s becomes window s+1..s+active; startup = first start_tick. The engine's exact off-by-one is unconfirmed.",
    "Recovery runs to the original anim_length; cancels from iasa_at / interrupt_frames open on frame index+1 and last until the move ends.",
    "block_advantage is the original plus_frames; blockstun must be tuned by playtesting.",
    "Tags are the original interrupt_from categories (snake_case); a cancel's `into` matches any tag.",
    "Hit cancels open at the first active frame (the original opens them after hitlag) and include the engine-wide extras (burst, instant cancel, defense, grab).",
    "Physics flags come from scene flags plus recognised calls in _tick; every other script statement counts as unexpressed logic (see behavior).",
    "Input bindings are placeholders derived from the menu type and the move name. Many moves collide; see the report.",
    "Enter/timed forces are direction-normalised and rotated into facing space; y is flipped to 'up'.",
    "Units (speeds, forces, gravity) are copied unchanged, in the original's per-tick units.",
];

/// Converts an extracted character (the JSON from `extract`) into the project's files.
pub fn convert_character(game: &std::path::Path, data: &Value, char_id: &str) -> Converted {
    convert_character_with(game, data, char_id, false)
}

/// A script variable's scene value, if it is plain data (not a resource or an expression).
fn script_var(v: &Value) -> Option<R> {
    Some(match v {
        Value::Bool(b) => R::Call("Bool".into(), vec![R::Bool(*b)]),
        Value::Number(n) => match n.as_i64() {
            Some(i) => R::Call("Int".into(), vec![R::Int(i)]),
            None => R::Call("Float".into(), vec![R::Float(n.as_f64()?)]),
        },
        Value::String(t) => R::Call("Str".into(), vec![R::Str(t.clone())]),
        Value::Array(items) => R::Call("List".into(), vec![R::List(items.iter().map(script_var).collect::<Option<Vec<_>>>()?)]),
        Value::Object(o) => {
            if let Some(xy) = o.get("Vector2").and_then(Value::as_array) {
                R::Call("Vec2".into(), vec![R::Float(xy.first()?.as_f64()?), R::Float(xy.get(1)?.as_f64()?)])
            } else if let Some(c) = o.get("Color").and_then(Value::as_array) {
                let n = |i: usize| c.get(i).and_then(Value::as_f64).unwrap_or(1.0);
                R::Call("Color".into(), vec![R::Float(n(0)), R::Float(n(1)), R::Float(n(2)), R::Float(n(3))])
            } else if let Some(r) = o.get("res").and_then(Value::as_str) {
                R::Call("Res".into(), vec![R::str(r)])
            } else {
                return None;
            }
        }
        Value::Null => return None,
    })
}

/// `SceneDef(path, nodes)` from the extractor's scene record.
fn scene_def_r(v: &Value) -> R {
    let nodes: Vec<R> = at(v, "nodes")
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| {
            let props: Map<String, Value> = at(n, "props").as_object().cloned().unwrap_or_default();
            let mut f = vec![("path", R::str(at(n, "path").as_str().unwrap_or(""))), ("ty", R::str(at(n, "ty").as_str().unwrap_or("")))];
            let mut keys: Vec<&String> = props.keys().filter(|k| !k.starts_with("_c_")).collect();
            keys.sort();
            let pv: Vec<R> = keys.into_iter().filter_map(|k| Some(R::tup2(R::Str(k.clone()), script_var(props.get(k)?)?))).collect();
            if !pv.is_empty() {
                f.push(("props", R::List(pv)));
            }
            let anims: Vec<R> = at(n, "anims")
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|a| {
                    R::st(
                        "AnimDef",
                        vec![
                            ("name", R::str(at(a, "name").as_str().unwrap_or(""))),
                            ("frames", R::Int(at(a, "frames").as_i64().unwrap_or(1))),
                            ("fps", R::Float(at(a, "fps").as_f64().unwrap_or(5.0))),
                            ("looped", R::Bool(at(a, "looped").as_bool().unwrap_or(false))),
                        ],
                    )
                })
                .collect();
            if !anims.is_empty() {
                f.push(("anims", R::List(anims)));
            }
            R::st("NodeDef", f)
        })
        .collect();
    R::st("SceneDef", vec![("path", R::str(at(v, "path").as_str().unwrap_or(""))), ("nodes", R::List(nodes))])
}

/// `StateScript(path, node, vars)` for a scene node and the values it sets.
fn state_script(path: &str, node: &str, ex: &Map<String, Value>) -> R {
    let vars: Vec<R> = clean_keys(ex).into_iter().filter_map(|k| Some(R::tup2(R::Str(k.clone()), script_var(ex.get(&k)?)?))).collect();
    let mut fields = vec![("path", R::str(path)), ("node", R::str(node))];
    if !vars.is_empty() {
        fields.push(("vars", R::List(vars)));
    }
    R::st("StateScript", fields)
}

/// `scripted`: the fighter's scripts will run in the interpreter (imported mods), so moves keep
/// only what their scenes say and carry their scripts; nothing is guessed from script code.
pub fn convert_character_with(game: &std::path::Path, data: &Value, char_id: &str, scripted: bool) -> Converted {
    let own: Vec<&Value> = at(data, "states").as_array().map(|a| a.iter().filter(|s| at(s, "origin").as_str() != Some("base")).collect()).unwrap_or_default();
    let lib_names: BTreeSet<String> = at(data, "states").as_array().map(|a| a.iter().filter(|s| at(s, "origin").as_str() == Some("base")).filter_map(|s| at(s, "name").as_str().map(str::to_string)).collect()).unwrap_or_default();
    let root = at(data, "character");
    let mut entered_by: HashMap<String, Vec<String>> = HashMap::new();
    for s in &own {
        let sname = at(s, "name").as_str().unwrap_or("").to_string();
        for h in at(s, "hitboxes").as_array().cloned().unwrap_or_default() {
            let hk = at(&h, "key");
            if let Some(fu) = string(hk.get("followup_state")).filter(|s| !s.is_empty()) {
                entered_by.entry(fu.to_string()).or_default().push(sname.clone());
            }
            if at(&h, "kind").as_str() == Some("throwbox") {
                if let Some(ts) = string(at(&h, "explicit").get("throw_state")).filter(|s| !s.is_empty()) {
                    entered_by.entry(ts.to_string()).or_default().push(sname.clone());
                }
            }
        }
    }
    let scene_rel = root["scene"].as_str().unwrap_or("").to_string();
    let scene_text = read_file(game, &scene_rel);
    let base_text = read_file(game, "characters/BaseChar.tscn");
    let mut anims = animations_in(&base_text);
    anims.extend(animations_in(&scene_text));
    let mut c = Conv {
        game,
        char_id: char_id.to_string(),
        src_name: format!("{char_id}.json"),
        data,
        conv_ids: own.iter().map(|s| sid(at(s, "name").as_str().unwrap_or(""))).collect(),
        lib_ids: lib_names.iter().map(|n| sid(n)).collect(),
        own,
        entered_by,
        rp: at(root, "root_properties").as_object().cloned().unwrap_or_default(),
        rd: at(root, "root_class_defaults").as_object().cloned().unwrap_or_default(),
        texts: HashMap::new(),
        analyses: HashMap::new(),
        defaults: HashMap::new(),
        scene_ext: ext_map(&scene_text).into_iter().collect(),
        anims,
        hit_unmapped: BTreeSet::new(),
        clamped: vec![],
        prop_hits: vec![],
        mode_use: BTreeSet::new(),
        scripted,
    };

    let mut moves: Vec<R> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    for s in c.own.clone() {
        let (mv, entry) = c.convert_state(s);
        moves.push(mv);
        entries.push(entry);
    }
    let (mut projectiles, prep) = c.convert_projectiles();
    crate::verified::apply(char_id, &mut moves, &mut projectiles);

    // spawn references must resolve
    let proj_ids: HashSet<String> = projectiles.iter().filter_map(|p| match p.get("id") {
        Some(R::Str(s)) => Some(s.clone()),
        _ => None,
    }).collect();
    for (mv, entry) in moves.iter_mut().zip(entries.iter_mut()) {
        let Some(R::List(sp)) = mv.get("spawns").cloned() else { continue };
        let target = |x: &R| match x.get("projectile") {
            Some(R::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let missing: Vec<String> = sp.iter().map(target).filter(|t| !proj_ids.contains(t)).collect();
        if !missing.is_empty() {
            let kept: Vec<R> = sp.into_iter().filter(|x| proj_ids.contains(&target(x))).collect();
            if kept.is_empty() {
                mv.remove("spawns");
            } else {
                mv.put("spawns", R::List(kept));
            }
            entry.losses.push(format!("projectile scene(s) not converted: {}", missing.join(", ")));
            entry.partial = true;
        }
    }

    // input collisions
    let mut groups: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for mv in &moves {
        if let Some(inp) = mv.get("input") {
            let button = inp.get("button").map(|b| b.name().to_string()).unwrap_or_default();
            let dir = inp.get("direction").map(|d| d.name().to_string()).unwrap_or_else(|| "Any".into());
            let stance = mv.get("stance").map(|s| s.name().to_string()).unwrap_or_default();
            let id = match mv.get("id") {
                Some(R::Str(s)) => s.clone(),
                _ => String::new(),
            };
            groups.entry((button, dir, stance)).or_default().push(id);
        }
    }
    let conflicts: Vec<Value> = groups.iter().filter(|(_, v)| v.len() > 1).map(|(k, v)| json!({"binding": format!("{} / {} / {}", k.0, k.1, k.2), "moves": v})).collect();

    // body
    let rootv = |c: &Conv, key: &str| -> Option<Value> {
        let v = c.rp.get(key).filter(|v| !v.is_null()).or_else(|| c.rd.get(key).filter(|v| !v.is_null()));
        v.cloned()
    };
    let n = |c: &Conv, k: &str, d: f64| f(num(rootv(c, k).as_ref(), d));
    let mut body = R::st(
        "Body",
        vec![
            ("max_health", R::Int(1500)),
            ("gravity", n(&c, "gravity", 0.0)),
            ("ground_friction", n(&c, "ground_friction", 0.0)),
            ("air_friction", n(&c, "air_friction", 0.0)),
            ("max_ground_speed", n(&c, "max_ground_speed", 0.0)),
            ("max_air_speed", n(&c, "max_air_speed", 0.0)),
            ("max_fall_speed", n(&c, "max_fall_speed", 0.0)),
            ("air_actions", R::Int(to_int(Some(&rootv(&c, "num_air_movements").unwrap_or(json!(0)))))),
            ("collision_box", c.scene_box("CollisionBox")),
            ("hurtbox", c.scene_box("Hurtbox")),
        ],
    );
    if num(rootv(&c, "damage_taken_modifier").as_ref(), 1.0) != 1.0 {
        body.put("damage_taken", n(&c, "damage_taken_modifier", 1.0));
    }
    if num(rootv(&c, "knockback_taken_modifier").as_ref(), 1.0) != 1.0 {
        body.put("knockback_taken", n(&c, "knockback_taken_modifier", 1.0));
    }
    let rscript = string(root.get("root_script")).filter(|s| !s.is_empty()).map(str::to_string);
    let mech = rscript.as_ref().filter(|r| c.analyze_script(r).loc > 30).map(|_| char_id.to_string());

    let cap = {
        let mut cs = char_id.chars();
        cs.next().map(|f| f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase()).unwrap_or_default()
    };
    let mut character = R::st("Character", vec![("id", R::str(char_id)), ("name", R::Str(cap)), ("body", body)]);
    if let Some(m) = mech.filter(|_| !scripted) {
        character.put("mechanic", R::Str(m));
    }
    if let Some(r) = rscript.as_ref().filter(|_| scripted) {
        let rp = c.rp.clone();
        character.put("script", state_script(r, root["name"].as_str().unwrap_or(""), &rp));
    }
    if scripted {
        character.put("scene", R::Some(Box::new(R::str(&scene_rel))));
        let defs: Vec<R> = at(data, "scene_defs").as_array().cloned().unwrap_or_default().iter().map(scene_def_r).collect();
        if !defs.is_empty() {
            character.put("scenes", R::List(defs));
        }
    }
    let extra_modes: Vec<String> = c.mode_use.iter().filter(|m| *m != "normal").cloned().collect();
    if !extra_modes.is_empty() {
        let mut modes = vec!["normal".to_string()];
        modes.extend(extra_modes);
        character.put("modes", R::strs(modes));
    } else {
        for mv in moves.iter_mut() {
            mv.remove("modes");
            mv.remove("set_mode");
        }
    }
    let move_count = moves.len();
    character.put("moves", R::List(moves));
    let proj_count = projectiles.len();
    if !projectiles.is_empty() {
        character.put("projectiles", R::List(projectiles));
    }
    character.put(
        "provenance",
        R::st(
            "Provenance",
            vec![
                ("source", R::Str(format!("{} ({}); {} character states, {} projectiles", c.src_name, scene_rel, c.own.len(), proj_count))),
                ("assumptions", R::strs(ASSUMPTIONS)),
                ("todo", R::strs(["Replace placeholder input bindings with a real control scheme.", "Write the behaviors named in `behavior` and the character mechanic.", "Resolve the losses listed in the conversion report, then verify frame counts in the running game."])),
            ],
        ),
    );
    let doc = R::st("CharacterFile", vec![("format_version", R::Int(1)), ("character", character)]);
    let header = format!(
        "#![enable(implicit_some)]\n// Converted by the yemi-convert crate from {}.\n// PRIVATE: contains numbers taken from the original game. Do not publish.\n",
        c.src_name
    );
    let ron = format!("{header}{}\n", emit(&doc, 0));

    // ------------------------------------------------------------------ report
    let total = entries.len();
    let complete: Vec<&Entry> = entries.iter().filter(|e| e.data() == "complete" && e.behavior.is_none()).collect();
    let beh_only: Vec<&Entry> = entries.iter().filter(|e| e.data() == "complete" && e.behavior.is_some()).collect();
    let partial: Vec<&Entry> = entries.iter().filter(|e| e.data() == "partial").collect();
    let mut loss_kinds: Vec<(String, Vec<String>)> = Vec::new();
    const PREFIXES: [&str; 6] = [
        "hitbox properties not expressed", "data the format cannot hold", "unrecognised scene properties",
        "parameter widgets with no mapping", "extra child nodes not converted", "projectile scene(s) not converted",
    ];
    for e in &entries {
        for l in &e.losses {
            let key = match PREFIXES.iter().find(|p| l.starts_with(*p)) {
                Some(p) => p.to_string(),
                None => {
                    let k = lazy_re!(r"projectile \w+ is spawned").replace_all(l, "projectile is spawned").into_owned();
                    let k = lazy_re!(r"hitbox \S+: ").replace_all(&k, "hitbox: ").into_owned();
                    lazy_re!(r"\d+").replace_all(&k, "N").into_owned()
                }
            };
            match loss_kinds.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(e.state.clone()),
                None => loss_kinds.push((key, vec![e.state.clone()])),
            }
        }
    }
    let base_names: Vec<&String> = lib_names.iter().collect();
    let with_behavior = entries.iter().filter(|e| e.behavior.is_some()).count();
    let script_lines: usize = entries.iter().filter(|e| e.behavior.is_some()).map(|e| e.script_lines).sum();
    let summary = json!({
        "states_converted": total, "complete": complete.len(), "complete_needs_behavior": beh_only.len(),
        "partial": partial.len(), "with_behavior": with_behavior,
        "script_lines_needing_port": script_lines,
        "base_states_not_converted": base_names.len(),
        "input_collisions": conflicts.len(),
        "moves_with_input": entries.iter().filter(|e| e.input).count(), "moves_without_input": entries.iter().filter(|e| !e.input).count(),
    });
    // properties, sorted by (class, -moves, name)
    let mut props: Vec<&((String, String, String), BTreeSet<String>)> = c.prop_hits.iter().collect();
    props.sort_by(|a, b| (&a.0 .2, std::cmp::Reverse(a.1.len()), &a.0 .1).cmp(&(&b.0 .2, std::cmp::Reverse(b.1.len()), &b.0 .1)));
    let properties: Vec<Value> = props
        .iter()
        .map(|((lv, p, cls), v)| json!({"property": p, "level": lv, "class": cls, "moves": v.iter().collect::<Vec<_>>(), "note": prop_note(p)}))
        .collect();
    let states_json: Vec<Value> = entries.iter().map(Entry::to_json).collect();
    let report_json = json!({
        "states": states_json,
        "input_conflicts": conflicts,
        "projectiles": prep,
        "base_states_not_converted": base_names,
        "summary": summary,
        "properties": properties,
    });

    let md = report_markdown(char_id, &c.src_name, &entries, &complete, &beh_only, &partial, &loss_kinds, &report_json, &prep, &conflicts, &base_names);
    let fx = fx_file(&c, char_id);
    Converted { ron, fx, report_json, report_md: md, moves: move_count, projectiles: proj_count, summary }
}

#[allow(clippy::too_many_arguments)]
fn report_markdown(
    char_id: &str,
    src_name: &str,
    st: &[Entry],
    complete: &[&Entry],
    beh_only: &[&Entry],
    partial: &[&Entry],
    loss_kinds: &[(String, Vec<String>)],
    report: &Value,
    prep: &[Value],
    conflicts: &[Value],
    base_names: &[&String],
) -> String {
    let summary = &report["summary"];
    let n = st.len();
    let cnt = |k: &str| summary[k].as_u64().unwrap_or(0);
    let converted = prep.iter().filter(|p| matches!(p["status"].as_str(), Some("converted" | "partial"))).count();
    let visual = prep.iter().filter(|p| p["status"].as_str().is_some_and(|s| s.starts_with("visual"))).count();
    let mut l: Vec<String> = vec![
        format!("# Conversion report: {char_id}"), String::new(),
        format!("Generated by the yemi-convert crate from `{src_name}`. PRIVATE (derived from game data)."), String::new(),
        "## Summary".into(), String::new(),
        format!("- States converted: **{n}** (the character's own and overriding states). {} inherited base states are not converted; they belong to the shared base library.", base_names.len()),
        format!("- **{}** convert completely as data (no script logic, nothing dropped).", complete.len()),
        format!("- **{}** convert as data but also need a named behavior (script logic or library behavior).", beh_only.len()),
        format!("- **{}** have data that could not be fully expressed (see their losses).", partial.len()),
        format!("- **{}** states need a behavior in total, covering about **{}** lines of original script.", cnt("with_behavior"), cnt("script_lines_needing_port")),
        format!("- Input: {} moves got a placeholder binding, {} have none (entered by other moves or the engine). {} bindings are shared by several moves.", cnt("moves_with_input"), cnt("moves_without_input"), conflicts.len()),
        format!("- Projectile scenes: {converted} converted, {visual} visual-only skipped."), String::new(),
        "## What could not be expressed, by kind".into(), String::new(), "| kind | moves | examples |".into(), "|---|---|---|".into(),
    ];
    let mut kinds: Vec<&(String, Vec<String>)> = loss_kinds.iter().collect();
    kinds.sort_by_key(|(_, names)| std::cmp::Reverse(names.iter().collect::<BTreeSet<_>>().len() * 0 + names.len()));
    for (kind, names) in kinds {
        let set: BTreeSet<&String> = names.iter().collect();
        let ex: Vec<&str> = set.iter().take(5).map(|s| s.as_str()).collect();
        l.push(format!("| {kind} | {} | {} |", set.len(), ex.join(", ")));
    }
    let title = |c: &str| match c {
        "gap" => "Gameplay data the format cannot hold yet (real losses)",
        "engine" => "Engine rules to re-decide in the new game",
        "turn" => "Turn-based concepts dropped by design",
        "script" => "Parameters that belong with behavior code",
        "unknown" => "Not recognised by the converter",
        _ => "Presentation (sound, shake, particles), not converted",
    };
    l.push(String::new());
    l.push("## Scene properties that were not converted, by class".into());
    l.push(String::new());
    let props = report["properties"].as_array().cloned().unwrap_or_default();
    for cls in ["gap", "engine", "turn", "script", "unknown", "presentation"] {
        let rows: Vec<&Value> = props.iter().filter(|p| p["class"].as_str() == Some(cls)).collect();
        if rows.is_empty() {
            continue;
        }
        l.push(format!("### {}", title(cls)));
        l.push(String::new());
        l.push("| property | on | moves | what it is | examples |".into());
        l.push("|---|---|---|---|---|".into());
        for p in rows {
            let moves: Vec<&str> = p["moves"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let note = p["note"].as_str().filter(|s| !s.is_empty()).unwrap_or("-");
            l.push(format!("| {} | {} | {} | {note} | {} |", p["property"].as_str().unwrap_or(""), p["level"].as_str().unwrap_or(""), moves.len(), moves.iter().take(4).copied().collect::<Vec<_>>().join(", ")));
        }
        l.push(String::new());
    }
    l.push(String::new());
    l.push("## Per state".into());
    l.push(String::new());
    l.push("| state | data | behavior | input | boxes | script lines | losses and notes |".into());
    l.push("|---|---|---|---|---|---|---|".into());
    for e in st {
        let mut parts: Vec<String> = e.losses.iter().chain(e.notes.iter()).cloned().collect();
        if e.no_input_reason.as_deref().is_some_and(|r| r.contains("entered")) {
            parts.push("entered by other moves".into());
        }
        let txt: String = parts.join("; ").replace('|', "/").chars().take(240).collect();
        l.push(format!(
            "| {} | {} | {} | {} | {} | {} | {txt} |",
            e.state, e.data(), e.behavior.as_deref().unwrap_or("-"), if e.input { "placeholder" } else { "-" }, e.hitboxes,
            if e.script_lines > 0 { e.script_lines.to_string() } else { "-".into() }
        ));
    }
    l.push(String::new());
    l.push("## Behavior reasons".into());
    l.push(String::new());
    for e in st {
        if !e.behavior_reasons.is_empty() {
            l.push(format!("- **{}** → `{}`: {}", e.state, e.behavior.as_deref().unwrap_or("None"), e.behavior_reasons.join("; ")));
        }
    }
    l.push(String::new());
    l.push("## Projectiles".into());
    l.push(String::new());
    l.push("| id | status | boxes | behavior | losses |".into());
    l.push("|---|---|---|---|---|".into());
    for p in prep {
        let losses: Vec<&str> = p["losses"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let ltxt: String = losses.join("; ").chars().take(200).collect::<String>().replace('|', "/");
        l.push(format!(
            "| {} | {} | {} | {} | {ltxt} |",
            p["id"].as_str().unwrap_or(""), p["status"].as_str().unwrap_or(""),
            p.get("hitboxes").map(|h| h.to_string()).unwrap_or_else(|| "-".into()),
            p.get("behavior").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("-")
        ));
    }
    l.push(String::new());
    l.push("## Input bindings shared by several moves".into());
    l.push(String::new());
    for c in conflicts {
        let moves: Vec<&str> = c["moves"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        l.push(format!("- `{}`: {}", c["binding"].as_str().unwrap_or(""), moves.join(", ")));
    }
    l.push(String::new());
    l.push("## Base states not converted".into());
    l.push(String::new());
    l.push(base_names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
    l.push(String::new());
    l.join("\n")
}

// ------------------------------------------------------------------ sounds and effects
fn res_path(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.get("res")).and_then(Value::as_str).map(str::to_string)
}

fn vec2(v: Option<&Value>) -> R {
    let parts: Vec<R> = v.and_then(|v| v.get("Vector2")).and_then(Value::as_array).map(|a| a.iter().map(|x| f(num(Some(x), 0.0))).collect()).unwrap_or_else(|| vec![f(0.0), f(0.0)]);
    R::Tuple(parts)
}

fn sound(path: Option<String>, volume: f64) -> Option<R> {
    path.map(|p| R::st("Sound", vec![("path", R::Str(p)), ("volume", f(volume))]))
}

fn opt(v: Option<R>) -> R {
    match v {
        Some(r) => R::Some(Box::new(r)),
        None => R::None,
    }
}

fn state_fx(s: &Value) -> R {
    let none = Map::new();
    let ex = at(s, "explicit_overrides").as_object().unwrap_or(&none);
    let g = |k: &str| ex.get(k);
    let enter = sound(res_path(g("enter_sfx")), num(g("enter_sfx_volume"), -15.0));
    let timed = res_path(g("sfx"));
    let sfx = timed.map(|p| R::st("TimedSound", vec![("path", R::Str(p)), ("tick", R::Int(to_int(Some(g("sfx_tick").unwrap_or(&json!(1)))))), ("volume", f(num(g("sfx_volume"), -15.0)))]));
    let release = sound(res_path(g("release_sfx")), num(g("release_sfx_volume"), -10.0));
    let mut particle = None;
    if let Some(scene) = res_path(g("particle_scene")) {
        if truthy(g("spawn_particle_on_enter")) {
            particle = Some(R::st("Particle", vec![("scene", R::Str(scene)), ("pos", vec2(g("particle_position"))), ("tick", R::Int(1))]));
        }
    }
    let timed_particle = res_path(g("timed_particle_scene")).map(|scene| {
        R::st("Particle", vec![("scene", R::Str(scene)), ("pos", vec2(g("timed_particle_position"))), ("tick", R::Int(to_int(Some(g("timed_spawn_particle_tick").unwrap_or(&json!(1))))))])
    });
    let hitboxes = at(s, "hitboxes").as_array().cloned().unwrap_or_default();
    let n = hitboxes.len();
    let mut ids: HashSet<String> = HashSet::new();
    let mut boxes: Vec<(String, R)> = Vec::new();
    for h in &hitboxes {
        let hname = at(h, "name").as_str().unwrap_or("");
        let base_id = if n > 1 { snake(hname) } else { "main".to_string() };
        let (mut hid, mut j) = (base_id.clone(), 2);
        while ids.contains(&hid) {
            hid = format!("{base_id}_{j}");
            j += 1;
        }
        ids.insert(hid.clone());
        let none2 = Map::new();
        let e = at(h, "explicit").as_object().unwrap_or(&none2);
        let snd = |key: &str, dpath: &str, dvol: f64| {
            let path = if e.contains_key(key) { res_path(e.get(key)) } else { Some(dpath.to_string()) };
            sound(path, num(e.get(&format!("{key}_volume")), dvol))
        };
        let hit = snd("hit_sound", "sound/common/hit1.wav", -5.0);
        let whiff = snd("whiff_sound", "sound/common/whiff1.wav", -8.0);
        let bass = if e.contains_key("hit_bass_sound") { res_path(e.get("hit_bass_sound")) } else { Some("sound/common/hit_bass.wav".to_string()) };
        boxes.push((
            hid,
            R::st(
                "HitboxFx",
                vec![
                    ("hit_sound", opt(hit)),
                    ("whiff_sound", opt(whiff)),
                    ("bass_sound", opt(sound(bass, num(e.get("bass_sound_volume"), -5.0)))),
                    ("bass_on_whiff", R::Bool(truthy(e.get("bass_on_whiff")))),
                    ("hit_particle", opt(res_path(e.get("hit_particle")).map(R::Str))),
                    ("replace_hit_particle", R::Bool(truthy(e.get("replace_hit_particle")))),
                    ("whiff_particle", opt(res_path(e.get("whiff_particle")).map(R::Str))),
                    ("screenshake", R::Int(to_int(Some(e.get("screenshake_amount").unwrap_or(&json!(4)))))),
                    ("screenshake_frames", R::Int(to_int(Some(e.get("screenshake_frames").unwrap_or(&json!(-1)))))),
                    ("bump_dir", vec2(e.get("camera_bump_dir"))),
                ],
            ),
        ));
    }
    R::st(
        "MoveFx",
        vec![
            ("enter_sfx", opt(enter)),
            ("sfx", opt(sfx)),
            ("release_sfx", opt(release)),
            ("particle", opt(particle)),
            ("timed_particle", opt(timed_particle)),
            ("hitboxes", R::Map(boxes)),
        ],
    )
}

fn fx_file(c: &Conv, char_id: &str) -> String {
    let mut moves: Vec<(String, R)> = Vec::new();
    for s in at(c.data, "states").as_array().cloned().unwrap_or_default() {
        let mid = sid(at(&s, "name").as_str().unwrap_or(""));
        if !moves.iter().any(|(m, _)| *m == mid) {
            moves.push((mid, state_fx(&s)));
        }
    }
    format!(
        "// Sounds and effects of the converted {char_id} (PRIVATE: paths into the original game's files).\n// Generated by the yemi-convert crate.\n{}\n",
        crate::ron::emit_fx(&R::st("CharacterFx", vec![("moves", R::Map(moves))]), 0)
    )
}
