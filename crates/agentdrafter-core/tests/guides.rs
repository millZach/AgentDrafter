//! Guides (#157): `guide` checks a guide on the set as it is and records it
//! beside the journal, writing nothing else; `guides` lists each with its
//! steps worked out on the set as it stands; a change set sent for a step
//! names it under `guide`, which marks its journal entry and checks it
//! against the step's operation.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, end_guide, guide_with, guides_with, journal, propose_with, undo,
    ChangeReport, ChangeRequest, EndGuideRequest, ErrorCode, GuideChecked, GuideRequest,
    GuidesReport, GuidesRequest, Interface, JournalRequest, ProposeRequest, Reply, SetBuildRequest,
    Settings, UndoRequest, GUIDES_FOLDER, JOURNAL_FILE,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;

/// A built copy of the demo set in a folder of its own.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-guides")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    set
}

fn corner() -> Value {
    json!({"snap": {"id": "DEMO-002/border", "snap": "endpoint", "which": "points[0]"}})
}

fn side() -> Value {
    json!({"snap": {"id": "DEMO-002/border", "snap": "midpoint", "which": "segments[1]"}})
}

/// A guide of three steps on DEMO-002: a setting, a line, and a move of
/// the line the second step draws.
fn a_guide() -> Value {
    json!({
        "title": "Draw a line from the border's corner",
        "sheet": "demo-002",
        "steps": [
            {"say": "Make notes the current layer.", "setting": {"current_layer": "notes"}},
            {"say": "Draw a line from the corner to the side's middle.", "title": "Draw the line",
             "tool": "line",
             "points": [{"at": corner(), "label": "the corner"}, {"at": side(), "label": "the side's middle"}],
             "completes": {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": corner(), "to": side()}},
            {"say": "Move the line 0.5 up.", "tool": "move",
             "points": [{"object": {"made_by": 2}, "label": "the line"},
                        {"at": {"snap": {"made_by": 2, "snap": "midpoint"}}, "label": "its middle"}],
             "completes": {"op": "move", "target": {"made_by": 2}, "by": [0, 0.5, 0]}}
        ]
    })
}

fn give(set: &Path, guide: Value) -> Result<u64, agentdrafter_core::Error> {
    guide_with(
        &GuideRequest {
            set: set.to_owned(),
            guide,
            by: Some("Assistant".to_owned()),
        },
        &Settings::default(),
        Interface::Mcp,
    )
    .map(|r| r.guide)
}

fn listed(set: &Path) -> GuidesReport {
    guides_with(
        &GuidesRequest {
            set: set.to_owned(),
        },
        &Settings::default(),
    )
    .unwrap()
}

fn change(set: &Path, change_set: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    )
}

/// Every file of the set but its guides, with its bytes.
fn files(set: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![set.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.ends_with("guides") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_guide_is_checked_recorded_beside_the_journal_and_listed_with_its_points() {
    let set = demo_set("recorded");
    let before = files(&set);
    assert_eq!(give(&set, a_guide()).unwrap(), 1);
    assert_eq!(files(&set), before, "a guide changes no file of the set");
    assert!(!set.join(JOURNAL_FILE).exists(), "nor writes the journal");
    assert!(set.join(GUIDES_FOLDER).join("1.json").is_file());

    let report = listed(&set);
    let guide = &report.guides[0];
    assert_eq!((guide.guide, guide.by.as_str()), (1, "Assistant"));
    assert_eq!(guide.sheet, "DEMO-002", "named as the set names it");
    let steps = &guide.steps;
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].setting.as_ref().unwrap().current_layer, "notes");
    assert_eq!(steps[0].title, "Make notes the current layer.");
    assert_eq!(steps[1].title, "Draw the line");
    let corner = &steps[1].points[0];
    assert_eq!(corner.label.as_deref(), Some("the corner"));
    assert_eq!(corner.at, Some([0.5, 0.5, 0.0]));
    assert_eq!(corner.snap.as_ref().unwrap().id, "DEMO-002/border");
    assert!(steps[1].operation.is_some());
    // The third step names what the second makes, which nothing has yet.
    assert_eq!(steps[2].needs, vec![2]);
    assert_eq!(steps[2].points[0].waits_on, Some(2));
    assert!(steps[2].operation.is_none());
    assert!(steps[2].landed.is_empty());
}

#[test]
fn a_guide_the_set_would_refuse_is_not_recorded() {
    let set = demo_set("refused");
    let wrong = |edit: &dyn Fn(&mut Value), code: ErrorCode, field: &str| {
        let mut guide = a_guide();
        edit(&mut guide);
        let error = give(&set, guide).unwrap_err();
        assert_eq!(error.code, code, "{}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
    };
    wrong(
        &|g| g["sheet"] = json!("DEMO-009"),
        ErrorCode::UnknownName,
        "guide.sheet",
    );
    wrong(
        &|g| g["steps"][0]["setting"]["current_layer"] = json!("wires"),
        ErrorCode::UnknownName,
        "guide.steps[0].setting.current_layer",
    );
    wrong(
        &|g| g["steps"][1]["points"][1]["at"]["snap"]["which"] = json!("segments[9]"),
        ErrorCode::UnknownName,
        "guide.steps[1].points[1].at.snap.which",
    );
    wrong(
        &|g| g["steps"][1]["completes"]["layer"] = json!("wires"),
        ErrorCode::UnknownName,
        "guide.steps[1].completes.layer",
    );
    wrong(
        &|g| g["steps"][1]["tool"] = json!("circle"),
        ErrorCode::BadValue,
        "guide.steps[1].completes.op",
    );
    wrong(
        &|g| g["steps"][1]["tool"] = json!("pencil"),
        ErrorCode::UnknownName,
        "guide.steps[1].tool",
    );
    wrong(
        &|g| g["steps"][2]["completes"]["target"] = json!({"made_by": 3}),
        ErrorCode::BadValue,
        "guide.steps[2].completes.target.made_by",
    );
    wrong(
        &|g| g["steps"][2]["completes"]["target"] = json!({"made_by": 1}),
        ErrorCode::BadValue,
        "guide.steps[2].completes.target.made_by",
    );
    wrong(
        &|g| g["steps"][0]["completes"] = json!({"op": "delete"}),
        ErrorCode::Usage,
        "guide.steps[0]",
    );
    wrong(
        &|g| g["steps"] = json!([]),
        ErrorCode::BadValue,
        "guide.steps",
    );
    // A key of the wrong kind inside a step is refused at its own key.
    wrong(
        &|g| g["steps"][1]["points"][0]["label"] = json!(5),
        ErrorCode::WrongType,
        "guide.steps[1].points[0].label",
    );
    wrong(
        &|g| g["steps"][1]["colour"] = json!("red"),
        ErrorCode::UnknownKey,
        "guide.steps[1].colour",
    );
    wrong(
        &|g| g["steps"][0]["setting"] = json!({}),
        ErrorCode::MissingKey,
        "guide.steps[0].setting.current_layer",
    );
    wrong(
        &|g| g["steps"][1]["points"][0]["at"] = json!("corner"),
        ErrorCode::WrongType,
        "guide.steps[1].points[0].at",
    );
    // Rings that are not the operation's points: the drafter who clicks
    // them would draw another line.
    wrong(
        &|g| g["steps"][1]["points"][1]["at"] = json!([2, 1]),
        ErrorCode::BadValue,
        "guide.steps[1].points",
    );
    // An object to pick that is not the one the operation moves.
    wrong(
        &|g| g["steps"][2]["points"][0]["object"] = json!({"id": "DEMO-002/heading"}),
        ErrorCode::BadValue,
        "guide.steps[2].points",
    );
    assert!(!set.join(GUIDES_FOLDER).exists(), "nothing was recorded");
}

#[test]
fn a_change_set_sent_for_a_step_is_marked_and_checked_against_it() {
    let set = demo_set("checked");
    give(&set, a_guide()).unwrap();

    // To the wrong place: it lands, as the drafter's own edit, and says so.
    let wrong = change(
        &set,
        json!({"guide": {"guide": 1, "step": 2}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": corner(), "to": [3, 3, 0]}]}),
    );
    assert!(wrong.ok, "{:#?}", wrong.error);
    let checked = wrong.result.as_ref().unwrap().guide.clone().unwrap();
    assert!(!checked.completes);
    assert_eq!((checked.step, checked.of), (2, 3));
    assert!(
        checked.message.contains("`to` went to [3, 3, 0]")
            && checked.message.contains("the step wants the midpoint"),
        "{}",
        checked.message
    );
    let entry = wrong.result.unwrap().journal_entry.unwrap();
    undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });

    // Drawn the other way round, its end as coordinates: it completes.
    let right = change(
        &set,
        json!({"guide": {"guide": 1, "step": 2}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": side(), "to": [0.5, 0.5, 0]}]}),
    );
    let report = right.result.unwrap();
    let checked = report.guide.unwrap();
    assert!(checked.completes, "{}", checked.message);
    let made = checked.made.unwrap();

    // The journal marks it, with the step and how many there are.
    let journal = journal(&JournalRequest { set: set.clone() })
        .result
        .unwrap();
    let marked: Vec<_> = journal
        .entries
        .iter()
        .filter_map(|e| e.recorded.guide.map(|m| (e.recorded.entry, m.step, m.of)))
        .collect();
    assert_eq!(marked, vec![(1, 2, 3), (3, 2, 3)]);

    // The listing moves on: the undone entry is passed over, and the third
    // step's points and operation name the line the second made.
    let report = listed(&set);
    let steps = &report.guides[0].steps;
    assert_eq!(steps[1].landed, vec![3]);
    assert_eq!(steps[1].made.as_deref(), Some(made.as_str()));
    assert_eq!(steps[2].points[0].object.as_deref(), Some(made.as_str()));
    assert!(steps[2].points[1].at.is_some());
    let op = steps[2].operation.clone().unwrap();
    assert_eq!(op["target"], json!({"id": made}));

    // Done for the drafter, as the page's Do it for me sends it.
    let done = change(
        &set,
        json!({"guide": {"guide": 1, "step": 3, "of": 3}, "operations": [op]}),
    );
    assert!(done.result.unwrap().guide.unwrap().completes);

    // Another operation than the step's says which it wants.
    let other = change(
        &set,
        json!({"guide": {"guide": 1, "step": 3}, "operations": [
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5}]}),
    );
    let checked = other.result.unwrap().guide.unwrap();
    assert!(!checked.completes);
    assert_eq!(
        checked.message,
        "That was `add_circle`; the step wants `move`."
    );
}

#[test]
fn a_change_set_naming_no_step_of_a_guide_is_refused_whole() {
    let set = demo_set("marks");
    give(&set, a_guide()).unwrap();
    let line = json!({"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [2, 2, 0]});
    let refused = |mark: Value, code: ErrorCode, field: &str| {
        let reply = change(&set, json!({"guide": mark, "operations": [line.clone()]}));
        let error = reply.error.unwrap();
        assert_eq!(error.code, code, "{}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
    };
    refused(
        json!({"guide": 2, "step": 1}),
        ErrorCode::UnknownName,
        "guide.guide",
    );
    refused(
        json!({"guide": 1, "step": 4}),
        ErrorCode::BadValue,
        "guide.step",
    );
    refused(
        json!({"guide": 1, "step": 1}),
        ErrorCode::BadValue,
        "guide.step",
    );
    refused(
        json!({"guide": 1, "step": 2, "of": 5}),
        ErrorCode::BadValue,
        "guide.of",
    );
    refused(json!({"guide": 1}), ErrorCode::MissingKey, "guide.step");
    refused(
        json!({"guide": 1, "step": 2, "by": "me"}),
        ErrorCode::UnknownKey,
        "guide.by",
    );
    refused(json!(3), ErrorCode::WrongType, "guide");
    assert!(
        !set.join(JOURNAL_FILE).exists(),
        "a change set refused whole before it was read is not an attempt"
    );
}

/// Lands `change_set` for a step and gives the core's check and its entry.
fn landed(set: &Path, change_set: Value) -> (GuideChecked, u64) {
    let reply = change(set, change_set);
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    (report.guide.unwrap(), report.journal_entry.unwrap())
}

fn undone(set: &Path, entry: u64) {
    let reply = undo(&UndoRequest {
        set: set.to_owned(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(reply.ok, "{:#?}", reply.error);
}

#[test]
fn only_the_change_set_that_completes_a_step_names_what_it_made() {
    let set = demo_set("completes");
    give(&set, a_guide()).unwrap();
    let line = |to: Value| {
        json!({"guide": {"guide": 1, "step": 2}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": corner(), "to": to}]})
    };
    // A line to the wrong place lands, marked for the step, but is not the
    // step's: the step after it still waits.
    let (wrong, wrong_entry) = landed(&set, line(json!([3, 3, 0])));
    assert!(!wrong.completes);
    let report = listed(&set);
    let steps = &report.guides[0].steps;
    assert_eq!(steps[1].landed, vec![wrong_entry]);
    assert_eq!(steps[1].completed_by, None);
    assert_eq!(steps[1].made, None);
    assert_eq!(steps[2].points[0].waits_on, Some(2));
    assert!(steps[2].operation.is_none());

    // The journal records the core's check with the mark.
    let marks = || -> Vec<(u64, Option<bool>)> {
        journal(&JournalRequest { set: set.clone() })
            .result
            .unwrap()
            .entries
            .iter()
            .filter_map(|e| e.recorded.guide.map(|m| (e.recorded.entry, m.completes)))
            .collect()
    };
    assert_eq!(marks(), vec![(wrong_entry, Some(false))]);

    // The right line, left beside the wrong one, is the step's.
    let (right, right_entry) = landed(&set, line(side()));
    assert!(right.completes, "{}", right.message);
    let report = listed(&set);
    let steps = &report.guides[0].steps;
    assert_eq!(steps[1].landed, vec![wrong_entry, right_entry]);
    assert_eq!(steps[1].completed_by, Some(right_entry));
    assert_eq!(steps[1].made, right.made);
    assert_eq!(steps[2].points[0].object, right.made);
    assert_eq!(
        marks(),
        vec![(wrong_entry, Some(false)), (right_entry, Some(true))]
    );

    // Undone, the step is not done: the listing steps back.
    undone(&set, right_entry);
    let report = listed(&set);
    assert_eq!(report.newest_entry, right_entry + 1, "as of the undo");
    let steps = &report.guides[0].steps;
    assert_eq!(steps[1].completed_by, None);
    assert_eq!(steps[1].made, None);
    assert_eq!(steps[2].points[0].waits_on, Some(2));

    // Whether a change set completes its step is the core's to say.
    let error = change(
        &set,
        json!({"guide": {"guide": 1, "step": 2, "completes": true}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [2, 2, 0]}]}),
    )
    .error
    .unwrap();
    assert_eq!(error.code, ErrorCode::UnknownKey, "{}", error.message);
    assert_eq!(error.field.as_deref(), Some("guide.completes"));
}

#[test]
fn a_step_is_completed_on_its_sheet_by_its_objects_and_refuses_explicit_id() {
    let set = demo_set("where");
    // An add_* step meant to be drawn by hand refuses an explicit `id`.
    let with_id = give(
        &set,
        json!({"title": "Rule a line", "sheet": "DEMO-002", "steps": [
            {"say": "Draw the rule.", "tool": "line", "points": [{"at": [1, 1]}, {"at": [2, 1]}],
             "completes": {"op": "add_line", "sheet": "DEMO-002", "id": "rule", "from": [1, 1, 0], "to": [2, 1, 0]}}
        ]}),
    )
    .unwrap_err();
    assert_eq!(with_id.code, ErrorCode::BadValue, "{}", with_id.message);
    assert_eq!(
        with_id.field.as_deref(),
        Some("guide.steps[0].completes.id")
    );

    give(
        &set,
        json!({"title": "Rule a line and copy the heading", "sheet": "DEMO-002", "steps": [
            {"say": "Draw the rule.", "tool": "line", "points": [{"at": [1, 1]}, {"at": [2, 1]}],
             "completes": {"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [2, 1, 0]}},
            {"say": "Copy the heading 1 down.", "tool": "copy",
             "points": [{"object": {"id": "DEMO-002/heading"}}, {"at": [1, 10]}, {"at": [1, 9]}],
             "completes": {"op": "copy", "target": {"id": "DEMO-002/heading"}, "by": [0, -1, 0]}}
        ]}),
    )
    .unwrap();
    let step = |n: u64, op: Value| json!({"guide": {"guide": 1, "step": n}, "operations": [op]});
    let line =
        |sheet: &str| json!({"op": "add_line", "sheet": sheet, "from": [1, 1, 0], "to": [2, 1, 0]});
    let tried = |change_set: Value| {
        let (checked, entry) = landed(&set, change_set);
        undone(&set, entry);
        checked
    };

    let elsewhere = tried(step(1, line("DEMO-003")));
    assert!(!elsewhere.completes);
    assert_eq!(
        elsewhere.message,
        "That landed on `DEMO-003`; the step wants `DEMO-002`."
    );
    assert_eq!(
        elsewhere.made, None,
        "a wrong-sheet add must not rewrite `made` to the guide's sheet"
    );
    assert!(tried(step(1, line("DEMO-002"))).completes);

    let copy = |id: &str| json!({"op": "copy", "target": {"id": id}, "by": [0, -1, 0]});
    let border = tried(step(2, copy("DEMO-002/border")));
    assert!(!border.completes);
    assert_eq!(
        border.message,
        "That went to `DEMO-002/border`; the step wants `DEMO-002/heading`."
    );
    let heading = tried(step(2, copy("DEMO-002/heading")));
    assert!(heading.completes, "{}", heading.message);
    assert!(heading.made.unwrap().starts_with("DEMO-002/"));
}

#[test]
fn text_matches_exactly_and_a_rectangle_by_its_vertices_in_order() {
    let set = demo_set("exact");
    let rectangle = json!([[1, 1, 0], [3, 1, 0], [3, 2, 0], [1, 2, 0]]);
    give(
        &set,
        json!({"title": "A box and its name", "sheet": "DEMO-002", "steps": [
            {"say": "Draw the box.", "tool": "rectangle", "points": [{"at": [1, 1]}, {"at": [3, 2]}],
             "completes": {"op": "add_polyline", "sheet": "DEMO-002", "points": rectangle, "closed": true}},
            {"say": "Name it.", "tool": "text", "points": [{"at": [1.2, 1.4]}],
             "completes": {"op": "add_text", "sheet": "DEMO-002", "contents": "Pump A", "height": 0.1, "position": [1.2, 1.4, 0]}}
        ]}),
    )
    .unwrap();
    let tried = |op: Value, n: u64| {
        let (checked, entry) = landed(
            &set,
            json!({"guide": {"guide": 1, "step": n}, "operations": [op]}),
        );
        undone(&set, entry);
        checked
    };
    let polyline = |points: Value| json!({"op": "add_polyline", "sheet": "DEMO-002", "points": points, "closed": true});
    // The same four corners crossed, a bow-tie.
    let bow_tie = tried(
        polyline(json!([[1, 1, 0], [3, 2, 0], [3, 1, 0], [1, 2, 0]])),
        1,
    );
    assert!(!bow_tie.completes, "{}", bow_tie.message);
    assert!(bow_tie.message.contains("`points`"), "{}", bow_tie.message);
    // The rectangle drawn the other way round, from another corner.
    let reversed = tried(
        polyline(json!([[3, 2, 0], [3, 1, 0], [1, 1, 0], [1, 2, 0]])),
        1,
    );
    assert!(reversed.completes, "{}", reversed.message);
    // The same corners with one side an arc, given by its angle: a curve.
    let arced = tried(
        polyline(json!([[1, 1, 0], {"point": [3, 1, 0], "arc_angle": 90}, [3, 2, 0], [1, 2, 0]])),
        1,
    );
    assert!(!arced.completes, "{}", arced.message);

    let text = |contents: &str| json!({"op": "add_text", "sheet": "DEMO-002", "contents": contents, "height": 0.1, "position": [1.2, 1.4, 0]});
    let shouted = tried(text("PUMP A"), 2);
    assert!(!shouted.completes);
    assert_eq!(
        shouted.message,
        "Its `contents` is `PUMP A`; the step wants `Pump A`."
    );
    assert!(tried(text("Pump A"), 2).completes);
}

#[test]
fn a_rectangle_step_completes_a_rectangle_at_its_two_corners() {
    let set = demo_set("rectangle");
    let guide = |points: Value, rings: Value| {
        json!({"title": "A box", "sheet": "DEMO-002", "steps": [
            {"say": "Draw the box.", "tool": "rectangle", "points": rings,
             "completes": {"op": "add_polyline", "sheet": "DEMO-002", "points": points, "closed": true}}
        ]})
    };
    let corners = json!([{"at": [1, 1]}, {"at": [3, 2]}]);
    let refused = |points: Value, rings: Value, field: &str| {
        let error = give(&set, guide(points, rings)).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadValue, "{}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
    };
    refused(
        json!([[1, 1, 0], [3, 2, 0], [3, 1, 0], [1, 2, 0]]),
        corners.clone(),
        "guide.steps[0].completes",
    );
    refused(
        json!([[1, 1, 0], [3, 1, 0], [2, 2, 0], [1, 2, 0]]),
        corners.clone(),
        "guide.steps[0].completes",
    );
    refused(
        json!([[1, 1, 0], [3, 1, 0], [3, 2, 0], [1, 2, 0]]),
        json!([{"at": [1, 1]}, {"at": [3, 3]}]),
        "guide.steps[0].points",
    );
    // A side curved by its arc's angle, as by a bulge, is no rectangle,
    // rings or none.
    for rings in [corners.clone(), json!([])] {
        refused(
            json!([[1, 1, 0], {"point": [3, 1, 0], "arc_angle": 90}, [3, 2, 0], [1, 2, 0]]),
            rings,
            "guide.steps[0].completes",
        );
    }
    assert!(give(
        &set,
        guide(json!([[3, 2, 0], [1, 2, 0], [1, 1, 0], [3, 1, 0]]), corners)
    )
    .is_ok());
}

#[test]
fn a_guide_may_give_its_steps_no_rings() {
    let set = demo_set("no-rings");
    let guide = json!({"title": "A line, moved", "sheet": "DEMO-002", "steps": [
        {"say": "Draw a line.", "tool": "line",
         "completes": {"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [2, 1, 0]}},
        {"say": "Move it up.", "tool": "move",
         "completes": {"op": "move", "target": {"made_by": 1}, "by": [0, 0.5, 0]}}
    ]});
    assert_eq!(give(&set, guide).unwrap(), 1);
    assert!(listed(&set).guides[0].steps[0].points.is_empty());
}

#[test]
fn a_step_names_what_a_copy_made_and_what_a_template_add_made_on_the_guides_sheet() {
    let set = demo_set("made");
    let guide = json!({"title": "Copy the heading, rule a line, move both", "sheet": "DEMO-002", "steps": [
        {"say": "Copy the heading 1 down.", "tool": "copy",
         "points": [{"object": {"id": "DEMO-002/heading"}}],
         "completes": {"op": "copy", "target": {"id": "DEMO-003/heading"}, "by": [0, -1, 0]}},
        {"say": "Move the copy right.", "tool": "move", "points": [{"object": {"made_by": 1}}],
         "completes": {"op": "move", "target": {"made_by": 1}, "by": [0.5, 0, 0]}},
        {"say": "Rule a line on the template.", "tool": "line", "points": [{"at": [1, 1]}, {"at": [2, 1]}],
         "completes": {"op": "add_line", "template": "templates/ansi-b.yaml", "from": [1, 1, 0], "to": [2, 1, 0]}},
        {"say": "Move it up.", "tool": "move", "points": [{"object": {"made_by": 3}}],
         "completes": {"op": "move", "target": {"made_by": 3}, "by": [0, 0.5, 0]}}
    ]});
    give(&set, guide).unwrap();
    let operation = |n: usize| listed(&set).guides[0].steps[n].operation.clone().unwrap();
    let (copied, _) = landed(
        &set,
        json!({"guide": {"guide": 1, "step": 1}, "operations": [operation(0)]}),
    );
    assert!(copied.completes, "{}", copied.message);
    let made = copied.made.unwrap();
    assert!(made.starts_with("DEMO-002/"), "{made}");
    let report = listed(&set);
    assert_eq!(
        report.guides[0].steps[1].points[0].object.as_deref(),
        Some(made.as_str())
    );
    let (ruled, _) = landed(
        &set,
        json!({"guide": {"guide": 1, "step": 3}, "operations": [operation(2)]}),
    );
    assert!(ruled.completes, "{}", ruled.message);
    let report = listed(&set);
    let ruled = report.guides[0].steps[3].points[0].object.clone().unwrap();
    assert!(ruled.starts_with("DEMO-002/"), "{ruled}");
}

#[test]
fn a_setting_steps_layer_is_kept_as_the_sheet_names_it() {
    let set = demo_set("layer-name");
    let mut guide = a_guide();
    guide["steps"][0]["setting"]["current_layer"] = json!("NOTES");
    give(&set, guide).unwrap();
    let report = listed(&set);
    let setting = report.guides[0].steps[0].setting.clone().unwrap();
    assert_eq!(setting.current_layer, "notes");
}

#[test]
fn a_proposal_for_a_guide_the_set_does_not_hold_is_refused() {
    let set = demo_set("proposed");
    let error = propose_with(
        &ProposeRequest {
            set: set.clone(),
            change_set: json!({"guide": {"guide": 4, "step": 1}, "operations": [
                {"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [2, 2, 0]}]}),
            by: Some("Assistant".to_owned()),
        },
        &Settings::default(),
        Interface::Mcp,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName, "{}", error.message);
    assert_eq!(error.field.as_deref(), Some("guide.guide"));
}

#[test]
fn ending_a_guide_drops_it_from_guides_and_refuses_later_change_sets() {
    let set = demo_set("end-guide");
    assert_eq!(give(&set, a_guide()).unwrap(), 1);
    assert_eq!(listed(&set).guides.len(), 1);

    let ended = end_guide(
        &EndGuideRequest {
            set: set.clone(),
            guide: 1,
            by: Some("drafter".to_owned()),
        },
        Interface::Viewer,
    )
    .unwrap();
    assert_eq!(ended.guide, 1);
    assert_eq!(ended.ended_by, "drafter");

    // Ended guides drop out of `guides`, so they are not re-walked.
    assert!(listed(&set).guides.is_empty());

    // Ending again is refused.
    let again = end_guide(
        &EndGuideRequest {
            set: set.clone(),
            guide: 1,
            by: None,
        },
        Interface::Cli,
    )
    .unwrap_err();
    assert_eq!(again.code, ErrorCode::BadValue, "{}", again.message);
    assert_eq!(again.field.as_deref(), Some("guide"));

    // A change set naming an ended guide is refused.
    let err = change(
        &set,
        json!({"guide": {"guide": 1, "step": 2}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": corner(), "to": side()}]}),
    )
    .error
    .unwrap();
    assert_eq!(err.code, ErrorCode::BadValue, "{}", err.message);
    assert_eq!(err.field.as_deref(), Some("guide.guide"));

    // A new guide keeps a fresh number, never reusing 1.
    assert_eq!(give(&set, a_guide()).unwrap(), 2);
    let active: Vec<u64> = listed(&set).guides.iter().map(|g| g.guide).collect();
    assert_eq!(active, vec![2]);
}

#[test]
fn a_guide_step_checks_sibling_sheets_output_preparation_as_a_dry_run_does() {
    let set = demo_set("sibling-dwg");
    // Block sibling sheet DEMO-001's DWG path with a directory so writing it
    // fails, as an open DWG would on a shared template edit.
    let sibling = set.join("dwg").join("DEMO-001.dwg");
    fs::remove_file(&sibling).unwrap();
    fs::create_dir(&sibling).unwrap();

    let template_move = json!({
        "title": "Move the shared border",
        "sheet": "DEMO-002",
        "steps": [{
            "say": "Move the border right.",
            "tool": "move",
            "points": [{"object": {"id": "DEMO-002/border"}}],
            "completes": {"op": "move", "target": {"id": "DEMO-002/border"}, "by": [0.1, 0, 0]}
        }]
    });
    let err = give(&set, template_move).unwrap_err();
    assert_eq!(err.code, ErrorCode::WriteFailed, "{}", err.message);
    assert!(!set.join(GUIDES_FOLDER).exists(), "nothing was recorded");
}

#[test]
fn undo_with_newest_seen_refuses_when_another_edit_landed() {
    let set = demo_set("undo-race");
    let add = |y: f64| {
        change(
            &set,
            json!({"operations": [
                {"op": "add_line", "sheet": "DEMO-002", "from": [1, y, 0], "to": [2, y, 0]}]}),
        )
        .result
        .unwrap()
        .journal_entry
        .unwrap()
    };
    let e1 = add(1.0);
    let e2 = add(2.0);
    assert_eq!((e1, e2), (1, 2));

    let refused = undo(&UndoRequest {
        set: set.clone(),
        change_set: e1,
        newest_seen: Some(e1),
        dry_run: false,
    });
    assert!(!refused.ok);
    let err = refused.error.unwrap();
    assert_eq!(err.code, ErrorCode::MovedOn, "{}", err.message);
    assert_eq!(err.field.as_deref(), Some("newest_seen"));

    // Matching newest_seen succeeds.
    let ok = undo(&UndoRequest {
        set: set.clone(),
        change_set: e2,
        newest_seen: Some(e2),
        dry_run: false,
    });
    assert!(ok.ok, "{:#?}", ok.error);
}

#[test]
fn a_legacy_guide_mark_without_completes_keeps_its_made_by_binding() {
    let set = demo_set("legacy-mark");
    give(&set, a_guide()).unwrap();
    let (checked, entry) = landed(
        &set,
        json!({"guide": {"guide": 1, "step": 2}, "operations": [
            {"op": "add_line", "sheet": "DEMO-002", "layer": "notes", "from": corner(), "to": side()}]}),
    );
    assert!(checked.completes);
    let made = checked.made.unwrap();

    // Strip `"completes":true` from the journal line to simulate a pre-`completes` mark.
    let jpath = set.join(JOURNAL_FILE);
    let text = fs::read_to_string(&jpath).unwrap();
    let patched = text.replace(",\"completes\":true", "");
    assert_ne!(patched, text);
    fs::write(&jpath, patched).unwrap();

    let report = listed(&set);
    let steps = &report.guides[0].steps;
    assert_eq!(steps[1].completed_by, Some(entry));
    assert_eq!(steps[1].made.as_deref(), Some(made.as_str()));
    assert_eq!(steps[2].points[0].object.as_deref(), Some(made.as_str()));
}

#[test]
fn a_quoted_arc_angle_or_bulge_is_still_a_curve() {
    let set = demo_set("quoted-curve");
    let rect_guide = |points: Value| {
        json!({"title": "A box", "sheet": "DEMO-002", "steps": [
            {"say": "Draw the box.", "tool": "rectangle",
             "points": [{"at": [1, 1]}, {"at": [3, 2]}],
             "completes": {"op": "add_polyline", "sheet": "DEMO-002", "points": points, "closed": true}}
        ]})
    };
    // Quoted `arc_angle` or `bulge` in the guide itself is refused as not a rectangle.
    for curved in [
        json!([[1, 1, 0], {"point": [3, 1, 0], "arc_angle": "90"}, [3, 2, 0], [1, 2, 0]]),
        json!([[1, 1, 0], {"point": [3, 1, 0], "bulge": "0.5"}, [3, 2, 0], [1, 2, 0]]),
    ] {
        let err = give(&set, rect_guide(curved)).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadValue, "{}", err.message);
        assert_eq!(err.field.as_deref(), Some("guide.steps[0].completes"));
    }

    // And a change set with quoted `arc_angle: "90"` cannot complete a Rectangle step.
    give(
        &set,
        rect_guide(json!([[1, 1, 0], [3, 1, 0], [3, 2, 0], [1, 2, 0]])),
    )
    .unwrap();
    let (checked, _) = landed(
        &set,
        json!({"guide": {"guide": 1, "step": 1}, "operations": [{
            "op": "add_polyline",
            "sheet": "DEMO-002",
            "points": [[1, 1, 0], {"point": [3, 1, 0], "arc_angle": "90"}, [3, 2, 0], [1, 2, 0]],
            "closed": true
        }]}),
    );
    assert!(!checked.completes, "{}", checked.message);
}
