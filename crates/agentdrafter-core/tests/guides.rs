//! Guides (#157): `guide` checks a guide on the set as it is and records it
//! beside the journal, writing nothing else; `guides` lists each with its
//! steps worked out on the set as it stands; a change set sent for a step
//! names it under `guide`, which marks its journal entry and checks it
//! against the step's operation.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, guide_with, guides_with, journal, undo, ChangeReport,
    ChangeRequest, ErrorCode, GuideRequest, GuidesReport, GuidesRequest, Interface, JournalRequest,
    Reply, SetBuildRequest, Settings, UndoRequest, GUIDES_FOLDER, JOURNAL_FILE,
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
