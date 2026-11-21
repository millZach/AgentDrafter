//! The drawing tools, natively: each tool's steps, the change set it gives,
//! its snapped points, typed coordinates, Ortho, the template warning and
//! its aliases.

use super::*;
use serde_json::json;

/// A made-up sheet like a demo plant loop sheet: a template's border and
/// title, and the row's own device, on a template three sheets share.
fn context() -> Context {
    let object = |id: &str, kind: &str, template: bool| ObjectInfo {
        id: Some(format!("DP-LD-101/{id}")),
        name: None,
        kind: kind.to_owned(),
        block: None,
        layer: "0".to_owned(),
        values: None,
        from_template: Some(template),
        through_viewport: false,
    };
    Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects: vec![
            object("border", "polyline", true),
            object("title", "text", true),
            object("device", "insert", false),
            ObjectInfo {
                id: None,
                kind: "line".to_owned(),
                ..ObjectInfo::default()
            },
        ],
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec![
                "DP-LD-101".to_owned(),
                "DP-LD-102".to_owned(),
                "DP-LD-104".to_owned(),
            ],
        }),
        past: false,
        space: None,
        layout: None,
    }
}

fn tools() -> Tools {
    let mut tools = Tools::default();
    tools.load(context());
    tools
}

fn at(x: f64, y: f64) -> Taken {
    Taken {
        at: [x, y],
        z: 0.0,
        snap: None,
        typed: false,
    }
}

fn snapped(x: f64, y: f64, snap: SnapKind, object: usize, which: Option<&str>) -> Taken {
    Taken {
        at: [x, y],
        z: 0.0,
        snap: Some(SnapRef {
            snap,
            object,
            which: which.map(str::to_owned),
            with: None,
        }),
        typed: false,
    }
}

