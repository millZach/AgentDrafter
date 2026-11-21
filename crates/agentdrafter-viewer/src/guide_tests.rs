//! A guide's step as the drafter does it, natively: what it wants next, its
//! ring, the notes for a point or a pick the step does not want, and taking
//! a point back.

use super::*;
use crate::tools::{Context, ObjectInfo, Reach, SnapRef, Taken};
use agentdrafter_core::SnapKind;
use serde_json::json;

/// A made-up loop sheet: a device, two terminals and a wire a guide's
/// earlier step drew.
fn tools() -> Tools {
    let object = |id: &str, kind: &str, name: Option<&str>| ObjectInfo {
        id: Some(format!("DP-LD-101/{id}")),
        name: name.map(str::to_owned),
        kind: kind.to_owned(),
        block: None,
        layer: "0".to_owned(),
        values: None,
        from_template: Some(false),
        through_viewport: false,
    };
    let mut tools = Tools::default();
    tools.load(Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects: vec![
            object("device", "insert", Some("FT-101")),
            object("terminal-1", "insert", None),
            object("terminal-2", "insert", None),
            object("line-1", "line", None),
            object("text-1", "text", None),
        ],
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec!["DP-LD-101".to_owned()],
        }),
        past: false,
        space: None,
        layout: None,
    });
    tools
}

fn line_step() -> StepPlan {
    serde_json::from_value(json!({
        "tool": "line",
        "points": [
            {"label": "FT-101's + terminal", "at": [3.2, 6.75, 0.0]},
            {"label": "TB1-1", "at": [7.2, 6.75, 0.0]}
        ],
        "operation": {"op": "add_line"},
        "step": 2, "say": "Draw the wire."
    }))
    .unwrap()
}

fn move_step() -> StepPlan {
    serde_json::from_value(json!({
        "tool": "move",
        "points": [
            {"label": "101+", "object": "DP-LD-101/text-1"},
            {"label": "base point", "at": [5.2, 6.75, 0.0]}
        ]
    }))
    .unwrap()
}

fn on(x: f64, y: f64, snap: SnapKind, object: usize) -> Taken {
    Taken {
        at: [x, y],
        z: 0.0,
        snap: Some(SnapRef {
            snap,
            object,
            which: None,
            with: None,
        }),
        typed: false,
    }
}

/// Takes `point` as `web` does: the phase and the points before, then the
/// tool's point, then the note.
fn click(tools: &mut Tools, step: &StepPlan, point: Taken) -> Option<Note> {
    let before = phase(step, tools);
    let count = tools.state(false).points.len();
    tools.point(point.clone());
    after_point(step, before, tools, &point, count)
}

#[test]
fn before_its_tool_starts_the_step_rings_its_first_point() {
    let tools = tools();
    let step = line_step();
    assert_eq!(phase(&step, &tools), Phase::Start);
    let ring = ring(&step, &tools).unwrap();
    assert_eq!(ring.point, 0);
    assert_eq!(ring.at, Some([3.2, 6.75]));
    assert_eq!(ring.label, "FT-101's + terminal");
}

#[test]
fn another_tool_running_is_the_step_waiting_for_its_own() {
    let mut tools = tools();
    tools.start(ToolKind::Rectangle, &[]);
    assert_eq!(phase(&line_step(), &tools), Phase::Start);
}

#[test]
fn each_point_taken_moves_the_ring_on_and_the_last_wants_enter() {
    let mut tools = tools();
    let step = line_step();
    tools.start(ToolKind::Line, &[]);
    assert_eq!(phase(&step, &tools), Phase::Click(0));
    let note = click(&mut tools, &step, on(3.2, 6.75, SnapKind::Center, 0));
    assert_eq!(note, None, "the step's own point");
    assert_eq!(phase(&step, &tools), Phase::Click(1));
    assert_eq!(ring(&step, &tools).unwrap().label, "TB1-1");
    assert_eq!(
        click(&mut tools, &step, on(7.2, 6.75, SnapKind::Midpoint, 1)),
        None
    );
    assert_eq!(phase(&step, &tools), Phase::Finish);
    assert_eq!(ring(&step, &tools), None);
    // A third point would draw on past the step's end.
    let note = click(&mut tools, &step, on(9.0, 6.75, SnapKind::Endpoint, 3)).unwrap();
    assert_eq!(
        note.words,
        "That would draw on past TB1-1; the step ends there."
    );
    assert_eq!(note.take_back, "point");
}

