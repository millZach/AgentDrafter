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
        bounds: Some(match id {
            "border" => [0.0, 0.0, 17.0, 11.0],
            "title" => [5.0, 1.0, 7.0, 1.3],
            _ => [1.5, 6.3, 2.5, 6.7],
        }),
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
    for word in ["AR", "ar", "ARRAY", "arrayrect", "ARRAYPOLAR"] {
        assert_eq!(tool_for(word), Some(ToolKind::Array), "{word}");
    }
    // A path array is the core's, for the assistant: ARRAYPATH is no
    // tool's word.
    assert_eq!(tool_for("ARRAYPATH"), None);
    assert_eq!(array::kind_for("arrayrect"), Some(ArrayKind::Rectangular));
    assert_eq!(array::kind_for("ARRAYPOLAR"), Some(ArrayKind::Polar));
    assert_eq!(array::kind_for("AR"), None);
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
    assert_eq!(
        t.aim([3.0, 1.4], None).at,
        [3.0, 1.0],
        "Ortho holds it level"
    );
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
    assert_eq!(
        t.state(false).warning.unwrap().edit,
        "Copy border, 2 copies"
    );
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
fn copy_of_several_names_its_targets_and_another_tool_lands_the_copies_first_and_waits() {
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
    // Another tool lands the copies placed; as one is the template's,
    // they are asked about first, and Line waits.
    let outcome = t.input("L", None);
    assert_eq!(t.running(), None);
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

#[test]
fn another_tool_waits_while_the_copies_it_ended_are_asked_about() {
    let mut t = tools();
    // Copies of a template's object placed, then Circle picked: the copies
    // are given to be asked about, and Circle does not start, so nothing
    // can land before Change all or Cancel.
    t.start(ToolKind::Copy, &[0]);
    t.input("0,0", None);
    t.input("@1,0", None);
    let outcome = t.start(ToolKind::Circle, &[]);
    assert_eq!(outcome.sends.len(), 1);
    assert!(
        outcome.sends[0].template.is_some(),
        "the copies are asked about"
    );
    assert_eq!(
        outcome.log,
        ["CIRCLE not started: Change all or Cancel first"]
    );
    assert_eq!(t.running(), None, "no tool runs while it is asked");
    assert!(!t.takes_points());
    // An alias typed is the same, and so is Erase on objects selected,
    // which would otherwise erase them at once.
    t.start(ToolKind::Copy, &[0]);
    t.input("0,0", None);
    t.input("@1,0", None);
    let outcome = t.input("C", None);
    assert_eq!(outcome.sends.len(), 1);
    assert_eq!(t.running(), None);
    t.start(ToolKind::Copy, &[0]);
    t.input("0,0", None);
    t.input("@1,0", None);
    let outcome = t.start(ToolKind::Erase, &[2]);
    assert_eq!(outcome.sends.len(), 1, "nothing is erased: {outcome:?}");
    assert_eq!(operations(&outcome)[0]["op"], "copy");
    assert_eq!(t.running(), None);
    // A row's own object's copies are not asked about: the next tool starts.
    t.start(ToolKind::Copy, &[2]);
    t.input("0,0", None);
    t.input("@1,0", None);
    let outcome = t.start(ToolKind::Circle, &[]);
    assert!(outcome.sends[0].template.is_none());
    assert_eq!(t.running(), Some(ToolKind::Circle));
}

#[test]
fn leaving_drops_the_copies_placed_with_nothing_sent() {
    let mut t = tools();
    t.start(ToolKind::Copy, &[2]);
    t.input("0,0", None);
    t.input("@1,0", None);
    t.input("@2,0", None);
    let outcome = t.leave();
    assert!(outcome.sends.is_empty(), "nothing is sent");
    assert_eq!(
        outcome.log,
        ["COPY: 2 copies placed dropped, nothing changed"]
    );
    assert_eq!(t.running(), None);
    // With none placed, it ends.
    t.start(ToolKind::Copy, &[2]);
    let outcome = t.leave();
    assert!(outcome.sends.is_empty());
    assert_eq!(t.running(), None);
    // Any other tool ends as Esc ends it: Line keeps what it drew.
    t.start(ToolKind::Line, &[]);
    t.input("0,0", None);
    t.input("@1,0", None);
    assert_eq!(operations(&t.leave()).len(), 1);
    assert_eq!(t.running(), None);
}

/// The device, the row's own object at place 2, selected first, then AR:
/// Array's panel opens at once.
fn array_on_device(word: &str) -> Tools {
    let mut t = tools();
    let outcome = t.start_word(word, &[2]);
    assert!(outcome.sends.is_empty(), "nothing is sent as it opens");
    t
}

fn panel(t: &Tools, touch: bool) -> ArrayPanel {
    t.state(touch).array.expect("Array's panel")
}

fn field(t: &Tools, key: &str) -> String {
    panel(t, false)
        .fields
        .into_iter()
        .find(|f| f.key == key)
        .unwrap_or_else(|| panic!("no field {key}"))
        .value
}

#[test]
fn array_opens_its_panel_on_the_object_selected_first_rectangular_by_default() {
    let t = array_on_device("AR");
    assert_eq!(t.running(), Some(ToolKind::Array));
    let s = t.state(false);
    assert_eq!(s.command, Some("ARRAYRECT"));
    assert!(!s.takes_points && !s.selecting, "the panel takes no clicks");
    assert_eq!(s.picked, [2]);
    let p = s.array.unwrap();
    assert_eq!(p.kind, ArrayKind::Rectangular);
    assert_eq!(p.spacing, Spacing::Between);
    assert_eq!(p.of, "of device");
    // Three rows, a box and a little more apart downwards, to a quarter
    // of an inch: the device is 0.4 tall and 1 across.
    let keys: Vec<(&str, String, String)> = p
        .fields
        .iter()
        .map(|f| (f.key, f.label.clone(), f.value.clone()))
        .collect();
    assert_eq!(
        keys,
        [
            ("rows", "Rows".to_owned(), "3".to_owned()),
            ("columns", "Columns".to_owned(), "1".to_owned()),
            (
                "row_spacing",
                "Row spacing, in".to_owned(),
                "-0.75".to_owned()
            ),
            (
                "column_spacing",
                "Column spacing, in".to_owned(),
                "1.25".to_owned()
            ),
        ]
    );
    assert_eq!(p.count, "3 × 1: 2 copies");
    assert!(p.ready);
    assert_eq!(
        s.prompt,
        "3 rows × 1 column, rows 0.750 in apart down: 2 copies. Set it in the panel, or type \
         rows,columns or @column,row spacing"
    );
    let labels: Vec<&str> = s.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Array: 2 copies", "Cancel"]);
    assert_eq!(s.placeholder, "rows,columns  @dx,dy");
    // On a phone, the panel sits above the prompt.
    let phone = t.state(true);
    assert!(
        phone.prompt.ends_with(": 2 copies. Set it above"),
        "{}",
        phone.prompt
    );
    assert_eq!(phone.options[0].label, "Array");
}

#[test]
fn array_picks_objects_first_when_none_is_selected() {
    let mut t = tools();
    t.input("array", None);
    assert!(t.state(false).selecting);
    assert_eq!(
        t.state(false).prompt,
        "Click the objects to array, then Enter"
    );
    // An object with no ID cannot be named.
    t.pick(Some(3));
    assert!(t.state(false).error.unwrap().contains("no ID"));
    t.pick(Some(2));
    t.pick(Some(1));
    let out = t.input("", None);
    assert!(out.sends.is_empty());
    assert_eq!(out.log, ["ARRAYRECT"]);
    let p = panel(&t, false);
    assert_eq!(p.of, "of 2 objects");
    // Each object makes two copies.
    assert_eq!(p.count, "3 × 1: 4 copies");
}

#[test]
fn enter_lands_one_rectangular_array_with_the_spacing_between_items() {
    let mut t = array_on_device("AR");
    t.array_value("rows", "3");
    t.array_value("columns", "2");
    t.array_value("row_spacing", "-0.5");
    t.array_value("column_spacing", "1");
    // A field mid-typing that does not read as a number leaves it as it was.
    t.array_value("column_spacing", "-");
    assert_eq!(field(&t, "column_spacing"), "1");
    assert_eq!(panel(&t, false).count, "3 × 2: 5 copies");
    let out = t.input("", None);
    assert_eq!(t.running(), None);
    assert_eq!(out.sends.len(), 1);
    let send = &out.sends[0];
    assert_eq!(
        send.change_set,
        json!({"operations": [{
            "op": "array",
            "target": {"id": "DP-LD-101/device"},
            "kind": "rectangular",
            "rows": 3,
            "columns": 2,
            "row_spacing": -0.5,
            "column_spacing": 1.0
        }]})
    );
    assert_eq!(send.what, "Arrayed device, 3 × 2, 5 copies");
    assert_eq!(send.sheets, ["DP-LD-101"]);
    assert!(send.template.is_none(), "a row's own object lands at once");
    // One row: no row spacing is sent.
    let mut t = array_on_device("ARRAY");
    t.array_value("rows", "1");
    t.array_value("columns", "4");
    let op = &operations(&t.option("array"))[0];
    assert_eq!(op["rows"], json!(1));
    assert!(op.get("row_spacing").is_none());
    assert_eq!(op["column_spacing"], json!(1.25));
}

#[test]
fn a_total_held_is_divided_evenly_and_sent_as_the_spacing_between_items() {
    let mut t = array_on_device("AR");
    t.array_value("rows", "4");
    let out = t.option("total");
    assert_eq!(out.log, ["ARRAY: spacing as the total, divided evenly"]);
    let p = panel(&t, false);
    assert_eq!(p.spacing, Spacing::Total);
    let labels: Vec<&str> = p.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Rows",
            "Columns",
            "Rows, first to last, in",
            "Columns, first to last, in"
        ]
    );
    // The total follows what was between them: 3 gaps of 0.75.
    assert_eq!(field(&t, "row_total"), "-2.25");
    // 1.2 over 3 gaps is 0.4, not 0.39999999999999997.
    t.array_value("row_total", "-1.2");
    assert_eq!(
        t.state(false).prompt.split(':').next().unwrap(),
        "4 rows × 1 column, rows 0.400 in apart down"
    );
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["row_spacing"], json!(-0.4));
    // Two over three gaps: two thirds, cleared of noise past a ten-billionth.
    let mut t = array_on_device("AR");
    t.option("total");
    t.array_value("rows", "4");
    t.array_value("row_total", "2");
    assert_eq!(field(&t, "row_total"), "2");
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["row_spacing"], json!(0.6666666667));
    // Back to between items: the spacing worked out shows, to four places.
    let mut t = array_on_device("AR");
    t.option("total");
    t.array_value("rows", "4");
    t.array_value("row_total", "2");
    t.option("between");
    assert_eq!(field(&t, "row_spacing"), "0.6667");
    assert!(t.state(false).prompt.contains("rows 0.667 in apart up"));
    t.option("total");
    assert_eq!(field(&t, "row_total"), "2");
}

