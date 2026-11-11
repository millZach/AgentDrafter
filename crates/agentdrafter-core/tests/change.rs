//! Change sets on the demo set's working revision: every operation, a
//! change set across sheets, a sheet that fails left byte for byte as it
//! was, errors that name the operation and list the choices, the
//! inconsistent mark reported until fixed, a dry run that changes nothing,
//! a template edit reaching every sheet, a row's own objects, and IDs that
//! stay put.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_set_schema, change_with, operations, query_with, read_dwg, Answer,
    ChangeReport, ChangeRequest, ErrorCode, Object, Query, QueryRequest, Reply, Selection,
    SetBuildRequest, Settings, JOURNAL_FILE, MARK_FILE,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;

const SHEETS: [&str; 3] = ["DEMO-001", "DEMO-002", "DEMO-003"];

/// A built copy of the demo set in a folder of its own, under Cargo's
/// scratch folder in `target/`.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-change")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    set
}

fn apply(set: &Path, change_set: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    )
}

fn dry_run(set: &Path, change_set: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: true,
        },
        &Settings::default(),
    )
}

/// Applies a change set that must land on every sheet it touches.
fn lands(set: &Path, change_set: Value) -> ChangeReport {
    let reply = apply(set, change_set);
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!(report.failed, 0);
    report
}

fn dwg(set: &Path, sheet: &str) -> PathBuf {
    set.join("dwg").join(format!("{sheet}.dwg"))
}

/// Each object a built sheet's DWG holds, by the ID it carries.
fn built(set: &Path, sheet: &str) -> BTreeMap<String, Object> {
    let read = read_dwg(&dwg(set, sheet)).unwrap();
    assert_eq!(read.ids_and_names.len(), read.objects.len());
    read.ids_and_names
        .into_iter()
        .map(|i| i.id.expect("every object a build writes carries its ID"))
        .zip(read.objects)
        .collect()
}

/// Each object the records put on every sheet, by its ID.
fn recorded(set: &Path) -> BTreeMap<String, Object> {
    let report = query_with(
        &QueryRequest {
            source: set.to_owned(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = report.answer else {
        panic!("a find's answer")
    };
    objects.into_iter().map(|o| (o.id, o.object)).collect()
}

/// Every file in `folder` and below, with its bytes, but the journal, which
/// records every change set and failed attempt.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let journal = folder.join(JOURNAL_FILE);
    let mut files = BTreeMap::new();
    let mut stack = vec![folder.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path != journal {
                files.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// The text of sheet `sheet`'s row in the rows file.
fn row_text(set: &Path, sheet: &str) -> String {
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let start = rows.find(&format!("  - sheet: {sheet}\n")).unwrap();
    let end = rows[start + 1..]
        .find("\n  - sheet: ")
        .map_or(rows.len(), |i| start + 2 + i);
    rows[start..end].to_owned()
}

fn polyline_points(object: &Object) -> Vec<[f64; 3]> {
    let Object::Polyline(p) = object else {
        panic!("a polyline: {object:?}")
    };
    p.points.iter().map(|p| [p.x, p.y, p.z]).collect()
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} is not {b}");
}

#[test]
fn every_add_operation_adds_its_object_to_one_sheet_in_its_row() {
    let set = demo_set("adds");
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "add_line", "sheet": "DEMO-002", "from": [1, 1, 0], "to": [3, 2, 0], "layer": "notes"},
            {"op": "PLINE", "sheet": "DEMO-002", "points": [[1, 2, 0], [2, 2, 0], [2, 3, 0]], "closed": true},
            {"op": "add_arc", "sheet": "DEMO-002", "center": [4, 4, 0], "radius": 0.5, "start_angle": 0, "end_angle": 90},
            {"op": "circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": "0.25 in", "id": "hole"},
            {"op": "add_ellipse", "sheet": "DEMO-002", "center": [6, 6, 0], "major_axis": [1, 0, 0], "ratio": 0.5},
            {"op": "add_spline", "sheet": "DEMO-002", "fit_points": [[7, 1, 0], [8, 2, 0], [9, 1, 0]]},
            {"op": "add_hatch", "sheet": "DEMO-002", "pattern": "SOLID", "boundary": [{"polyline": {"points": [[3, 6, 0], [4, 6, 0], [4, 7, 0]]}}]},
            {"op": "add_text", "sheet": "DEMO-002", "contents": "ADDED", "position": [1, 8, 0], "height": 0.2, "style": "notes", "layer": "notes", "name": "added-note"},
            {"op": "add_mtext", "sheet": "DEMO-002", "contents": "TWO\nLINES", "position": [1, 7, 0], "height": 0.1},
            {"op": "insert_block", "sheet": "DEMO-002", "block": "TITLE_BLOCK", "position": [8, 3, 0], "scale": 0.5, "attributes": {"TITLE": "A SECOND TITLE"}},
        ]}),
    );
    assert_eq!(report.landed, 1);
    let ids: Vec<&str> = report
        .operations
        .iter()
        .map(|o| o.id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "DEMO-002/line-1",
            "DEMO-002/polyline-1",
            "DEMO-002/arc-1",
            "DEMO-002/hole",
            "DEMO-002/ellipse-1",
            "DEMO-002/spline-1",
            "DEMO-002/hatch-1",
            "DEMO-002/text-1",
            "DEMO-002/mtext-1",
            "DEMO-002/insert-1",
        ]
    );
    let on_sheet = built(&set, "DEMO-002");
    for id in &ids {
        assert!(on_sheet.contains_key(*id), "{id} is in the DWG");
    }
    let Object::Circle(hole) = &on_sheet["DEMO-002/hole"] else {
        panic!()
    };
    close(hole.radius, 0.25);
    let Object::Insert(second) = &on_sheet["DEMO-002/insert-1"] else {
        panic!()
    };
    assert_eq!(second.attributes[0].value, "A SECOND TITLE");
    // The other sheets have none of them, and their DWGs were not written.
    assert!(!built(&set, "DEMO-001").contains_key("DEMO-001/line-1"));
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    assert!(rows.contains(
        "    objects:\n      - line: {from: [1, 1, 0], to: [3, 2, 0], layer: notes, id: line-1}\n"
    ));
    assert!(rows.contains(
        "      - mtext:\n          contents: |-\n            TWO\n            LINES\n          position: [1, 7, 0]\n"
    ));
    let sheets: Vec<(&str, bool)> = report
        .sheets
        .iter()
        .map(|s| (s.sheet.as_str(), s.dwg_written))
        .collect();
    assert_eq!(sheets, [("DEMO-002", true)]);
}

