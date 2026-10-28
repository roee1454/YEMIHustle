use charfmt::*;

const EXAMPLE: &str = include_str!("../examples/example_character.ron");

fn find<'a>(f: &'a CharacterFile, id: &str) -> &'a Move {
    f.character.moves.iter().find(|m| m.id == id).unwrap()
}

fn load(text: &str) -> CharacterFile {
    match parse(text) {
        Ok(f) => f,
        Err(e) => panic!("parse error: {e}"),
    }
}

#[test]
fn public_example_parses_and_validates() {
    let f = load(EXAMPLE);
    assert_eq!(f.validate(), Vec::<String>::new());
    let m = find(&f, "light_jab");
    assert_eq!(m.timeline.total(), 16);
    assert_eq!(m.timeline.active_span(), Some(Span { from: 6, to: 7 }));
    // defaults filled in
    assert_eq!(m.finish_into, "idle");
    assert_eq!(m.input.as_ref().unwrap().buffer, 6);
    assert!(m.hitboxes[0].flags.can_clash);
    assert_eq!(m.hitboxes[0].scaling, Scaling { proration: 0, scale_combo: true, counts_as_hit: true });
    assert_eq!(f.character.base_library, "standard");
}

#[test]
fn integer_literals_are_accepted_for_fixed_values() {
    let f = load(EXAMPLE);
    assert_eq!(f.character.body.max_ground_speed, Fx(12.0));
}

#[test]
fn round_trips_through_ron() {
    let f = load(EXAMPLE);
    let text = ron::ser::to_string_pretty(&f, ron::ser::PrettyConfig::default()).unwrap();
    // serialized form writes Option as Some(..), so it needs implicit_some off: parse as-is.
    let back: CharacterFile = ron::from_str(&text).unwrap();
    assert_eq!(f, back);
}

#[test]
fn unknown_fields_are_rejected() {
    let bad = EXAMPLE.replace("name: \"Light Jab\",", "name: \"Light Jab\", typo_field: 1,");
    assert!(parse(&bad).is_err());
}

#[test]
fn validation_catches_mistakes() {
    let mut f = load(EXAMPLE);
    {
        let m = f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap();
        m.hitboxes[0].window = Span { from: 1, to: 2 }; // outside active span 6..7
        m.cancels[0].window = Span { from: 6, to: 99 }; // beyond the timeline
        m.motion[0].at = 0; // frames are 1-based
        m.id = "Bad-Id".into();
    }
    let errs = f.validate();
    let joined = errs.join("\n");
    assert!(joined.contains("outside the active span"), "{joined}");
    assert!(joined.contains("cancel window"), "{joined}");
    assert!(joined.contains("motion event at 0"), "{joined}");
    assert!(joined.contains("snake_case"), "{joined}");
}

#[test]
fn active_frames_require_hitboxes() {
    let mut f = load(EXAMPLE);
    f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap().hitboxes.clear();
    assert!(f.validate().iter().any(|e| e.contains("no hitboxes")));
}

#[test]
fn extended_features_parse() {
    let f = load(EXAMPLE);
    assert_eq!(f.validate(), Vec::<String>::new());
    let dive = find(&f, "dive");
    assert_eq!(dive.timeline.end, TimelineEnd::OnLanding);
    assert!(dive.hitboxes[0].until_end);
    assert_eq!(dive.hurt.len(), 1);
    assert!(dive.cost.air_action);
    let grab = find(&f, "grab");
    assert_eq!(grab.hitboxes[0].throw_into.as_deref(), Some("toss_throw"));
    assert!(find(&f, "toss_throw").input.is_none());
    let spin = find(&f, "spin_finisher");
    assert!(spin.availability.combo_only);
    assert_eq!(spin.hitboxes[0].repeat, Some(Repeat { active: 2, inactive: 1 }));
    assert_eq!(f.character.projectiles[0].lifetime, 90);
}