fn operations(outcome: &Outcome) -> Vec<Value> {
    outcome
        .sends
        .last()
        .expect("a change set to send")
        .change_set["operations"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_aliases_start_their_tools_in_any_case() {
    for (word, kind) in [
        ("L", ToolKind::Line),
        ("line", ToolKind::Line),
        ("c", ToolKind::Circle),
        ("REC", ToolKind::Rectangle),
        ("rectang", ToolKind::Rectangle),
        ("T", ToolKind::Text),
        ("dt", ToolKind::Text),
        ("M", ToolKind::Move),
        ("co", ToolKind::Copy),
        ("CP", ToolKind::Copy),
        ("copy", ToolKind::Copy),
        ("e", ToolKind::Erase),
        ("erase", ToolKind::Erase),
    ] {
        assert_eq!(tool_for(word), Some(kind), "{word}");
    }
    assert_eq!(tool_for("PLINE"), None);
    assert_eq!(tool_for("X"), None);
    // No Array tool yet: its words are no tool's, so they go to the
    // assistant.
    for word in ["AR", "ARRAY", "ARRAYRECT"] {
        assert_eq!(tool_for(word), None, "{word}");
    }
}

#[test]
fn line_sends_one_add_line_per_segment_on_the_current_layer_with_its_snapped_points() {
    let mut t = tools();
    t.layer = "wire".to_owned();
    t.input("l", None);
    assert_eq!(t.running(), Some(ToolKind::Line));
    t.point(snapped(2.3, 6.5, SnapKind::Endpoint, 0, Some("points[0]")));
    t.point(at(4.123456, 6.5));
    let outcome = t.point(at(4.123456, 8.0));
    assert!(outcome.sends.is_empty(), "Line keeps going until Done");
    let outcome = t.option("done");
    let ops = operations(&outcome);
    assert_eq!(ops.len(), 2);
    assert_eq!(
        ops[0],
        json!({"op": "add_line", "sheet": "DP-LD-101", "layer": "wire",
               "from": {"snap": {"id": "DP-LD-101/border", "snap": "endpoint", "which": "points[0]"}},
               "to": [4.1235, 6.5, 0]})
    );
    assert_eq!(ops[1]["from"], json!([4.1235, 6.5, 0]));
    assert_eq!(ops[1]["to"], json!([4.1235, 8.0, 0]));
    let send = outcome.sends.last().cloned().unwrap();
    assert_eq!(send.what, "2 lines drawn");
    assert_eq!(send.sheets, ["DP-LD-101"]);
    assert!(send.template.is_none(), "an add goes in the sheet's row");
    assert_eq!(t.running(), None);
}

#[test]
fn typed_coordinates_relative_points_lengths_and_close() {
    let mut t = tools();
    t.input("LINE", None);
    t.input("1,1", None);
    t.input("@2,0", None);
    t.input("@1<90", None);
    // A length alone goes towards the cursor.
    t.input("0.5", Some([0.0, 2.0]));
    let state = t.state(false);
    assert_eq!(state.points.len(), 4);
    assert!((state.points[2][0] - 3.0).abs() < 1e-12 && (state.points[2][1] - 2.0).abs() < 1e-12);
    assert!((state.points[3][0] - 2.5).abs() < 1e-12 && (state.points[3][1] - 2.0).abs() < 1e-12);
    assert!(state.options.iter().any(|o| o.act == "close"));
    // C closes the line rather than starting Circle.
    let ops = operations(&t.input("c", None));
    assert_eq!(ops.len(), 4);
    assert_eq!(ops[3]["to"], json!([1.0, 1.0, 0]));
}

#[test]
fn u_takes_back_the_last_point_and_esc_keeps_what_was_drawn() {
    let mut t = tools();
    t.start(ToolKind::Line, &[]);
    t.point(at(0.0, 0.0));
    t.point(at(1.0, 0.0));
    t.point(at(1.0, 1.0));
    t.input("u", None);
    assert_eq!(t.state(false).points.len(), 2);
    let ops = operations(&t.escape());
    assert_eq!(
        ops.len(),
        1,
        "Esc keeps the line drawn, as AutoCAD's LINE does"
    );
    // Esc with nothing drawn changes nothing.
    t.start(ToolKind::Line, &[]);
    t.point(at(0.0, 0.0));
    let outcome = t.escape();
    assert!(outcome.sends.is_empty());
    assert_eq!(outcome.log, ["LINE: cancelled, nothing changed"]);
}

#[test]
fn ortho_holds_a_line_level_or_upright_but_a_snap_wins() {
    let mut t = tools();
    t.ortho = true;
    t.start(ToolKind::Line, &[]);
    t.point(at(1.0, 1.0));
    assert_eq!(t.aim([3.0, 1.4], None).at, [3.0, 1.0]);
    assert_eq!(t.aim([1.2, 4.0], None).at, [1.0, 4.0]);
    let snap = SnapRef {
        snap: SnapKind::Endpoint,
        object: 0,
        which: None,
        with: None,
    };
    assert_eq!(
        t.aim([3.0, 1.4], Some((snap, [3.1, 1.6, 0.0]))).at,
        [3.1, 1.6]
    );
    // A length typed alone goes level, towards the cursor.
    t.input("2", Some([1.5, 1.2]));
    assert_eq!(t.state(false).points[1], [3.0, 1.0]);
    // A click held level keeps the base's height exactly, though it is not
    // a whole ten-thousandth; the other coordinate is kept to one.
    t.escape();
    t.start(ToolKind::Line, &[]);
    t.point(snapped(1.0, 1.000_012_345, SnapKind::Endpoint, 0, None));
    let held = t.aim([3.123_456_7, 1.4], None);
    t.point(held);
    assert_eq!(t.state(false).points[1], [3.1235, 1.000_012_345]);
}

#[test]
fn circle_takes_a_radius_point_a_typed_radius_or_a_diameter() {
    let mut t = tools();
    t.start(ToolKind::Circle, &[]);
    t.point(snapped(5.0, 5.0, SnapKind::Center, 2, None));
    let ops = operations(&t.point(at(5.3, 5.4)));
    assert_eq!(
        ops[0],
        json!({"op": "add_circle", "sheet": "DP-LD-101", "layer": "0",
               "center": {"snap": {"id": "DP-LD-101/device", "snap": "center"}},
               "radius": 0.5})
    );
    t.start(ToolKind::Circle, &[]);
    t.input("1,1", None);
    t.option("diameter");
    assert!(t.state(false).prompt.starts_with("Type the diameter"));
    let ops = operations(&t.input("0.5", None));
    assert_eq!(ops[0]["radius"], json!(0.25));
}

#[test]
fn rectangle_is_one_closed_polyline() {
    let mut t = tools();
    t.input("rec", None);
    t.input("1,1", None);
    let state = t.state(false);
    assert!(state.prompt.contains("@width,height"));
    let outcome = t.input("@2,1", None);
    let ops = operations(&outcome);
    assert_eq!(
        ops[0],
        json!({"op": "add_polyline", "sheet": "DP-LD-101", "layer": "0",
               "points": [[1.0, 1.0, 0], [3.0, 1.0, 0], [3.0, 2.0, 0], [1.0, 2.0, 0]],
               "closed": true})
    );
    assert_eq!(
        outcome.sends.last().cloned().unwrap().what,
        "Rectangle drawn, 2.000 × 1.000 in"
    );
    // Corners level with each other would draw a line.
    t.start(ToolKind::Rectangle, &[]);
    t.input("1,1", None);
    assert!(t.input("3,1", None).sends.is_empty());
    assert!(t.state(false).error.is_some());
}

#[test]
fn text_is_one_add_text_per_line_at_its_height() {
    let mut t = tools();
    t.input("dt", None);
    t.input("h", None);
    t.input("0.2", None);
    t.point(at(1.0, 2.0));
    let state = t.state(false);
    assert!(state.typing_text);
    t.typing("PUMP P-1");
    assert_eq!(t.state(false).text.unwrap().lines[0].text, "PUMP P-1");
    t.input("PUMP P-1", None);
    t.input("SUCTION", None);
    let ops = operations(&t.input("", None));
    assert_eq!(ops.len(), 2);
    assert_eq!(
        ops[0],
        json!({"op": "add_text", "sheet": "DP-LD-101", "layer": "0",
               "position": [1.0, 2.0, 0], "contents": "PUMP P-1", "height": 0.2})
    );
    let second = ops[1]["position"].as_array().unwrap();
    assert!((second[1].as_f64().unwrap() - (2.0 - 0.2 * 5.0 / 3.0)).abs() < 1e-12);
    // Done finishes, the line being typed with it.
    t.start(ToolKind::Text, &[]);
    t.point(at(5.0, 5.0));
    t.input("FIRST", None);
    t.typing("SECOND");
    let ops = operations(&t.option("done"));
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[1]["contents"], "SECOND");
    // A phone's box adds its text at once, at the height it gives.
    t.start(ToolKind::Text, &[]);
    t.point(at(1.0, 1.0));
    let ops = operations(&t.text_box("VENT", Some(0.1)));
    assert_eq!(ops[0]["contents"], "VENT");
    assert_eq!(ops[0]["height"], json!(0.1));
}

#[test]
fn move_picks_then_takes_a_base_and_a_second_point() {
    let mut t = tools();
    t.input("m", None);
    assert!(t.state(false).selecting);
    t.pick(Some(2));
    t.pick(Some(3));
    assert!(
        t.state(false).error.unwrap().contains("no ID"),
        "an object with no ID cannot be named"
    );
    t.input("", None);
    t.point(snapped(2.0, 6.5, SnapKind::Insertion, 2, None));
    let outcome = t.input("@1,0", None);
    let ops = operations(&outcome);
    assert_eq!(
        ops[0],
        json!({"op": "move", "target": {"id": "DP-LD-101/device"},
               "from": {"snap": {"id": "DP-LD-101/device", "snap": "insertion"}},
               "to": [3.0, 6.5, 0]})
    );
    let send = outcome.sends.last().cloned().unwrap();
    assert!(
        send.template.is_none(),
        "the row's own object reaches its sheet alone"
    );
    assert_eq!(send.sheets, ["DP-LD-101"]);
}

#[test]
fn moving_a_templates_object_warns_and_names_its_sheets_with_row_snaps_as_coordinates() {
    let mut t = tools();
    // Selected first, then Move.
    t.start(ToolKind::Move, &[0]);
    let state = t.state(false);
    let warning = state.warning.unwrap();
    assert_eq!(warning.title, "This is part of the template.");
    assert_eq!(
        warning.detail,
        "Moving it changes 3 sheets. You will be asked before it lands."
    );
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    let outcome = t.point(snapped(2.0, 6.5, SnapKind::Insertion, 2, None));
    let send = outcome.sends.last().cloned().unwrap();
    let edit = send.template.unwrap();
    assert_eq!(edit.path, "templates/loop-2-wire.yaml");
    assert_eq!(edit.sheets, ["DP-LD-101", "DP-LD-102", "DP-LD-104"]);
    assert_eq!(edit.detail, "Moving it changes 3 sheets.");
    assert_eq!(
        edit.edit,
        format!("Move {}", send.what.trim_start_matches("Moved ")),
        "a preview names the edit"
    );
    assert_eq!(send.sheets, edit.sheets);
    let op = &send.change_set["operations"][0];
    assert_eq!(
        op["from"],
        json!({"snap": {"id": "DP-LD-101/border", "snap": "endpoint", "which": "points[0]"}}),
        "a snap on a template's object stays a snapped point"
    );
    assert_eq!(
        op["to"],
        json!([2.0, 6.5, 0]),
        "a snap on a row's own object goes as its coordinates"
    );
}

#[test]
fn erase_with_objects_selected_first_erases_them_at_once() {
    let mut t = tools();
    let outcome = t.start(ToolKind::Erase, &[1, 2]);
    let send = outcome.sends.last().cloned().unwrap();
    assert_eq!(
        send.change_set,
        json!({"operations": [
            {"op": "delete", "target": {"id": "DP-LD-101/title"}},
            {"op": "delete", "target": {"id": "DP-LD-101/device"}}
        ]})
    );
    assert_eq!(
        send.template.unwrap().title,
        "One of these is part of the template."
    );
    assert_eq!(t.running(), None);
    // The tool first, then the picks.
    t.input("E", None);
    t.pick(Some(2));
    t.pick(Some(2));
    assert!(
        t.state(false).picked.is_empty(),
        "a second pick takes it out"
    );
    t.pick(Some(2));
    assert_eq!(t.state(false).options[0].label, "Erase 1");
    let send = t.option("done").sends.last().cloned().unwrap();
    assert_eq!(send.what, "Erased device");
}

#[test]
fn enter_at_an_empty_prompt_repeats_the_last_tool_and_a_word_not_a_tool_says_so() {
    let mut t = tools();
    t.input("c", None);
    t.escape();
    t.input("", None);
    assert_eq!(t.running(), Some(ToolKind::Circle));
    t.escape();
    t.input("hello", None);
    assert!(t.state(false).error.unwrap().contains("not a drawing tool"));
}

#[test]
fn another_tool_keeps_the_lines_drawn_and_starts() {
    let mut t = tools();
    t.input("L", None);
    t.input("0,0", None);
    t.input("1,0", None);
    let outcome = t.input("REC", None);
    assert_eq!(operations(&outcome).len(), 1);
    assert_eq!(t.running(), Some(ToolKind::Rectangle));
}

#[test]
fn a_sheet_that_is_not_a_sets_or_the_set_as_it_was_is_not_drawn_on() {
    let mut t = Tools::default();
    t.load(Context {
        template: None,
        ..context()
    });
    t.input("L", None);
    assert_eq!(t.running(), None);
    assert!(t.state(false).error.unwrap().contains("drawing set"));
    t.load(Context {
        past: true,
        ..context()
    });
    t.input("L", None);
    assert_eq!(t.running(), None);
}

#[test]
fn a_sheet_drawn_again_keeps_the_picks_by_their_ids_and_another_sheet_ends_the_tool() {
    let mut t = tools();
    t.start(ToolKind::Move, &[]);
    t.pick(Some(2));
    // An agent deletes the border: the device moves up one place.
    let mut after = context();
    after.objects.remove(0);
    t.load(after.clone());
    assert_eq!(t.state(false).picked, [1]);
    after.sheet = "DP-LD-102".to_owned();
    t.load(after);
    assert_eq!(t.running(), None);
}

#[test]
fn the_rubber_band_and_the_words_at_the_cursor() {
    let mut t = tools();
    t.start(ToolKind::Line, &[]);
    t.point(at(0.0, 0.0));
    let aim = t.aim([3.0, 4.0], None);
    let band = t.band(Some(&aim));
    assert_eq!(band.lines, [[[0.0, 0.0], [3.0, 4.0]]]);
    let (length, angle) = t.cursor_words(&aim).unwrap();
    assert_eq!(length, "5.000 in");
    assert_eq!(angle.unwrap(), "53.1°");
    t.escape();
    t.start(ToolKind::Circle, &[]);
    t.point(at(0.0, 0.0));
    let band = t.band(Some(&t.aim([1.0, 0.0], None)));
    assert_eq!(band.lines.len(), CIRCLE_PIECES);
    assert_eq!(band.guides.len(), 1);
    t.escape();
    t.start(ToolKind::Move, &[2]);
    t.point(at(1.0, 1.0));
    let band = t.band(Some(&t.aim([2.0, 3.0], None)));
    assert_eq!(band.moving, Some((vec![2], [1.0, 2.0])));
}

#[test]
fn a_group_move_goes_by_coordinates_and_one_objects_keeps_its_snaps() {
    // Two objects from a snap on the first: the core would find the snap
    // again after moving the first, and the second would move by nothing.
    let mut t = tools();
    t.start(ToolKind::Move, &[0, 1]);
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    let ops = operations(&t.input("@1,0", None));
    assert_eq!(ops.len(), 2);
    for op in &ops {
        assert_eq!(op["from"], json!([0.5, 0.5, 0]));
        assert_eq!(op["to"], json!([1.5, 0.5, 0]));
    }
    // Two objects to a snap on another object: by coordinates too, the
    // same displacement for each.
    t.start(ToolKind::Move, &[0, 1]);
    t.point(at(1.0, 1.0));
    let ops = operations(&t.point(snapped(
        2.5,
        6.5,
        SnapKind::Endpoint,
        2,
        Some("objects[3].to"),
    )));
    assert!(ops.iter().all(|op| op["to"] == json!([2.5, 6.5, 0])));
    // One object to a snap on itself: the core works it out before it
    // moves, so the snapped point stays.
    t.start(ToolKind::Move, &[2]);
    t.point(at(2.0, 6.5));
    let ops = operations(&t.point(snapped(
        2.5,
        6.5,
        SnapKind::Endpoint,
        2,
        Some("objects[3].to"),
    )));
    assert_eq!(ops[0]["to"]["snap"]["which"], "objects[3].to");
    // One object from a snap on another: the snapped point stays.
    t.start(ToolKind::Move, &[2]);
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    let ops = operations(&t.input("@1,0", None));
    assert_eq!(ops[0]["from"]["snap"]["id"], "DP-LD-101/border");
}

#[test]
fn on_a_sheet_drawn_in_a_layout_an_add_names_its_space_and_move_leaves_what_a_viewport_shows() {
    let mut c = context();
    c.space = Some("paper".to_owned());
    c.objects[2].through_viewport = true;
    let mut t = Tools::default();
    t.load(c);
    t.input("L", None);
    t.input("1,1", None);
    t.input("2,1", None);
    let ops = operations(&t.input("", None));
    assert_eq!(ops[0]["space"], "paper");
    // Move refuses an object seen through a viewport, picked or selected first.
    t.start(ToolKind::Move, &[]);
    t.pick(Some(2));
    assert!(t.state(false).picked.is_empty());
    assert!(t
        .state(false)
        .error
        .unwrap()
        .contains("seen through a viewport"));
    t.escape();
    t.start(ToolKind::Move, &[2]);
    assert_eq!(t.running(), Some(ToolKind::Move));
    assert!(
        t.state(false).selecting,
        "with nothing left to move, it asks for objects"
    );
    // Erase takes it: deleting needs no point of its own.
    t.escape();
    let send = t.start(ToolKind::Erase, &[2]).sends.pop().unwrap();
    assert_eq!(send.change_set["operations"][0]["op"], "delete");
}

#[test]
fn cancel_lands_nothing_typed_in_the_line_being_typed() {
    let mut t = tools();
    t.start(ToolKind::Text, &[]);
    t.point(at(1.0, 1.0));
    t.typing("VALVE");
    let outcome = t.option("cancel");
    assert!(outcome.sends.is_empty(), "nothing typed in the box lands");
    assert_eq!(t.running(), None);
    // Esc keeps it, as AutoCAD's TEXT does.
    t.start(ToolKind::Text, &[]);
    t.point(at(1.0, 1.0));
    t.typing("VALVE");
    assert_eq!(operations(&t.escape())[0]["contents"], "VALVE");
}

#[test]
fn another_tool_that_sends_at_once_sends_after_what_the_first_kept() {
    let mut t = tools();
    t.start(ToolKind::Text, &[]);
    t.point(at(1.0, 1.0));
    t.typing("VENT");
    let outcome = t.start(ToolKind::Erase, &[2]);
    let ops: Vec<&str> = outcome
        .sends
        .iter()
        .map(|s| s.change_set["operations"][0]["op"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["add_text", "delete"], "both, in order");
    assert!(outcome.log.iter().any(|l| l.contains("Text “VENT” added")));
}

#[test]
fn radius_sets_the_radius_and_diameter_the_diameter() {
    let mut t = tools();
    t.input("C", None);
    t.input("0,0", None);
    t.input("RADIUS", None);
    assert_eq!(operations(&t.input("10", None))[0]["radius"], json!(10.0));
    t.input("C", None);
    t.input("0,0", None);
    t.input("D", None);
    t.input("D", None);
    assert!(
        t.state(false).prompt.starts_with("Type the diameter"),
        "D twice stays on Diameter"
    );
    t.option("radius");
    assert_eq!(operations(&t.input("10", None))[0]["radius"], json!(10.0));
}

#[test]
fn a_group_move_keeps_each_points_height() {
    // From a snap at Z 0 to one at Z 5 on stationary geometry: each object
    // moves by (10, 0, 5), as each single move would.
    let mut t = tools();
    t.start(ToolKind::Move, &[0, 1]);
    t.point(snapped(
        0.0,
        0.0,
        SnapKind::Endpoint,
        2,
        Some("objects[3].from"),
    ));
    let mut to = snapped(10.0, 0.0, SnapKind::Endpoint, 2, Some("objects[3].to"));
    to.z = 5.0;
    let ops = operations(&t.point(to));
    for op in &ops {
        assert_eq!(op["from"], json!([0.0, 0.0, 0]));
        assert_eq!(op["to"], json!([10.0, 0.0, 5.0]));
    }
    // A point typed relative to a snap at Z 5 is at Z 5: the move is level.
    t.start(ToolKind::Move, &[0, 1]);
    let mut from = snapped(1.0, 1.0, SnapKind::Endpoint, 2, Some("objects[3].from"));
    from.z = 5.0;
    t.point(from);
    let ops = operations(&t.input("@2,0", None));
    assert_eq!(ops[0]["from"], json!([1.0, 1.0, 5.0]));
    assert_eq!(ops[0]["to"], json!([3.0, 1.0, 5.0]));
}

#[test]
fn another_layout_of_the_sheet_drops_the_tool_running_with_nothing_sent() {
    // As another sheet does: its points were taken on another page.
    let on = |layout: &str| Context {
        layout: Some(layout.to_owned()),
        space: Some(layout.to_owned()),
        ..context()
    };
    let mut t = Tools::default();
    t.load(on("Assembly"));
    t.input("L", None);
    t.input("1,1", None);
    t.input("2,1", None);
    t.load(on("Assembly"));
    assert_eq!(
        t.running(),
        Some(ToolKind::Line),
        "the same layout drawn again keeps it"
    );
    t.load(on("Wiring"));
    assert_eq!(t.running(), None);
    // Enter then sends nothing of what was taken on Assembly.
    assert!(t.input("", None).sends.is_empty());
}

#[test]
fn copy_picks_takes_a_base_and_places_copies_until_enter_as_one_copy_with_offsets() {
    let mut t = tools();
    t.input("co", None);
    assert_eq!(t.running(), Some(ToolKind::Copy));
    let state = t.state(false);
    assert!(state.selecting);
    assert_eq!(state.prompt, "Click the objects to copy, then Enter");
    t.pick(Some(2));
    t.input("", None);
    assert_eq!(
        t.state(false).prompt,
        "Click a point to copy it from: the base point"
    );
    t.point(snapped(2.0, 6.5, SnapKind::Insertion, 2, None));
    let state = t.state(false);
    assert_eq!(
        state.prompt,
        "Click where the copy goes, or type @dx,dy or a length"
    );
    assert_eq!(state.options.len(), 1, "{:?}", state.options);
    assert_eq!(state.options[0].label, "Done");
    // Each point places a copy, and the tool goes on from the same base.
    let outcome = t.point(at(2.0, 7.7));
    assert!(outcome.sends.is_empty(), "the copies wait for Enter");
    assert_eq!(
        outcome.log,
        ["COPY 1: 0.0000, 1.2000 in from the base point, dashed until Enter"]
    );
    // A typed displacement is from the base point, not the last copy.
    t.input("@0,-1.2", None);
    let state = t.state(false);
    assert_eq!(state.placed, 2);
    assert_eq!(state.points, [[2.0, 6.5]], "the base point stays");
    assert_eq!(
        state.prompt,
        "2 copies placed, dashed until you finish. Click where the next goes, or type @dx,dy \
         or a length"
    );
    let labels: Vec<&str> = state.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Undo", "Done: copy 2"]);
    assert_eq!(
        t.placed_tags(false),
        [
            ("copy 1 · waits for Enter".to_owned(), [0.0, 1.2]),
            ("copy 2 · waits for Enter".to_owned(), [0.0, -1.2]),
        ]
    );
    assert_eq!(t.placed_tags(true)[1].0, "copy 2");
    let outcome = t.input("", None);
    let ops = operations(&outcome);
    assert_eq!(
        ops,
        [json!({"op": "copy", "target": {"id": "DP-LD-101/device"},
                "offsets": [[0.0, 1.2, 0], [0.0, -1.2, 0]]})],
        "one copy operation, its offsets cleared of subtraction's noise"
    );
    let send = outcome.sends.last().cloned().unwrap();
    assert_eq!(send.what, "Copied device, 2 copies");
    assert_eq!(send.sheets, ["DP-LD-101"]);
    assert!(send.template.is_none());
    assert_eq!(t.running(), None);
}

#[test]
fn one_copy_goes_from_and_to_with_its_snapped_points() {
    let mut t = tools();
    t.start(ToolKind::Copy, &[2]);
    t.point(snapped(2.0, 6.5, SnapKind::Insertion, 2, None));
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    let outcome = t.option("done");
    assert_eq!(
        operations(&outcome),
        [json!({"op": "copy", "target": {"id": "DP-LD-101/device"},
                "from": {"snap": {"id": "DP-LD-101/device", "snap": "insertion"}},
                "to": {"snap": {"id": "DP-LD-101/border", "snap": "endpoint", "which": "points[0]"}}})]
    );
    assert_eq!(outcome.sends[0].what, "Copied device");
}

#[test]
fn copy_undo_takes_back_the_last_placed_and_esc_lands_the_rest() {
    let mut t = tools();
    t.start(ToolKind::Copy, &[2]);
    t.input("1,1", None);
    assert!(
        t.input("u", None).sends.is_empty(),
        "with none placed, U takes nothing back"
    );
    assert_eq!(
        t.state(false).error.as_deref(),
        Some("No copy is placed yet to take back.")
    );
    t.input("@1,0", None);
    t.input("@2,0", None);
    t.input("@3,0", None);
    let outcome = t.option("undo");
    assert_eq!(outcome.log, ["COPY: last copy taken back, 2 copies placed"]);
    assert!(outcome.sends.is_empty());
    t.input("U", None);
    assert_eq!(t.state(false).placed, 1);
    // A copy placed on the base point would sit on the original.
    t.input("1,1", None);
    assert!(t.state(false).error.unwrap().contains("base point"));
    assert_eq!(t.state(false).placed, 1);
    t.input("@0,2", None);
    // Esc lands the copies placed, as AutoCAD's COPY keeps them.
    let outcome = t.escape();
    assert_eq!(
        operations(&outcome)[0]["offsets"],
        json!([[1.0, 0.0, 0], [0.0, 2.0, 0]])
    );
    assert_eq!(t.running(), None);
    // With none placed, Esc just ends the tool.
    t.start(ToolKind::Copy, &[2]);
    t.input("1,1", None);
    let outcome = t.escape();
    assert!(outcome.sends.is_empty());
    assert_eq!(outcome.log, ["COPY: cancelled, nothing changed"]);
    assert_eq!(t.running(), None);
}

#[test]
fn copy_takes_a_length_towards_the_cursor_and_ortho() {
    let mut t = tools();
    t.ortho = true;
    t.start(ToolKind::Copy, &[2]);
    t.input("1,1", None);
    assert_eq!(t.aim([3.0, 1.4], None).at, [3.0, 1.0], "Ortho holds it level");
    t.input("2", Some([1.2, 5.0]));
    let band = t.band(Some(&at(4.0, 1.0)));
    let (picked, offsets) = band.placed.clone().expect("the copy placed shows");
    assert_eq!(picked, [2]);
    assert_eq!(offsets, [[0.0, 2.0]], "upright, held by Ortho, 2 long");
    assert_eq!(band.copying, Some((vec![2], [3.0, 0.0])));
    assert_eq!(band.guides, [[[1.0, 1.0], [4.0, 1.0]]]);
    assert!(band.moving.is_none(), "the objects stay where they are");
    let words = t.cursor_words(&at(4.0, 1.0)).unwrap();
    assert_eq!(words.0, "3.000 in");
}

#[test]
fn copying_a_templates_object_warns_and_asks_once_when_the_copies_land() {
    let mut t = tools();
    t.input("cp", None);
    t.pick(Some(0));
    t.input("", None);
    let warning = t.state(false).warning.unwrap();
    assert_eq!(warning.title, "This is part of the template.");
    assert_eq!(
        warning.detail,
        "This copies it on every sheet built from this template: 3 sheets. You will be asked \
         before it lands."
    );
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    // Each copy placed asks nothing: the change set waits for Enter.
    assert!(t.point(at(0.5, 2.0)).sends.is_empty());
    assert!(t.point(at(0.5, 3.0)).sends.is_empty());
    assert_eq!(t.state(false).warning.unwrap().edit, "Copy border, 2 copies");
    let outcome = t.input("", None);
    assert_eq!(outcome.sends.len(), 1, "one change set, asked about once");
    let send = outcome.sends[0].clone();
    let edit = send.template.unwrap();
    assert_eq!(edit.sheets, ["DP-LD-101", "DP-LD-102", "DP-LD-104"]);
    assert_eq!(
        edit.detail,
        "This copies it on every sheet built from this template: 3 sheets."
    );
    assert_eq!(edit.edit, "Copy border, 2 copies");
    assert_eq!(send.sheets, edit.sheets);
    // One copy of a template's object keeps a snap on the template's
    // object, and a snap on a row's own object goes as its coordinates.
    t.start(ToolKind::Copy, &[0]);
    t.point(snapped(0.5, 0.5, SnapKind::Endpoint, 0, Some("points[0]")));
    t.point(snapped(2.0, 6.5, SnapKind::Insertion, 2, None));
    let op = &operations(&t.input("", None))[0];
    assert_eq!(
        op["from"],
        json!({"snap": {"id": "DP-LD-101/border", "snap": "endpoint", "which": "points[0]"}})
    );
    assert_eq!(op["to"], json!([2.0, 6.5, 0]));
}

#[test]
fn copy_of_several_names_its_targets_and_another_tool_lands_the_copies_first() {
    let mut t = tools();
    t.start(ToolKind::Copy, &[]);
    t.pick(Some(1));
    t.pick(Some(2));
    t.input("", None);
    assert_eq!(
        t.state(false).prompt,
        "Click a point to copy them from: the base point"
    );
    t.input("0,0", None);
    t.input("@5,0", None);
    let warning = t.state(false).warning.unwrap();
    assert_eq!(warning.title, "One of these is part of the template.");
    // Another tool lands the copies placed, then starts.
    let outcome = t.input("L", None);
    assert_eq!(t.running(), Some(ToolKind::Line));
    let ops = operations(&outcome);
    assert_eq!(
        ops[0]["targets"],
        json!([{"id": "DP-LD-101/title"}, {"id": "DP-LD-101/device"}])
    );
    assert_eq!(ops[0]["from"], json!([0.0, 0.0, 0]));
    assert_eq!(ops[0]["to"], json!([5.0, 0.0, 0]));
    let send = &outcome.sends[0];
    assert_eq!(send.what, "Copied 2 objects");
    assert_eq!(send.sheets, ["DP-LD-101", "DP-LD-102", "DP-LD-104"]);
}

#[test]
fn a_sheet_drawn_again_mid_copy_keeps_the_copies_placed_by_their_objects_ids() {
    let mut t = tools();
    t.start(ToolKind::Copy, &[2]);
    t.input("1,1", None);
    t.input("@1,0", None);
    // An object landing before the device moves its place.
    let mut again = context();
    again.objects.insert(
        0,
        ObjectInfo {
            id: Some("DP-LD-101/new".to_owned()),
            kind: "line".to_owned(),
            ..ObjectInfo::default()
        },
    );
    t.load(again);
    assert_eq!(t.state(false).picked, [3]);
    assert_eq!(t.state(false).placed, 1);
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["target"], json!({"id": "DP-LD-101/device"}));
    // Another sheet drops the tool, with nothing sent.
    t.start(ToolKind::Copy, &[2]);
    t.input("1,1", None);
    t.input("@1,0", None);
    let mut other = context();
    other.sheet = "DP-LD-102".to_owned();
    t.load(other);
    assert_eq!(t.running(), None);
}

#[test]
fn copy_leaves_what_a_viewport_shows_as_move_does() {
    let mut t = tools();
    let mut c = context();
    c.objects[2].through_viewport = true;
    t.load(c);
    t.start(ToolKind::Copy, &[]);
    t.pick(Some(2));
    assert!(t
        .state(false)
        .error
        .unwrap()
        .contains("Copy copies only what is drawn on the paper here"));
    assert!(t.state(false).picked.is_empty());
}