#[test]
fn change_move_delete_and_set_attribute_edit_the_object_they_name() {
    let set = demo_set("edits");
    lands(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-003", "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "name": "spot"},
            {"op": "add_line", "sheet": "DEMO-003", "from": [0, 0, 0], "to": [1, 1, 0], "id": "doomed"},
        ]}),
    );
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "change", "target": {"name": "spot", "sheet": "DEMO-003"}, "set": {"radius": 0.75, "layer": null}},
            {"op": "move", "target": {"name": "spot"}, "by": [1, "0.5 in", 0]},
            {"op": "ERASE", "target": {"id": "DEMO-003/doomed"}},
            {"op": "set_attribute", "target": {"id": "DEMO-003/objects[2]"}, "tag": "REV", "value": "B"},
            {"op": "attedit", "target": {"id": "DEMO-001/objects[2]"}, "tag": "DRAWN", "value": "ZM"},
        ]}),
    );
    let on_sheet = built(&set, "DEMO-003");
    let Object::Circle(spot) = &on_sheet["DEMO-003/circle-1"] else {
        panic!()
    };
    close(spot.radius, 0.75);
    assert_eq!(spot.layer, "0");
    close(spot.center.x, 6.0);
    close(spot.center.y, 5.5);
    assert!(!on_sheet.contains_key("DEMO-003/doomed"));
    let Object::Insert(title) = &on_sheet["DEMO-003/objects[2]"] else {
        panic!()
    };
    let value = |tag: &str| {
        title
            .attributes
            .iter()
            .find(|a| a.tag == tag)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value("REV"), "B");
    assert_eq!(value("DRAWN"), "ZM");
    // REV comes from the row, so it was set in DEMO-003's row alone and
    // the reply names where it fills; DRAWN is the template's, so every
    // sheet follows.
    let rev = report.operations[3].row_value.as_ref().unwrap();
    assert_eq!((rev.was.as_str(), rev.now.as_str()), ("A", "B"));
    assert_eq!(rev.fills.len(), 1);
    assert_eq!(rev.fills[0].id, "DEMO-003/objects[2]");
    assert_eq!(rev.fills[0].at, "attributes.REV");
    assert_eq!(report.operations[3].sheets, ["DEMO-003"]);
    assert_eq!(report.operations[4].sheets, SHEETS);
    for sheet in ["DEMO-001", "DEMO-002"] {
        let Object::Insert(other) = &built(&set, sheet)[&format!("{sheet}/objects[2]")] else {
            panic!()
        };
        let rev = other.attributes.iter().find(|a| a.tag == "REV").unwrap();
        assert_eq!(rev.value, "0");
        let drawn = other.attributes.iter().find(|a| a.tag == "DRAWN").unwrap();
        assert_eq!(drawn.value, "ZM");
    }
    assert!(row_text(&set, "DEMO-003").contains("      REV: B\n"));
}

#[test]
fn a_text_the_template_takes_from_the_row_is_set_in_that_sheets_row() {
    let set = demo_set("row-text");
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "change", "target": {"id": "DEMO-002/heading"}, "set": {"contents": "AREA 200 RENAMED"}},
        ]}),
    );
    let set_value = report.operations[0].row_value.as_ref().unwrap();
    assert_eq!(set_value.value, "TITLE");
    let fills: Vec<(&str, &str)> = set_value
        .fills
        .iter()
        .map(|f| (f.id.as_str(), f.at.as_str()))
        .collect();
    assert_eq!(
        fills,
        [
            ("DEMO-002/objects[2]", "attributes.TITLE"),
            ("DEMO-002/heading", "contents")
        ]
    );
    let template = fs::read_to_string(set.join("templates/ansi-b.yaml")).unwrap();
    assert!(template.contains("contents: {row: TITLE}"));
    let Object::Text(heading) = &built(&set, "DEMO-002")["DEMO-002/heading"] else {
        panic!()
    };
    assert_eq!(heading.contents, "AREA 200 RENAMED");
    let Object::Text(other) = &built(&set, "DEMO-001")["DEMO-001/heading"] else {
        panic!()
    };
    assert_eq!(other.contents, "AREA 100 GENERAL ARRANGEMENT");
}