#[test]
fn extended_validation_rules() {
    let mut f = load(EXAMPLE);
    {
        let m = &mut f.character.moves;
        m.iter_mut().find(|x| x.id == "dive").unwrap().timeline.end = TimelineEnd::Script; // no behavior
        m.iter_mut().find(|x| x.id == "grab").unwrap().hitboxes[0].throw_into = Some("nope".into());
        m.iter_mut().find(|x| x.id == "spin_finisher").unwrap().hitboxes[0].repeat =
            Some(Repeat { active: 0, inactive: 1 });
        m.iter_mut().find(|x| x.id == "spin_finisher").unwrap().availability.neutral_only = true;
    }
    let joined = f.validate().join("\n");
    assert!(joined.contains("no behavior to end it"), "{joined}");
    assert!(joined.contains("unknown move \"nope\""), "{joined}");
    assert!(joined.contains("repeat needs non-zero"), "{joined}");
    assert!(joined.contains("exclude each other"), "{joined}");
}

#[test]
fn open_ended_moves_may_list_no_frames() {
    let mut f = load(EXAMPLE);
    let dive = f.character.moves.iter_mut().find(|x| x.id == "dive").unwrap();
    dive.hitboxes.clear();
    dive.timeline = Timeline { startup: 0, active: 0, recovery: 0, end: TimelineEnd::OnLanding };
    assert_eq!(f.validate(), Vec::<String>::new());
}

#[test]
fn private_converted_files_validate_when_present() {
    // Converted game data lives outside the public tree; check whatever is there.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/converted");
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut checked = 0;
    for e in entries.flatten() {
        let p = e.path();
        // control schemes and sound/effect files sit next to the characters
        let not_character = p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.ends_with("_controls.ron") || n.ends_with("_fx.ron")
        });
        if p.extension().is_some_and(|x| x == "ron") && !not_character {
            let text = std::fs::read_to_string(&p).unwrap();
            let f = load(&text);
            assert_eq!(f.validate(), Vec::<String>::new(), "{}", p.display());
            assert!(!f.character.moves.is_empty());
            checked += 1;
        }
    }
    eprintln!("validated {checked} converted file(s)");
}

#[test]
fn zero_size_hitboxes_need_a_behavior() {
    let mut f = load(EXAMPLE);
    let jab = f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap();
    jab.hitboxes[0].rect.half_w = 0;
    assert!(f.validate().iter().any(|e| e.contains("zero size")));
    let jab = f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap();
    jab.behavior = Some("sample.light_jab".into());
    assert_eq!(f.validate(), Vec::<String>::new());
    let jab = f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap();
    jab.hitboxes[0].rect.half_w = -3;
    assert!(f.validate().iter().any(|e| e.contains("negative half size")));
}

#[test]
fn negative_proration_is_allowed() {
    let mut f = load(EXAMPLE);
    let jab = f.character.moves.iter_mut().find(|m| m.id == "light_jab").unwrap();
    jab.hitboxes[0].scaling.proration = -1;
    assert_eq!(f.validate(), Vec::<String>::new());
}

#[test]
fn modes_must_be_declared() {
    let mut f = load(EXAMPLE);
    assert_eq!(find(&f, "draw_staff").set_mode.as_deref(), Some("staff"));
    f.character.moves.iter_mut().find(|m| m.id == "draw_staff").unwrap().set_mode = Some("ghost".into());
    let joined = f.validate().join("\n");
    assert!(joined.contains("mode \"ghost\" is not declared"), "{joined}");
}

fn get<'a>(f: &'a mut CharacterFile, id: &str) -> &'a mut Move {
    f.character.moves.iter_mut().find(|m| m.id == id).unwrap()
}