#[test]
fn the_command_bar_takes_rows_and_columns_spacings_and_the_total_typed() {
    let mut t = array_on_device("AR");
    let out = t.input("4,2", None);
    assert_eq!(out.log, ["ARRAY 4,2: 4 × 2"]);
    t.input("3x5", None);
    assert_eq!(panel(&t, false).count, "3 × 5: 14 copies");
    t.input("@1.5,-0.5", None);
    assert_eq!(field(&t, "column_spacing"), "1.5");
    assert_eq!(field(&t, "row_spacing"), "-0.5");
    // TO holds the total: @ then sets the distances first to last.
    t.input("TO", None);
    t.input("@6,-1", None);
    assert_eq!(field(&t, "column_total"), "6");
    assert_eq!(field(&t, "row_total"), "-1");
    t.input("be", None);
    assert_eq!(panel(&t, false).spacing, Spacing::Between);
    assert_eq!(field(&t, "row_spacing"), "-0.5");
    // Words it does not read say what it takes, and change nothing.
    t.input("lots", None);
    assert_eq!(
        t.state(false).error.as_deref(),
        Some("Type rows,columns, such as 4,2, or @column,row spacing, such as @1,-0.5.")
    );
    assert_eq!(panel(&t, false).count, "3 × 5: 14 copies");
    // Another tool's alias ends Array with nothing sent.
    let out = t.input("L", None);
    assert!(out.sends.is_empty());
    assert_eq!(t.running(), Some(ToolKind::Line));
}