#[test]
fn every_operation_is_listed_with_its_alias_and_a_schema_that_takes_it() {
    let listed = operations();
    let names: Vec<(&str, &str)> = listed
        .operations
        .iter()
        .map(|o| (o.name.as_str(), o.alias.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("add_line", "LINE"),
            ("add_polyline", "PLINE"),
            ("add_arc", "ARC"),
            ("add_circle", "CIRCLE"),
            ("add_ellipse", "ELLIPSE"),
            ("add_spline", "SPLINE"),
            ("add_hatch", "HATCH"),
            ("add_text", "TEXT"),
            ("add_mtext", "MTEXT"),
            ("insert_block", "INSERT"),
            ("change", "CHANGE"),
            ("move", "MOVE"),
            ("delete", "ERASE"),
            ("set_attribute", "ATTEDIT"),
        ]
    );
    let samples = [
        json!({"op": "add_line", "sheet": "DEMO-001", "from": [0, 0, 0], "to": [1, 1, 0]}),
        json!({"op": "pline", "template": "templates/ansi-b.yaml", "points": [[0, 0, 0], [1, 0, 0]]}),
        json!({"op": "add_arc", "sheet": "S", "center": [0, 0, 0], "radius": 1, "start_angle": 0, "end_angle": 90}),
        json!({"op": "CIRCLE", "sheet": "S", "center": [0, 0, 0], "radius": "2 mm"}),
        json!({"op": "add_ellipse", "sheet": "S", "center": [0, 0, 0], "major_axis": [1, 0, 0], "ratio": 0.5}),
        json!({"op": "add_spline", "sheet": "S", "control_points": [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0]]}),
        json!({"op": "HATCH", "sheet": "S", "pattern": "SOLID", "boundary": [{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}),
        json!({"op": "add_text", "sheet": "S", "contents": "A", "position": [0, 0, 0], "height": 1}),
        json!({"op": "add_mtext", "sheet": "S", "contents": "A\nB", "position": [0, 0, 0], "height": 1}),
        json!({"op": "INSERT", "sheet": "S", "block": "B", "position": [0, 0, 0], "attributes": {"TAG": "X"}}),
        json!({"op": "change", "target": {"id": "S/a"}, "set": {"layer": "notes"}}),
        json!({"op": "move", "target": {"name": "n", "sheet": "S"}, "by": [1, 0, 0]}),
        json!({"op": "erase", "target": {"name": "n"}}),
        json!({"op": "set_attribute", "target": {"id": "S/objects[2]"}, "tag": "REV", "value": "B"}),
    ];
    for (info, sample) in listed.operations.iter().zip(&samples) {
        assert!(info.does.contains(&info.alias), "{}", info.does);
        let validator = jsonschema::validator_for(&info.schema).unwrap();
        assert!(validator.is_valid(sample), "{}: {sample}", info.name);
        for other in &samples {
            if other != sample {
                assert!(!validator.is_valid(other), "{} takes {other}", info.name);
            }
        }
    }
    let whole = jsonschema::validator_for(&change_set_schema()).unwrap();
    assert!(whole.is_valid(&json!({"operations": samples, "fixes": ["DEMO-002"]})));
    for wrong in [
        json!({"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [1, 1, 0]}]}),
        json!({"operations": [{"op": "add_line", "sheet": "S", "template": "t.yaml", "from": [0, 0, 0], "to": [1, 1, 0]}]}),
        json!({"operations": [{"op": "change", "target": {"id": "S/a"}, "set": {"id": "b"}}]}),
        json!({"operations": [{"op": "move", "target": {"id": "S/a", "name": "n"}, "by": [1, 0, 0]}]}),
        json!({"operations": [{"op": "move", "target": {"id": "S/a"}, "by": [1, 0]}]}),
        json!({"operations": [{"op": "nudge", "target": {"id": "S/a"}}]}),
        json!({"operations": [], "undo": true}),
    ] {
        assert!(!whole.is_valid(&wrong), "the schema refuses {wrong}");
    }
}

#[test]
fn a_change_set_spanning_several_sheets_lands_on_each_and_builds_only_them() {
    let set = demo_set("spanning");
    let untouched = fs::read(dwg(&set, "DEMO-003")).unwrap();
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "add_text", "sheet": "DEMO-001", "contents": "ONE", "position": [2, 2, 0], "height": 0.2},
            {"op": "set_attribute", "target": {"id": "DEMO-002/objects[2]"}, "tag": "REV", "value": "C"},
        ]}),
    );
    assert_eq!(report.landed, 2);
    let touched: Vec<&str> = report.sheets.iter().map(|s| s.sheet.as_str()).collect();
    assert_eq!(touched, ["DEMO-001", "DEMO-002"]);
    assert!(report.sheets.iter().all(|s| s.ok && s.dwg_written));
    assert!(built(&set, "DEMO-001").contains_key("DEMO-001/text-1"));
    let Object::Insert(title) = &built(&set, "DEMO-002")["DEMO-002/objects[2]"] else {
        panic!()
    };
    assert_eq!(
        title
            .attributes
            .iter()
            .find(|a| a.tag == "REV")
            .unwrap()
            .value,
        "C"
    );
    assert_eq!(fs::read(dwg(&set, "DEMO-003")).unwrap(), untouched);
    // The records and the DWGs agree: building again changes nothing.
    let before = snapshot(&set);
    let again = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
        },
        &Settings::default(),
    );
    assert!(again.ok);
    assert_eq!(snapshot(&set), before);
}

