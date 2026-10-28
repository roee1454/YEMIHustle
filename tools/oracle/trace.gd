# Oracle: runs the original game headless and records what objects do each tick.
extends Node

func vec(v):
	return [str(v.x), str(v.y)]

func fighter_info(p):
	return {"x": str(p.get_pos().x), "y": str(p.get_pos().y), "state": p.current_state().name, "hp": p.hp, "facing": p.get_facing_int(), "hc": [str(p.get_hurtbox_center().x), str(p.get_hurtbox_center().y)]}

func obj_info(o):
	var info = {"px": str(o.position.x), "py": str(o.position.y), "name": o.name, "x": str(o.get_pos().x), "y": str(o.get_pos().y), "facing": o.get_facing_int(), "disabled": o.disabled}
	if o.get("state_machine") != null and o.state_machine.get("state") != null:
		info["state"] = o.current_state().name
		info["tick"] = o.current_state().current_tick
	if o.has_method("get_vel"):
		var v = o.get_vel()
		info["vx"] = str(v.x)
		info["vy"] = str(v.y)
	info["scene"] = o.filename
	return info

func _ready():
	call_deferred("run")

func run():
	yield(get_tree(), "idle_frame")
	var spec_path = OS.get_environment("YEMI_TRACE_SPEC")
	var f = File.new()
	f.open(spec_path, File.READ)
	var spec = parse_json(f.get_as_text())
	f.close()

	var data = {
		"singleplayer": true,
		"selected_characters": {1: {"name": spec.get("char", "Ninja")}, 2: {"name": spec.get("opponent", "Ninja")}},
		"selected_styles": {1: null, 2: null},
		"seed": 1,
		"stage_width": int(spec.get("stage_width", 1100)),
		"char_distance": int(spec.get("char_distance", 200)),
		"p2_dummy": true,
		"has_ceiling": true,
		"ceiling_height": 400,
		"game_length": 3000,
		"clashing_enabled": true,
		"asymmetrical_clashing": false,
		"prediction_enabled": false,
	}
	var game = preload("res://Game.tscn").instance()
	get_tree().root.add_child(game)
	var r = game.start_game(true, data)
	var p1 = game.get_player(1)
	var p2 = game.get_player(2)
	# let both fighters settle until the game asks for actions
	var guard = 0
	while not game.is_waiting_on_player() and guard < 600:
		game.tick()
		guard += 1
	var rows = []
	var pre = int(spec.get("pre_ticks", 0))
	p1.queued_action = spec.get("state", "Wait")
	p1.queued_data = spec.get("data", null)
	p1.queued_extra = spec.get("extra", {})
	if spec.get("air", false):
		p1.set_pos(p1.get_pos().x, -int(spec.get("air_height", 100)))
		p1.update_grounded()
	p2.queued_action = "Wait"
	for t in range(int(spec.get("ticks", 90))):
		game.tick()
		var objs = []
		for o in game.active_objects:
			if o.is_in_group("Fighter"):
				continue
			objs.append(obj_info(o))
		rows.append({"tick": game.current_tick, "p1": fighter_info(p1), "p2": fighter_info(p2), "objs": objs})
	var out = File.new()
	out.open(spec.get("out", "/tmp/trace.json"), File.WRITE)
	out.store_string(to_json({"spec": spec, "settle_ticks": guard, "rows": rows}))
	out.close()
	print("TRACE done: ", rows.size(), " rows")
	get_tree().quit()