#[test]
fn values_that_make_no_array_are_said_and_nothing_lands() {
    let mut t = array_on_device("AR");
    t.array_value("rows", "1");
    let p = panel(&t, false);
    assert!(!p.ready);
    assert_eq!(
        p.count,
        "One row and one column make no copy: give 2 or more of one."
    );
    let out = t.input("", None);
    assert!(out.sends.is_empty());
    assert_eq!(
        t.running(),
        Some(ToolKind::Array),
        "it stays to be put right"
    );
    assert_eq!(
        t.state(false).error.as_deref(),
        Some("One row and one column make no copy: give 2 or more of one.")
    );
    t.array_value("rows", "2.5");
    assert_eq!(
        panel(&t, false).count,
        "Rows and columns are whole numbers, 1 or more."
    );
    t.array_value("rows", "2");
    t.array_value("row_spacing", "0");
    assert_eq!(panel(&t, false).count, "Rows need a spacing other than 0.");
    t.array_value("row_spacing", "1");
    t.array_value("columns", "10001");
    assert_eq!(
        panel(&t, false).count,
        "One array makes at most 10,000 copies of each object."
    );
    // The tag over the copies says it too, and no copy is drawn.
    let (tag, _, problem) = t.array_tag(false).unwrap();
    assert!(problem && tag.starts_with("⚠ One array"), "{tag}");
    assert!(t.band(None).arrayed.is_none());
}