#[test]
fn a_sheet_that_fails_keeps_its_row_template_and_dwg_byte_for_byte() {
    let set = demo_set("failure");
    let row = row_text(&set, "DEMO-002");
    let template = fs::read(set.join("templates/ansi-b.yaml")).unwrap();
    let dwgs: Vec<Vec<u8>> = SHEETS
        .iter()
        .map(|s| fs::read(dwg(&set, s)).unwrap())
        .collect();
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "set_attribute", "target": {"id": "DEMO-002/objects[2]"}, "tag": "REV", "value": "B"},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::SheetsFailed);
    let report = reply.result.unwrap();
    assert_eq!((report.landed, report.failed), (1, 1));
    assert_eq!(row_text(&set, "DEMO-002"), row);
    assert_eq!(
        fs::read(set.join("templates/ansi-b.yaml")).unwrap(),
        template
    );
    assert_eq!(fs::read(dwg(&set, "DEMO-002")).unwrap(), dwgs[1]);
    assert_eq!(fs::read(dwg(&set, "DEMO-003")).unwrap(), dwgs[2]);
    assert_ne!(fs::read(dwg(&set, "DEMO-001")).unwrap(), dwgs[0]);
    assert!(row_text(&set, "DEMO-001").contains("circle-1"));
    assert!(!report.operations[1].ok && !report.operations[2].ok);
}

#[test]
fn a_template_edit_one_sheet_cannot_take_lands_on_none_of_its_sheets() {
    // Zach decided on 2026-11-11 that a template edit lands only if every
    // sheet built from the template takes its whole part.
    let set = demo_set("template-all-or-nothing");
    let before = snapshot(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
            {"op": "add_circle", "sheet": "DEMO-003", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    assert!(!reply.ok);
    let report = reply.result.unwrap();
    assert_eq!((report.landed, report.failed), (0, 3));
    let codes: Vec<ErrorCode> = report
        .sheets
        .iter()
        .map(|s| s.error.as_ref().unwrap().code)
        .collect();
    assert_eq!(
        codes,
        [
            ErrorCode::SharedEditFailed,
            ErrorCode::UnknownName,
            ErrorCode::SharedEditFailed
        ]
    );
    assert!(
        report.message.contains("unknown layer `nope`"),
        "{}",
        report.message
    );
    assert_eq!(snapshot(&set), before);
    assert!(
        report.inconsistent.is_none(),
        "nothing landed, so nothing disagrees"
    );
}

#[test]
fn a_failed_operation_names_itself_and_lists_the_valid_choices() {
    let set = demo_set("errors");
    let refused = |change_set: Value| {
        let reply = apply(&set, change_set);
        assert!(!reply.ok);
        assert!(reply.result.is_none(), "the whole change set is refused");
        reply.error.unwrap()
    };
    let e = refused(json!({"operations": [{"op": "nudge", "target": {"id": "DEMO-001/border"}}]}));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].op"));
    assert!(
        e.message
            .contains("operation 0: there is no operation `nudge`"),
        "{}",
        e.message
    );
    assert!(e.message.contains("move (MOVE)") && e.message.contains("delete (ERASE)"));

    let e = refused(
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-009/border"}, "by": [1, 0, 0]}]}),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].target.id"));
    assert!(e.message.contains("operation 0 (`move`)"));
    assert!(
        e.message
            .contains("its sheets: DEMO-001, DEMO-002, DEMO-003"),
        "{}",
        e.message
    );

    let e = refused(
        json!({"operations": [{"op": "add_line", "sheet": "DEMO-001", "from": [0, 0, 0], "to": [1, 1, 0], "colour": 1}]}),
    );
    assert_eq!(e.code, ErrorCode::UnknownKey);
    assert!(
        e.message.contains("from, to, layer, id, name"),
        "{}",
        e.message
    );

    let failed = |change_set: Value| {
        let reply = apply(&set, change_set);
        assert!(!reply.ok);
        let report = reply.result.unwrap();
        let op = report.operations.into_iter().find(|o| !o.ok).unwrap();
        op.error.unwrap()
    };
    let e = failed(
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/nothing"}, "by": [1, 0, 0]}]}),
    );
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].target.id"));
    assert!(
        e.message.contains("its IDs: DEMO-001/objects[0], DEMO-001/border, DEMO-001/objects[2], DEMO-001/heading, DEMO-001/notes"),
        "{}",
        e.message
    );

    let e = failed(
        json!({"operations": [{"op": "add_text", "sheet": "DEMO-002", "contents": "A", "position": [0, 0, 0], "height": 1, "style": "bold"}]}),
    );
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].style"));
    assert!(
        e.message
            .starts_with("operation 0 (`add_text`) on sheet `DEMO-002`:"),
        "{}",
        e.message
    );
    assert!(e.message.contains("Standard, notes"), "{}", e.message);

    let e = failed(
        json!({"operations": [{"op": "set_attribute", "target": {"name": "title-block", "sheet": "DEMO-001"}, "tag": "OWNER", "value": "X"}]}),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].tag"));
    assert!(
        e.message.contains("TITLE, DWG_NO, SHEET, REV, DATE, DRAWN"),
        "{}",
        e.message
    );

    let e = failed(
        json!({"operations": [{"op": "change", "target": {"id": "DEMO-001/heading"}, "set": {"justify": "sideways"}}]}),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].set.justify"));
    assert!(e.message.contains("top_left"), "{}", e.message);

    let e = failed(
        json!({"operations": [{"op": "change", "target": {"id": "DEMO-001/border"}, "set": {"id": "frame"}}]}),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].set.id"));
    assert!(e.message.contains("fixed for its life"));
}