#[test]
fn a_point_off_the_ring_says_where_it_went_and_can_be_taken_back() {
    let mut tools = tools();
    let step = line_step();
    tools.start(ToolKind::Line, &[]);
    click(&mut tools, &step, on(3.2, 6.75, SnapKind::Center, 0));
    let note = click(&mut tools, &step, on(7.2, 6.25, SnapKind::Midpoint, 2)).unwrap();
    assert_eq!(
        note.words,
        "That went to the midpoint of terminal-2; the step wants TB1-1."
    );
    // Undo it: the point goes, and the step wants TB1-1 again.
    assert!(tools.take_back_point());
    assert_eq!(phase(&step, &tools), Phase::Click(1));
    // A point on no snap is said by its coordinates; within a millionth of
    // the ring it is the step's.
    let note = click(&mut tools, &step, Taken::typed([7.0, 6.0], 0.0)).unwrap();
    assert_eq!(
        note.words,
        "That went to 7.0000, 6.0000; the step wants TB1-1."
    );
    tools.take_back_point();
    assert_eq!(
        click(&mut tools, &step, Taken::typed([7.2000001, 6.75], 0.0)),
        None
    );
}

#[test]
fn a_move_step_rings_its_object_to_pick_then_its_base_point() {
    let mut tools = tools();
    let step = move_step();
    let ring_now = ring(&step, &tools).unwrap();
    assert_eq!((ring_now.object, ring_now.at), (Some(4), None));
    tools.start(ToolKind::Move, &[]);
    assert_eq!(phase(&step, &tools), Phase::Pick(0));

    // Another object picked: said, and taken out again when asked.
    let before = phase(&step, &tools);
    tools.pick(Some(2));
    let note = after_pick(&step, before, &tools, 2).unwrap();
    assert_eq!(note.words, "That picked terminal-2; the step wants 101+.");
    assert_eq!((note.take_back, note.place), ("pick", Some(2)));
    tools.pick(Some(2));
    assert!(tools.state(false).picked.is_empty());

    let before = phase(&step, &tools);
    tools.pick(Some(4));
    assert_eq!(after_pick(&step, before, &tools, 4), None);
    assert_eq!(phase(&step, &tools), Phase::Finish, "picked: Enter next");
    tools.input("", None);
    assert_eq!(phase(&step, &tools), Phase::Click(1));
    assert_eq!(ring(&step, &tools).unwrap().at, Some([5.2, 6.75]));
    assert_eq!(
        click(&mut tools, &step, Taken::typed([5.2, 6.75], 0.0)),
        None
    );
    assert_eq!(phase(&step, &tools), Phase::Finish, "then where it goes");
}

#[test]
fn a_point_waiting_on_an_earlier_step_has_no_ring() {
    let tools = tools();
    let step: StepPlan = serde_json::from_value(json!({
        "tool": "text",
        "points": [{"label": "the wire's middle", "waits_on": 2}]
    }))
    .unwrap();
    assert_eq!(ring(&step, &tools), None);
}

#[test]
fn a_text_step_gives_the_tool_its_height_and_wants_typing_once_placed() {
    let mut tools = tools();
    let step: StepPlan = serde_json::from_value(json!({
        "tool": "text",
        "points": [{"label": "the wire's middle", "at": [5.2, 6.75, 0.0]}],
        "operation": {"op": "add_text", "height": 0.08, "contents": "101+"}
    }))
    .unwrap();
    assert_eq!(text_height(&step), Some(0.08));
    assert_eq!(text_height(&line_step()), None);
    tools.set_text_height(0.08);
    assert_eq!(tools.text_height(), 0.08);
    tools.start(ToolKind::Text, &[]);
    assert_eq!(
        click(&mut tools, &step, Taken::typed([5.2, 6.75], 0.0)),
        None
    );
    assert_eq!(phase(&step, &tools), Phase::Type);
    // Taken back before a word is typed, the text starts again.
    assert!(tools.take_back_point());
    assert_eq!(phase(&step, &tools), Phase::Click(0));
}

#[test]
fn a_setting_step_has_no_tool_and_no_ring() {
    let tools = tools();
    let step: StepPlan =
        serde_json::from_value(json!({"setting": {"current_layer": "wire"}})).unwrap();
    assert_eq!(phase(&step, &tools), Phase::Start);
    assert_eq!(ring(&step, &tools), None);
}