#[test]
fn esc_and_leaving_send_nothing_and_cancel_ends_it() {
    let mut t = array_on_device("AR");
    t.array_value("columns", "2");
    let out = t.escape();
    assert!(out.sends.is_empty());
    assert_eq!(out.log, ["ARRAY: cancelled, nothing sent"]);
    assert_eq!(t.running(), None);
    let mut t = array_on_device("AR");
    let out = t.leave();
    assert!(out.sends.is_empty());
    assert_eq!(t.running(), None);
    let mut t = array_on_device("AR");
    assert!(t.option("cancel").sends.is_empty());
    assert_eq!(t.running(), None);
    // Another sheet drops it, with nothing sent.
    let mut t = array_on_device("AR");
    let mut other = context();
    other.sheet = "DP-LD-102".to_owned();
    t.load(other);
    assert_eq!(t.running(), None);
}

#[test]
fn a_polar_array_takes_its_centre_on_a_snap_then_lands_with_a_snapped_point() {
    let mut t = array_on_device("ARRAYPOLAR");
    let s = t.state(false);
    assert_eq!(s.command, Some("ARRAYPOLAR"));
    assert!(s.takes_points, "a click takes the centre");
    assert_eq!(s.prompt, "Click the centre the items go round, or type x,y");
    assert_eq!(s.placeholder, "x,y");
    let p = s.array.unwrap();
    assert_eq!(p.kind, ArrayKind::Polar);
    assert!(p.picking && !p.ready);
    assert_eq!(p.center.as_deref(), Some("Click it on the drawing"));
    assert_eq!(
        t.state(true).array.unwrap().center.as_deref(),
        Some("Tap it on the drawing")
    );
    // Enter before the centre lands nothing.
    assert!(t.input("", None).sends.is_empty());
    assert!(t.state(false).error.unwrap().starts_with("Pick the centre"));
    // While the centre is picked, the copies follow the cursor, faint.
    let (_, places, following) = t.band(Some(&at(5.0, 5.0))).arrayed.unwrap();
    assert!(following);
    assert_eq!(places.len(), 5);
    assert_eq!(places[0].about, [5.0, 5.0]);
    // The centre on the title's centre snap.
    let out = t.point(snapped(6.0, 1.15, SnapKind::Center, 1, None));
    assert_eq!(out.log, ["ARRAY centre Centre of title, 6.0000, 1.1500"]);
    let s = t.state(false);
    assert!(!s.takes_points);
    assert_eq!(s.points, [[6.0, 1.15]]);
    let p = s.array.unwrap();
    assert_eq!(p.center.as_deref(), Some("Centre of title, 6.000, 1.150"));
    let keys: Vec<(&str, &str)> = p.fields.iter().map(|f| (f.key, f.label.as_str())).collect();
    assert_eq!(
        keys,
        [("items", "Items"), ("angle_between", "Angle between, °")]
    );
    assert_eq!(field(&t, "items"), "6");
    assert_eq!(field(&t, "angle_between"), "60");
    // Eight round a whole turn: 45 apart, the angle left out.
    t.array_value("items", "8");
    assert_eq!(field(&t, "angle_between"), "45");
    assert_eq!(panel(&t, false).count, "8 round: 7 copies");
    assert_eq!(
        t.state(false).prompt,
        "8 items round the centre, a whole turn, 45° apart: 7 copies. Set it in the panel, or \
         type items or items,angle"
    );
    let out = t.input("", None);
    let op = &operations(&out)[0];
    assert_eq!(
        *op,
        json!({
            "op": "array",
            "target": {"id": "DP-LD-101/device"},
            "kind": "polar",
            "center": {"snap": {"id": "DP-LD-101/title", "snap": "center"}},
            "count": 8
        })
    );
    assert_eq!(out.sends[0].what, "Arrayed device, 8 round, 7 copies");
}