#[test]
fn an_inconsistent_set_is_reported_until_its_failed_sheet_is_fixed() {
    let set = demo_set("inconsistent");
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    let report = reply.result.unwrap();
    let mark = report.inconsistent.expect("the set is marked");
    assert_eq!(mark.failures.len(), 1);
    assert_eq!(mark.failures[0].sheet, "DEMO-002");
    assert_eq!(mark.failures[0].landed_on, ["DEMO-001"]);
    assert_eq!(mark.failures[0].operations[0]["layer"], "nope");
    assert!(set.join(MARK_FILE).is_file());

    // Every reply about the set says so: a build, a query, and a change
    // set that does not fix it.
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
        },
        &Settings::default(),
    );
    assert_eq!(
        built.result.unwrap().inconsistent.unwrap().failures[0].sheet,
        "DEMO-002"
    );
    let asked = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Blocks,
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(asked.inconsistent.unwrap().failures[0].sheet, "DEMO-002");
    let other = lands(
        &set,
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-003", "center": [1, 1, 0], "radius": 0.1}]}),
    );
    assert!(other.inconsistent.is_some());
    assert!(
        other
            .message
            .contains("inconsistent until `DEMO-002` is fixed"),
        "{}",
        other.message
    );

    // A fix that fails leaves the mark; one that lands clears it.
    let bad_fix = apply(
        &set,
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": -1}], "fixes": ["DEMO-002"]}),
    );
    assert!(!bad_fix.ok);
    assert!(set.join(MARK_FILE).is_file());
    let fixed = lands(
        &set,
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes"}], "fixes": ["DEMO-002"]}),
    );
    assert!(fixed.inconsistent.is_none());
    assert!(fixed.sheets[0].fixed);
    assert!(!set.join(MARK_FILE).exists());
    assert!(set.join(JOURNAL_FILE).is_file(), "the journal stays");
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
        },
        &Settings::default(),
    );
    assert!(built.result.unwrap().inconsistent.is_none());
    let e = apply(&set, json!({"operations": [], "fixes": ["DEMO-002"]}))
        .error
        .unwrap();
    assert!(
        e.message.contains("not marked inconsistent"),
        "{}",
        e.message
    );
}

#[test]
fn a_dry_run_reports_what_would_change_and_changes_nothing() {
    let set = demo_set("dry-run");
    let before = snapshot(&set);
    let reply = dry_run(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.25, 0, 0]},
            {"op": "set_attribute", "target": {"id": "DEMO-001/objects[2]"}, "tag": "REV", "value": "D"},
            {"op": "add_line", "sheet": "DEMO-002", "from": [0, 0, 0], "to": [1, 1, 0], "layer": "nope"},
        ]}),
    );
    assert_eq!(snapshot(&set), before, "a dry run writes nothing");
    let report = reply.result.unwrap();
    assert!(report.dry_run);
    assert!(report.message.starts_with("a dry run, which wrote nothing"));
    // The move is a template edit and DEMO-002 cannot take its part, so it
    // would land on no sheet, and DEMO-001's other operation with it.
    assert_eq!((report.landed, report.failed), (0, 3));
    let reply = dry_run(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.25, 0, 0]},
            {"op": "set_attribute", "target": {"id": "DEMO-001/objects[2]"}, "tag": "REV", "value": "D"},
        ]}),
    );
    assert_eq!(snapshot(&set), before, "a dry run writes nothing");
    let report = reply.result.unwrap();
    assert_eq!(report.landed, 3);
    assert!(report
        .sheets
        .iter()
        .all(|s| !s.dwg_written && s.result.is_some()));
    assert_eq!(
        report.operations[0].after.as_deref(),
        Some("polyline: {points: [[0.75, 0.5, 0], [16.75, 0.5, 0], [16.75, 10.5, 0], [0.75, 10.5, 0]], closed: true, layer: border, id: border}")
    );
    assert_eq!(report.operations[1].row_value.as_ref().unwrap().now, "D");
    let mut records: Vec<&str> = report.records.iter().map(String::as_str).collect();
    records.sort();
    assert!(records[0].ends_with("rows.yaml") && records[1].ends_with("ansi-b.yaml"));
    // A dry run that would mark the set inconsistent says so, and does not.
    let reply = dry_run(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    let report = reply.result.unwrap();
    assert!(report.inconsistent.is_some());
    assert_eq!(snapshot(&set), before, "a dry run writes nothing");
    assert!(!set.join(MARK_FILE).exists());
}