#[test]
fn gap_features_parse() {
    let f = load(EXAMPLE);
    assert_eq!(f.validate(), Vec::<String>::new());
    // hit reactions and knockback modes
    let grab = find(&f, "grab");
    assert_eq!(grab.hitboxes[0].reaction.grounded, ReactionKind::Held);
    assert!(!grab.hitboxes[0].scaling.counts_as_hit);
    let spin = find(&f, "spin_finisher");
    let kb = spin.hitboxes[0].knockback;
    assert!(kb.reversible && kb.from == KnockbackFrom::TowardHitbox);
    assert_eq!(spin.hitboxes[0].min_damage, Some(10));
    assert!(spin.hitboxes[0].effects.air_ground_bounce);
    // defaults of the new structs
    let r = Reaction::default();
    assert_eq!((r.grounded, r.aerial, r.min_launch_frames), (ReactionKind::Stumble, ReactionKind::Launch, -1));
    assert!(Effects::default().stun_until_landing);
    // throw choreography
    let toss = find(&f, "toss_throw");
    let t = toss.throw.as_ref().unwrap();
    assert_eq!(t.hold, (14, -12));
    assert_eq!(t.keyframes.len(), 2);
    let rel = t.release.as_ref().unwrap();
    assert_eq!(rel.at, 8);
    assert_eq!(rel.hit.scaling.proration, 2);
    assert!(!rel.hit.effects.stun_until_landing);
    assert_eq!(toss.hitboxes[0].kind, HitKind::ThrowDamage);
    // physics overrides
    let dive = find(&f, "dive");
    let ph = dive.physics;
    assert_eq!(ph.gravity_override, Some(GravityOverride { gravity: Fx(0.6), fall_speed: Fx(12.0) }));
    assert_eq!(ph.speed_caps, Some(SpeedCaps { ground: Fx(10.0), air: Fx(12.0) }));
    assert_eq!(ph.friction_x, Some(Fx(0.05)));
    assert!(ph.uncapped_forces);
    assert!(matches!(dive.motion[0].kind, MotionKind::StopVertical));
    // parameters of every kind
    let shot = find(&f, "charge_shot");
    assert_eq!(shot.params.len(), 6);
    assert_eq!(shot.params[0].binding, Some(Binding::Stick));
    assert_eq!(shot.params[2].binding, Some(Binding::Hold { max_ticks: 30 }));
    assert_eq!(shot.params[3].binding, Some(Binding::Button(Button::Heavy)));
    assert_eq!(shot.params[4].binding, None);
    match &shot.params[1].kind {
        ParamKind::Direction { steps, allowed, .. } => {
            assert_eq!(*steps, DirSteps::Eight);
            assert_eq!(allowed, &vec![Compass::Forward, Compass::Back]);
        }
        other => panic!("expected a direction, got {other:?}"),
    }
}

#[test]
fn param_validation() {
    let mut f = load(EXAMPLE);
    {
        let m = get(&mut f, "charge_shot");
        m.params[2].kind = ParamKind::Slider { min: Fx(0.0), max: Fx(10.0), default: Fx(50.0) };
        m.params[3].binding = Some(Binding::Stick); // a toggle on the stick
        m.params[4].kind = ParamKind::Choice { options: vec!["a".into()], default: "b".into() };
        m.params[5].kind = ParamKind::Count { min: 3, max: 1, default: 2 };
        m.params[0].name = "Aim".into();
        m.params.push(Param { name: "aim".into(), kind: ParamKind::Toggle { default: true }, binding: None });
    }
    if let ParamKind::Direction { steps, .. } = &mut get(&mut f, "charge_shot").params[1].kind {
        *steps = DirSteps::Free; // `allowed` only applies to eight-way
    }
    let joined = f.validate().join("\n");
    assert!(joined.contains("only applies to eight-way"), "{joined}");
    assert!(joined.contains("default is outside min..max"), "{joined}");
    assert!(joined.contains("only a direction can be bound to the stick"), "{joined}");
    assert!(joined.contains("is not one of the options"), "{joined}");
    assert!(joined.contains("snake_case"), "{joined}");
}

#[test]
fn throw_and_physics_validation() {
    let mut f = load(EXAMPLE);
    {
        let m = get(&mut f, "toss_throw");
        let t = m.throw.as_mut().unwrap();
        t.keyframes = vec![HoldKey { at: 5, offset: (1, 1) }, HoldKey { at: 5, offset: (2, 2) }];
        t.release.as_mut().unwrap().at = 99;
        let d = get(&mut f, "dive");
        d.physics.speed_caps = Some(SpeedCaps { ground: Fx(-1.0), air: Fx(1.0) });
        d.physics.friction_y = Some(Fx(f64::INFINITY));
    }
    let joined = f.validate().join("\n");
    assert!(joined.contains("increasing frame order"), "{joined}");
    assert!(joined.contains("throw release at 99"), "{joined}");
    assert!(joined.contains("speed_caps needs non-negative"), "{joined}");
    assert!(joined.contains("friction_y must be a finite"), "{joined}");
}