#[test]
fn a_whole_turn_stays_whole_as_the_items_change_and_less_sends_its_angle() {
    let mut t = array_on_device("AR");
    let out = t.option("polar");
    assert_eq!(out.log, ["ARRAYPOLAR"]);
    t.input("3,8.8", None);
    assert_eq!(t.state(false).points, [[3.0, 8.8]]);
    t.array_value("items", "7");
    // 360 over 7, shown to four places, still a whole turn.
    assert_eq!(field(&t, "angle_between"), "51.4286");
    let op = &operations(&t.clone().input("", None))[0];
    assert!(op.get("angle").is_none(), "{op}");
    assert_eq!(op["center"], json!([3.0, 8.8, 0]));
    // An angle between typed that falls short of a turn is sent as the
    // angle it fills: 5 items 45° apart fill 180°.
    t.input("5,45", None);
    assert_eq!(panel(&t, false).count, "5 round, 180°: 4 copies");
    let op = &operations(&t.clone().input("", None))[0];
    assert_eq!(op["angle"], json!(180.0));
    // More than a turn is said.
    t.array_value("angle_between", "100");
    assert_eq!(
        panel(&t, false).count,
        "5 items 100° apart fill 400°: more than a whole turn."
    );
    // The angle to fill held: clockwise half a turn, unturned items.
    t.option("total");
    t.array_value("angle_fill", "-180");
    assert_eq!(field(&t, "angle_fill"), "-180");
    t.option("no-turn");
    assert_eq!(panel(&t, false).count, "5 round, -180°: 4 copies");
    assert!(t
        .state(false)
        .prompt
        .contains("180° clockwise, 45° apart, not turned"));
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["angle"], json!(-180.0));
    assert_eq!(op["count"], json!(5));
    assert_eq!(op["rotate"], json!(false));
}

#[test]
fn the_copies_shown_go_where_the_core_puts_them() {
    // Rectangular: shifted by the spacings, row by row.
    let mut t = array_on_device("AR");
    t.input("2,2", None);
    t.input("@1,-0.5", None);
    let (picked, places, following) = t.band(None).arrayed.unwrap();
    assert_eq!(picked, [2]);
    assert!(!following);
    let by: Vec<[f64; 2]> = places.iter().map(|p| p.apply([0.0, 0.0])).collect();
    assert_eq!(by, [[1.0, 0.0], [0.0, -0.5], [1.0, -0.5]]);
    // Polar, turned: the copy a quarter turn round turns about the centre.
    let mut t = array_on_device("ARRAYPOLAR");
    t.point(at(2.0, 4.5));
    t.array_value("items", "4");
    let (_, places, _) = t.band(None).arrayed.unwrap();
    let p = places[0].apply([2.0, 6.5]);
    assert!(
        (p[0] - 0.0).abs() < 1e-12 && (p[1] - 4.5).abs() < 1e-12,
        "{p:?}"
    );
    assert_eq!(places[0].turn, 90.0);
    // Unturned: the box's centre goes round, as the core's base point.
    t.option("no-turn");
    let (_, places, _) = t.band(None).arrayed.unwrap();
    assert_eq!(places[0].turn, 0.0);
    let middle = places[0].apply([2.0, 6.5]);
    assert!(
        (middle[0] - 0.0).abs() < 1e-12 && (middle[1] - 4.5).abs() < 1e-12,
        "{middle:?}"
    );
}