#[test]
fn a_template_objects_edit_reaches_every_sheet_built_from_it() {
    let set = demo_set("template");
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"id": "DEMO-002/border"}, "by": [0.25, 0, 0]},
            {"op": "add_text", "template": "templates/ansi-b.yaml", "contents": "ON EVERY SHEET", "position": [1, 1, 0], "height": 0.2, "id": "everywhere"},
        ]}),
    );
    assert_eq!(report.operations[0].sheets, SHEETS);
    assert_eq!(report.operations[1].sheets, SHEETS);
    assert_eq!(report.landed, 3);
    for sheet in SHEETS {
        let objects = built(&set, sheet);
        let points = polyline_points(&objects[&format!("{sheet}/border")]);
        close(points[0][0], 0.75);
        close(points[1][0], 16.75);
        let Object::Text(text) = &objects[&format!("{sheet}/everywhere")] else {
            panic!()
        };
        assert_eq!(text.contents, "ON EVERY SHEET");
    }
}

#[test]
fn a_rows_own_objects_are_built_into_its_sheet_alone() {
    let set = demo_set("row-objects");
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap().replace(
        "      SHEET: 2 OF 3\n      REV: '0'\n      DATE: 2026-11-11\n",
        "      SHEET: 2 OF 3\n      REV: '0'\n      DATE: 2026-11-11\n    objects:\n      - circle: {center: [8, 5, 0], radius: 1, layer: notes, id: tank}\n      - text: {contents: TANK, position: [8, 5, 0], height: 0.2, style: notes, name: tank-label}\n",
    );
    fs::write(set.join("rows.yaml"), rows).unwrap();
    let built_again = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
        },
        &Settings::default(),
    );
    assert!(built_again.ok, "{:#?}", built_again.error);
    let objects = built(&set, "DEMO-002");
    assert!(matches!(objects["DEMO-002/tank"], Object::Circle(_)));
    assert!(matches!(
        objects["DEMO-002/rows[1].objects[1]"],
        Object::Text(_)
    ));
    assert_eq!(objects.len(), 7);
    assert_eq!(built(&set, "DEMO-001").len(), 5);
    let records = recorded(&set);
    assert!(records.contains_key("DEMO-002/tank"));
    assert!(!records.contains_key("DEMO-001/tank"));
    let report = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Find(Selection {
                name: Some("tank-label".into()),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = report.answer else {
        panic!()
    };
    assert!(objects[0].written_at.file.ends_with("rows.yaml"));
    assert_eq!(objects[0].written_at.field, "rows[1].objects[1]");

    // A row's object that names what the template does not define, or
    // takes a value from the row, is refused in the rows file.
    for (object, code, field) in [
        (
            "{center: [8, 5, 0], radius: 1, layer: nope}",
            ErrorCode::UnknownName,
            "rows[1].objects[0].circle.layer",
        ),
        (
            "{center: [8, 5, 0], radius: 1, id: border}",
            ErrorCode::DuplicateId,
            "rows[1].objects[0].circle.id",
        ),
    ] {
        let rows = fs::read_to_string(set.join("rows.yaml")).unwrap().replace(
            "{center: [8, 5, 0], radius: 1, layer: notes, id: tank}",
            object,
        );
        let broken = set
            .parent()
            .unwrap()
            .join(format!("broken-{}", field.len()));
        let _ = fs::remove_dir_all(&broken);
        demo_set::copy_demo_set(&broken);
        fs::write(broken.join("rows.yaml"), rows).unwrap();
        let reply = build_set_with(
            &SetBuildRequest {
                set: broken.clone(),
                overwrite: false,
            },
            &Settings::default(),
        );
        let error = reply.result.unwrap().sheets[1].error.clone().unwrap();
        assert_eq!(error.code, code, "{error:?}");
        assert_eq!(error.field.as_deref(), Some(field));
        assert!(error.file.unwrap().ends_with("rows.yaml"));
    }
    let rows = fs::read_to_string(set.join("rows.yaml"))
        .unwrap()
        .replace("contents: TANK", "contents: {row: TITLE}");
    fs::write(set.join("rows.yaml"), rows).unwrap();
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
        },
        &Settings::default(),
    );
    let error = reply.result.unwrap().sheets[1].error.clone().unwrap();
    assert!(error.message.contains("belongs in a template"), "{error:?}");
}