#[test]
fn the_tag_and_the_box_round_the_copies() {
    let mut t = array_on_device("AR");
    t.input("3,2", None);
    t.input("@1,-0.5", None);
    // The device is [1.5, 6.3, 2.5, 6.7]; the copies run 1 right and 1 down.
    assert_eq!(t.array_extent(None), Some([1.5, 5.3, 3.5, 6.7]));
    let (tag, top_right, problem) = t.array_tag(false).unwrap();
    assert_eq!(
        (tag.as_str(), top_right, problem),
        ("3 × 2 · waits for Enter", [3.5, 6.7], false)
    );
    assert_eq!(t.array_tag(true).unwrap().0, "3 × 2");
    // A polar array's tag waits for its centre.
    let mut t = array_on_device("ARRAYPOLAR");
    assert!(t.array_tag(false).is_none());
    t.point(at(2.0, 4.0));
    t.option("total");
    t.array_value("items", "5");
    t.array_value("angle_fill", "-180");
    assert_eq!(
        t.array_tag(false).unwrap().0,
        "5 round, -180° · waits for Enter"
    );
}

#[test]
fn arraying_a_templates_object_warns_and_asks_before_it_lands() {
    let mut t = tools();
    t.start_word("AR", &[1]);
    let w = t.state(false).warning.unwrap();
    assert_eq!(w.title, "This is part of the template.");
    assert_eq!(
        w.detail,
        "This arrays it on every sheet built from this template: 3 sheets. You will be asked \
         before it lands."
    );
    let phone = t.state(true).warning.unwrap();
    assert_eq!(phone.title, "Part of the template:");
    assert_eq!(
        phone.detail,
        "the copies land on all 3 sheets. You will be asked first."
    );
    t.input("2,2", None);
    let out = t.input("", None);
    let send = &out.sends[0];
    let asked = send.template.as_ref().expect("asked first");
    assert_eq!(asked.edit, "Array title, 2 × 2");
    assert_eq!(asked.sheets.len(), 3);
    assert_eq!(send.sheets.len(), 3);
    // A polar array of a template's object round a row's own object's
    // centre sends the centre as coordinates, the same on every sheet.
    let mut t = tools();
    t.start_word("ARRAYPOLAR", &[1]);
    t.point(snapped(2.0, 6.5, SnapKind::Center, 2, None));
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["center"], json!([2.0, 6.5, 0]));
}

#[test]
fn array_follows_its_picks_and_centre_through_a_sheet_drawn_again() {
    let mut t = array_on_device("ARRAYPOLAR");
    t.point(snapped(6.0, 1.15, SnapKind::Center, 1, None));
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
    let op = &operations(&t.input("", None))[0];
    assert_eq!(op["target"], json!({"id": "DP-LD-101/device"}));
    assert_eq!(
        op["center"],
        json!({"snap": {"id": "DP-LD-101/title", "snap": "center"}})
    );
    // Pick, again: the next click takes a new centre.
    let mut t = array_on_device("ARRAYPOLAR");
    t.point(at(1.0, 1.0));
    t.option("pick-center");
    assert!(t.takes_points());
    t.point(at(2.0, 2.0));
    assert_eq!(t.state(false).points, [[2.0, 2.0]]);
}

#[test]
fn array_leaves_what_a_viewport_shows_as_copy_does() {
    let mut context = context();
    context.objects[2].through_viewport = true;
    let mut t = Tools::default();
    t.load(context);
    t.start_word("AR", &[2]);
    assert!(t
        .state(false)
        .error
        .unwrap()
        .contains("Array arrays only what is drawn"));
    assert!(t.state(false).selecting, "it picks objects instead");
}