#[test]
fn ids_stay_put_across_edits() {
    let set = demo_set("ids");
    let first = recorded(&set);
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "delete", "target": {"id": "DEMO-001/objects[0]"}},
            {"op": "add_text", "template": "templates/ansi-b.yaml", "contents": "NEW", "position": [2, 2, 0], "height": 0.2},
            {"op": "add_line", "sheet": "DEMO-002", "from": [0, 0, 0], "to": [1, 1, 0]},
            {"op": "move", "target": {"id": "DEMO-001/heading"}, "by": [0.5, 0, 0]},
        ]}),
    );
    assert_eq!(
        report.operations[0].ids_written,
        [
            "DEMO-001/objects[2]",
            "DEMO-002/objects[2]",
            "DEMO-003/objects[2]"
        ]
    );
    let second = recorded(&set);
    for (id, object) in &first {
        if id.ends_with("/objects[0]") {
            assert!(!second.contains_key(id), "{id} was deleted");
            continue;
        }
        let now = second
            .get(id)
            .unwrap_or_else(|| panic!("{id} keeps its ID"));
        if !id.ends_with("/heading") {
            assert_eq!(now, object, "{id} is the same object");
        }
    }
    // The title block, which had no `id`, keeps its place as its ID, and
    // the DWGs carry the same IDs as the records.
    for sheet in SHEETS {
        let objects = built(&set, sheet);
        assert!(matches!(
            objects[&format!("{sheet}/objects[2]")],
            Object::Insert(_)
        ));
        assert!(objects.contains_key(&format!("{sheet}/text-1")));
        let on_sheet: Vec<&String> = second.keys().filter(|id| id.starts_with(sheet)).collect();
        let in_dwg: Vec<&String> = objects.keys().collect();
        assert_eq!(on_sheet, in_dwg);
    }
    // Deleting again, and editing what was added, keeps every ID.
    lands(
        &set,
        json!({"operations": [
            {"op": "delete", "target": {"id": "DEMO-003/border"}},
            {"op": "move", "target": {"id": "DEMO-002/line-1"}, "by": [1, 1, 0]},
            {"op": "change", "target": {"id": "DEMO-001/text-1"}, "set": {"height": 0.3}},
        ]}),
    );
    let third = recorded(&set);
    for id in second.keys() {
        if !id.ends_with("/border") {
            assert!(third.contains_key(id), "{id} keeps its ID");
        }
    }
    assert_eq!(third.len(), second.len() - 3);
}

#[test]
fn a_file_at_a_sheets_path_that_agentdrafter_did_not_build_is_never_touched() {
    let set = demo_set("original");
    let original = b"not a DWG AgentDrafter built: it may be an original".to_vec();
    fs::write(dwg(&set, "DEMO-002"), &original).unwrap();
    let row = row_text(&set, "DEMO-002");
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5},
        ]}),
    );
    let report = reply.result.unwrap();
    let error = report.sheets[1].error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::NotABuild);
    assert_eq!(fs::read(dwg(&set, "DEMO-002")).unwrap(), original);
    assert_eq!(row_text(&set, "DEMO-002"), row);
    assert!(report.sheets[0].ok);
}

#[test]
fn a_partial_change_whose_mark_has_a_file_in_its_folders_place_does_not_land() {
    let set = demo_set("mark-unreadable");
    // A file where the mark's folder would go.
    fs::write(set.join(".agentdrafter"), b"in the way").unwrap();
    let before = snapshot(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    assert!(reply.result.is_none(), "nothing landed");
    // Linux and macOS refuse to read a mark under a file; Windows finds no
    // mark there, and the commit then cannot write one. Either way nothing
    // lands.
    let code = reply.error.unwrap().code;
    assert!(
        matches!(code, ErrorCode::ReadFailed | ErrorCode::WriteFailed),
        "{code:?}"
    );
    assert_eq!(snapshot(&set), before);
}

/// A folder no one may write in holds the mark: only Unix keeps a folder
/// from being written in this way. The commit's own tests fail the mark's
/// step on every system.
#[cfg(unix)]
#[test]
fn a_partial_change_whose_mark_cannot_be_written_does_not_land() {
    use std::os::unix::fs::PermissionsExt;
    let set = demo_set("mark-unwritable");
    let folder = set.join(".agentdrafter");
    fs::create_dir(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
    let before = snapshot(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(reply.result.is_none(), "nothing landed");
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::WriteFailed);
    assert!(
        error.message.contains("inconsistent mark"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("nothing was changed"),
        "{}",
        error.message
    );
    assert_eq!(snapshot(&set), before);
    assert!(fs::read_dir(&folder).unwrap().next().is_none());
}

#[test]
fn two_spellings_of_one_template_file_are_one_template() {
    // On a file system that ignores case, as Windows' and macOS's do,
    // `templates/t.yaml` and `templates/T.yaml` are one file, so one
    // template: an edit to it is refused, naming both, since it would be
    // written through one of them only. Where case counts, as on Linux,
    // they are two paths and there is nothing to show.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-change")
        .join("template-case");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("templates")).unwrap();
    fs::write(
        dir.join("templates/t.yaml"),
        "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, id: c}\n",
    )
    .unwrap();
    if !dir.join("templates/T.yaml").exists() {
        return;
    }
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n  - sheet: S-2\n    template: templates/T.yaml\n",
    )
    .unwrap();
    let before = snapshot(&dir);
    let reply = apply(
        &dir,
        json!({"operations": [{"op": "change", "target": {"id": "S-1/c"}, "set": {"radius": 2}}]}),
    );
    assert!(reply.result.is_none());
    let e = reply.error.unwrap();
    assert!(
        e.message
            .contains("`templates/t.yaml` (sheet `S-1`), `templates/T.yaml` (sheet `S-2`)"),
        "{}",
        e.message
    );
    assert_eq!(snapshot(&dir), before);
}

/// A template one row names through a link and another by its own path:
/// only Unix makes such a link without asking for rights.
#[cfg(unix)]
#[test]
fn an_edit_to_a_template_reached_by_two_paths_is_refused_whole() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-change")
        .join("template-link");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("templates")).unwrap();
    fs::write(
        dir.join("templates/t.yaml"),
        "units: mm\nlayers:\n  notes: {}\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, id: c}\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("t.yaml", dir.join("templates/alias.yaml")).unwrap();
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/alias.yaml\n  - sheet: S-2\n    template: templates/t.yaml\n",
    )
    .unwrap();
    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    let before = snapshot(&dir);
    for (operation, field) in [
        (
            json!({"op": "change", "target": {"id": "S-2/c"}, "set": {"radius": 2}}),
            "operations[0].target",
        ),
        (
            json!({"op": "move", "target": {"id": "S-1/c"}, "by": [1, 0, 0]}),
            "operations[0].target",
        ),
        (
            json!({"op": "delete", "target": {"id": "S-2/c"}}),
            "operations[0].target",
        ),
        (
            json!({"op": "add_line", "template": "templates/t.yaml", "from": [0, 0, 0], "to": [1, 1, 0]}),
            "operations[0].template",
        ),
    ] {
        let reply = apply(&dir, json!({"operations": [operation]}));
        assert!(reply.result.is_none(), "the whole change set is refused");
        let e = reply.error.unwrap();
        assert_eq!(e.code, ErrorCode::BadValue);
        assert_eq!(e.field.as_deref(), Some(field));
        assert!(
            e.message
                .contains("`templates/alias.yaml` (sheet `S-1`), `templates/t.yaml` (sheet `S-2`)"),
            "{}",
            e.message
        );
        assert_eq!(snapshot(&dir), before);
        assert!(fs::symlink_metadata(dir.join("templates/alias.yaml"))
            .unwrap()
            .file_type()
            .is_symlink());
    }
    // An edit to one sheet's row is no edit to the template, and lands.
    let report = lands(
        &dir,
        json!({"operations": [{"op": "add_circle", "sheet": "S-1", "center": [1, 1, 0], "radius": 0.5, "layer": "notes"}]}),
    );
    assert_eq!(report.landed, 1);
}

#[test]
fn an_add_that_leaves_out_a_required_key_refuses_the_whole_change_set() {
    let set = demo_set("required");
    let before = snapshot(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "add_line", "sheet": "DEMO-001", "from": [0, 0, 0]},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5},
        ]}),
    );
    assert!(reply.result.is_none(), "nothing is applied");
    let e = reply.error.unwrap();
    assert_eq!(e.code, ErrorCode::MissingKey);
    assert_eq!(e.field.as_deref(), Some("operations[0].to"));
    assert!(
        e.message
            .starts_with("operation 0 (`add_line`): missing key `to`; a line needs from, to"),
        "{}",
        e.message
    );
    assert_eq!(snapshot(&set), before);

    let e = apply(
        &set,
        json!({"operations": [{"op": "add_spline", "sheet": "DEMO-001", "degree": 2}]}),
    )
    .error
    .unwrap();
    assert_eq!(e.code, ErrorCode::MissingKey);
    assert!(
        e.message
            .contains("a spline needs fit_points or control_points"),
        "{}",
        e.message
    );
    assert_eq!(snapshot(&set), before);
}

#[test]
fn an_error_names_the_operation_that_set_the_bad_value() {
    let set = demo_set("blame");
    let error_of = |change_set: Value| {
        let report = apply(&set, change_set).result.unwrap();
        report.sheets[0].error.clone().unwrap()
    };
    // The add put the bad layer there; the move after it only moved it.
    let e = error_of(json!({"operations": [
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope", "id": "spot"},
        {"op": "move", "target": {"id": "DEMO-002/spot"}, "by": [1, 0, 0]},
    ]}));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].layer"));
    assert!(
        e.message.starts_with("operation 0 (`add_circle`)"),
        "{}",
        e.message
    );
    // A change after the add set it, and a move after the change did not.
    let e = error_of(json!({"operations": [
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "id": "spot"},
        {"op": "change", "target": {"id": "DEMO-002/spot"}, "set": {"layer": "nope"}},
        {"op": "move", "target": {"id": "DEMO-002/spot"}, "by": [1, 0, 0]},
    ]}));
    assert_eq!(e.field.as_deref(), Some("operations[1].set.layer"));
    assert!(
        e.message.starts_with("operation 1 (`change`)"),
        "{}",
        e.message
    );
}
