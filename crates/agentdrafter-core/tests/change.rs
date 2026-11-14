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
    build_set_with, change_set_schema, change_with, operations, query_with, read_dwg, undo, Answer,
    ChangeReport, ChangeRequest, ErrorCode, HowChanged, JournalRequest, Object, ObjectColor,
    ObjectLineweight, Query, QueryRequest, Reply, Selection, SetBuildRequest, Settings,
    UndoRequest, JOURNAL_FILE, LOCK_FILE, MARK_FILE,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

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
/// records every change set and failed attempt, and its lock, which every
/// one takes.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let journal = folder.join(JOURNAL_FILE);
    let lock = folder.join(LOCK_FILE);
    let mut files = BTreeMap::new();
    let mut stack = vec![folder.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path != journal && path != lock {
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
            ("add_donut", "DONUT"),
            ("add_solid", "SOLID"),
            ("insert_block", "INSERT"),
            ("add_table", "TABLE"),
            ("change", "CHANGE"),
            ("move", "MOVE"),
            ("delete", "ERASE"),
            ("set_attribute", "ATTEDIT"),
            ("set_cell", "TABLEDIT"),
            ("add_row", ""),
            ("delete_row", ""),
            ("add_column", ""),
            ("delete_column", ""),
            ("add_layer", "LAYER"),
            ("change_layer", ""),
            ("rename_layer", "RENAME"),
            ("delete_layer", "LAYDEL"),
            ("add_linetype", "LINETYPE"),
            ("change_linetype", ""),
            ("rename_linetype", ""),
            ("delete_linetype", ""),
            ("add_text_style", "STYLE"),
            ("change_text_style", ""),
            ("rename_text_style", ""),
            ("delete_text_style", ""),
            ("add_block", "BLOCK"),
            ("redefine_block", "BEDIT"),
            ("attsync", "ATTSYNC"),
            ("import_blocks", "ADCENTER"),
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
        json!({"op": "add_donut", "sheet": "S", "center": [0, 0, 0], "outside_diameter": 1}),
        json!({"op": "SOLID", "sheet": "S", "corners": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}),
        json!({"op": "INSERT", "sheet": "S", "block": "B", "position": [0, 0, 0], "attributes": {"TAG": "X", "NOTE": ["a", null]}}),
        json!({"op": "TABLE", "sheet": "S", "position": [0, 10, 0], "columns": [5, 5], "rows": [2], "cells": [["A", {"text": "B", "height": 1}]]}),
        json!({"op": "change", "target": {"id": "S/a"}, "set": {"layer": "notes"}}),
        json!({"op": "move", "target": {"name": "n", "sheet": "S"}, "by": [1, 0, 0]}),
        json!({"op": "erase", "target": {"name": "n"}}),
        json!({"op": "set_attribute", "target": {"id": "S/objects[2]"}, "tag": "NOTE", "occurrence": 2, "value": "B"}),
        json!({"op": "tabledit", "target": {"id": "S/cables"}, "cell": [2, 1], "set": {"text": "JB-9", "merge": null}}),
        json!({"op": "add_row", "target": {"id": "S/cables"}, "at": 2, "height": "6 mm", "cells": ["A", "", {"text": "B", "justify": "middle_left"}]}),
        json!({"op": "delete_row", "target": {"name": "cables", "sheet": "S"}, "row": 3}),
        json!({"op": "add_column", "target": {"id": "S/cables"}, "width": 20}),
        json!({"op": "delete_column", "target": {"id": "S/cables"}, "column": 0}),
        json!({"op": "layer", "sheet": "S", "name": "markup", "color": "red", "plot": false}),
        json!({"op": "change_layer", "sheet": "S", "name": "markup", "set": {"locked": true}}),
        json!({"op": "rename", "sheet": "S", "name": "markup", "to": "redline"}),
        json!({"op": "LAYDEL", "template": "templates/ansi-b.yaml", "name": "redline"}),
        json!({"op": "add_linetype", "sheet": "S", "name": "DASHED", "pattern": [0.25, -0.125]}),
        json!({"op": "change_linetype", "sheet": "S", "name": "DASHED", "set": {"description": "Dashes"}}),
        json!({"op": "rename_linetype", "sheet": "S", "name": "DASHED", "to": "HIDDEN"}),
        json!({"op": "delete_linetype", "template": "templates/ansi-b.yaml", "name": "HIDDEN"}),
        json!({"op": "STYLE", "sheet": "S", "name": "tags", "font": "romans.shx"}),
        json!({"op": "change_text_style", "sheet": "S", "name": "tags", "set": {"width_factor": 0.8}}),
        json!({"op": "rename_text_style", "template": "t.yaml", "name": "tags", "to": "labels"}),
        json!({"op": "delete_text_style", "sheet": "S", "name": "labels"}),
        json!({"op": "BLOCK", "name": "BADGE", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1}}, {"attdef": {"tag": "CODE", "position": [0, 0, 0], "height": 1, "cell": [[0, 0, 0], [1, 1, 0]]}}]}),
        json!({"op": "BEDIT", "library": "blocks.yaml", "name": "BADGE", "base_point": [1, 0, 0], "objects": [{"text": {"contents": "A", "position": [0, 0, 0], "height": 1, "cell": [[0, 0, 0], [1, 1, 0]]}}]}),
        json!({"op": "ATTSYNC", "name": "BADGE"}),
        json!({"op": "adcenter", "library": "blocks.yaml", "from": "../parts/badges.dxf", "blocks": ["BADGE"], "replace": true, "units": "in"}),
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
    assert!(whole.is_valid(&json!({"operations": samples.to_vec(), "fixes": ["DEMO-002"]})));
    for wrong in [
        json!({"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [1, 1, 0]}]}),
        json!({"operations": [{"op": "add_line", "sheet": "S", "template": "t.yaml", "from": [0, 0, 0], "to": [1, 1, 0]}]}),
        json!({"operations": [{"op": "change", "target": {"id": "S/a"}, "set": {"id": "b"}}]}),
        json!({"operations": [{"op": "move", "target": {"id": "S/a", "name": "n"}, "by": [1, 0, 0]}]}),
        json!({"operations": [{"op": "move", "target": {"id": "S/a"}, "by": [1, 0]}]}),
        json!({"operations": [{"op": "nudge", "target": {"id": "S/a"}}]}),
        json!({"operations": [], "undo": true}),
        json!({"operations": [{"op": "set_attribute", "target": {"id": "S/a"}, "tag": "NOTE", "occurrence": 0, "value": "B"}]}),
        json!({"operations": [{"op": "set_attribute", "target": {"id": "S/a"}, "tag": "NOTE", "occurrence": "2", "value": "B"}]}),
        json!({"operations": [{"op": "set_cell", "target": {"id": "S/a"}, "cell": [0], "set": {"text": "A"}}]}),
        json!({"operations": [{"op": "set_cell", "target": {"id": "S/a"}, "cell": [0, 0], "set": {"colour": "red"}}]}),
        json!({"operations": [{"op": "delete_row", "target": {"id": "S/a"}, "row": -1}]}),
        json!({"operations": [{"op": "add_column", "target": {"id": "S/a"}, "height": 2}]}),
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
        e.message
            .contains("from, to, layer, color, linetype, lineweight, linetype_scale, id, name"),
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
fn donuts_solids_and_wide_arc_polylines_are_added_and_moved() {
    let set = demo_set("arcs-widths");
    lands(
        &set,
        json!({"operations": [
            {"op": "DONUT", "sheet": "DEMO-003", "center": [5, 5, 0], "inside_diameter": 0, "outside_diameter": 0.1, "name": "dot"},
            {"op": "add_solid", "sheet": "DEMO-003", "corners": [[1, 1, 0], [2, 1, 0], [1.5, 2, 0]], "name": "flag"},
            {"op": "PLINE", "sheet": "DEMO-003", "points": [{"point": [3, 3, 0], "arc_angle": 180, "start_width": 0.05}, [4, 3, 0]], "name": "bend"},
        ]}),
    );
    lands(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"name": "dot"}, "by": [1, 0, 0]},
            {"op": "move", "target": {"name": "flag"}, "by": [0, 1, 0]},
            {"op": "move", "target": {"name": "bend"}, "by": [0, 0.5, 0]},
        ]}),
    );
    let on_sheet = built(&set, "DEMO-003");
    // The donut reads back from its DWG as the polyline it is written as.
    let Object::Polyline(dot) = &on_sheet["DEMO-003/donut-1"] else {
        panic!("{:?}", on_sheet["DEMO-003/donut-1"])
    };
    close(dot.points[0].x, 6.025);
    close(dot.width, 0.05);
    let Object::Solid(flag) = &on_sheet["DEMO-003/solid-1"] else {
        panic!()
    };
    close(flag.corners[2].y, 3.0);
    let Object::Polyline(bend) = &on_sheet["DEMO-003/polyline-1"] else {
        panic!()
    };
    close(bend.points[0].y, 3.5);
    close(bend.bulge(0), 1.0);
    assert_eq!(bend.widths_at(0), [0.05, 0.0]);
    assert!(row_text(&set, "DEMO-003")
        .contains("{point: [3, 3.5, 0], arc_angle: 180, start_width: 0.05}"));
}

#[test]
fn a_table_is_added_and_moved() {
    let set = demo_set("table");
    lands(
        &set,
        json!({"operations": [
            {"op": "TABLE", "sheet": "DEMO-003", "position": [1, 8, 0], "columns": [1, 2], "rows": [0.4, 0.4], "cells": [["TAG", "SERVICE"], ["P-1", {"text": "PUMP", "justify": "middle_left"}]], "header": true, "name": "list"},
        ]}),
    );
    lands(
        &set,
        json!({"operations": [{"op": "move", "target": {"name": "list"}, "by": [0.5, -1, 0]}]}),
    );
    let Object::Table(list) = &built(&set, "DEMO-003")["DEMO-003/table-1"] else {
        panic!()
    };
    close(list.position.x, 1.5);
    close(list.position.y, 7.0);
    assert!(list.header && !list.title);
    assert_eq!(list.cells[1][1].text, "PUMP");
    assert!(row_text(&set, "DEMO-003").contains("position: [1.5, 7, 0]"));
}

#[test]
fn change_sets_an_objects_own_properties_and_a_null_gives_back_its_layers() {
    let set = demo_set("own-properties");
    lands(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-003", "center": [5, 5, 0], "radius": 0.5, "name": "spot", "color": "red", "lineweight": 0.5},
        ]}),
    );
    let spot = |set: &Path| {
        let Object::Circle(spot) = built(set, "DEMO-003")["DEMO-003/circle-1"].clone() else {
            panic!()
        };
        spot.properties
    };
    assert_eq!(spot(&set).color, ObjectColor::Index(1));
    lands(
        &set,
        json!({"operations": [
            {"op": "change", "target": {"name": "spot"}, "set": {"color": "ByBlock", "linetype_scale": 2, "lineweight": null}},
        ]}),
    );
    let changed = spot(&set);
    assert_eq!(changed.color, ObjectColor::ByBlock);
    assert_eq!(changed.lineweight, ObjectLineweight::ByLayer);
    assert_eq!(changed.linetype_scale, 2.0);
    let reply = apply(
        &set,
        json!({"operations": [{"op": "change", "target": {"name": "spot"}, "set": {"color": 300}}]}),
    );
    assert!(!reply.ok);
    let report = reply.result.unwrap();
    let e = report
        .operations
        .into_iter()
        .find(|o| !o.ok)
        .unwrap()
        .error
        .unwrap();
    assert_eq!(e.field.as_deref(), Some("operations[0].set.color"));
    assert!(e.message.contains("ByLayer"), "{}", e.message);
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

/// A folder no one may write in holds the mark, and the journal and its
/// lock, which a change set takes before it writes anything: only Unix
/// keeps a folder from being written in this way. The commit's own tests
/// fail the mark's step on every system.
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
        error.message.contains("the set's journal lock"),
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

#[test]
fn an_add_reads_a_null_as_a_sheet_description_reads_a_bare_null() {
    let set = demo_set("nulls");
    // On an optional text or path field a null leaves the key out: an id
    // so sent is given one, and a layer, a style and an attribute's value
    // take their defaults.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "id": null, "name": null, "layer": null},
            {"op": "add_text", "sheet": "DEMO-002", "contents": "NOTE", "position": [1, 8, 0], "height": 0.2, "style": null},
            {"op": "insert_block", "sheet": "DEMO-002", "block": "TITLE_BLOCK", "position": [8, 3, 0], "attributes": {"TITLE": "T", "REV": null}},
        ]}),
    );
    let ids: Vec<&str> = report
        .operations
        .iter()
        .map(|o| o.id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["DEMO-002/circle-1", "DEMO-002/text-1", "DEMO-002/insert-1"]
    );
    let on_sheet = recorded(&set);
    let Object::Circle(circle) = &on_sheet["DEMO-002/circle-1"] else {
        panic!()
    };
    assert_eq!(circle.layer, "0");
    let Object::Text(text) = &on_sheet["DEMO-002/text-1"] else {
        panic!()
    };
    assert_eq!(text.style, "Standard");
    let Object::Insert(insert) = &on_sheet["DEMO-002/insert-1"] else {
        panic!()
    };
    let rev = insert.attributes.iter().find(|a| a.tag == "REV").unwrap();
    assert_eq!(rev.value, "0", "the definition's default");
    let in_dwg = built(&set, "DEMO-002");
    for id in &ids {
        assert!(in_dwg.contains_key(*id), "{id} is built");
    }
    // The record writes each null as the bare null it stands for.
    let row = row_text(&set, "DEMO-002");
    assert!(
        row.contains(
            "- circle: {center: [5, 5, 0], radius: 0.5, layer: null, id: circle-1, name: null}\n"
        ),
        "{row}"
    );
    assert!(row.contains("style: null"), "{row}");
    assert!(row.contains("attributes: {REV: null, TITLE: T}"), "{row}");

    // On a required one, or on a key that is not text, it is an error, as
    // in a sheet description, and the sheet is left as it was.
    let before = snapshot(&set);
    for (op, code, field) in [
        (
            json!({"op": "add_text", "sheet": "DEMO-002", "contents": null, "position": [1, 8, 0], "height": 0.2}),
            ErrorCode::MissingKey,
            "operations[0].contents",
        ),
        (
            json!({"op": "insert_block", "sheet": "DEMO-002", "block": null, "position": [8, 3, 0]}),
            ErrorCode::MissingKey,
            "operations[0].block",
        ),
        (
            json!({"op": "add_polyline", "sheet": "DEMO-002", "points": [[0, 0, 0], [1, 0, 0]], "closed": null}),
            ErrorCode::WrongType,
            "operations[0].closed",
        ),
        (
            json!({"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": null}),
            ErrorCode::BadNumber,
            "operations[0].radius",
        ),
    ] {
        let reply = apply(&set, json!({"operations": [op]}));
        assert!(!reply.ok, "{op}");
        let e = reply.result.unwrap().sheets[0].error.clone().unwrap();
        assert_eq!(e.code, code, "{op}: {e:?}");
        assert_eq!(e.field.as_deref(), Some(field), "{op}: {e:?}");
        assert_eq!(snapshot(&set), before, "{op}");
    }
}

#[test]
fn an_adds_schema_takes_exactly_what_the_add_takes() {
    let set = demo_set("add-schema");
    let listed = operations();
    let on_sheet = |mut op: Value| {
        op["sheet"] = json!("DEMO-002");
        op
    };
    let with = |mut op: Value, key: &str, value: Value| {
        op[key] = value;
        on_sheet(op)
    };
    let circle = json!({"op": "add_circle", "center": [5, 5, 0], "radius": 0.5});
    let text = json!({"op": "add_text", "contents": "A", "position": [1, 8, 0], "height": 0.2});
    let mtext = json!({"op": "add_mtext", "contents": "A", "position": [1, 7, 0], "height": 0.1});
    let insert = json!({"op": "insert_block", "block": "TITLE_BLOCK", "position": [8, 3, 0]});
    let hatch = json!({"op": "add_hatch", "pattern": "SOLID", "boundary": [{"polyline": {"points": [[3, 6, 0], [4, 6, 0], [4, 7, 0]]}}]});
    let polyline = json!({"op": "add_polyline", "points": [[0, 0, 0], [1, 0, 0]]});
    let cases = [
        // A null leaves an optional text or path field out.
        (with(circle.clone(), "id", Value::Null), true),
        (with(circle.clone(), "name", Value::Null), true),
        (with(circle.clone(), "layer", Value::Null), true),
        (with(text.clone(), "style", Value::Null), true),
        (
            with(insert.clone(), "attributes", json!({"REV": null})),
            true,
        ),
        // A required one so sent is an error, and so is a null on a key
        // that is not text.
        (with(text.clone(), "contents", Value::Null), false),
        (with(insert.clone(), "block", Value::Null), false),
        (with(hatch.clone(), "pattern", Value::Null), false),
        (with(circle.clone(), "radius", Value::Null), false),
        (with(text.clone(), "rotation", Value::Null), false),
        (with(polyline.clone(), "closed", Value::Null), false),
        // Quoted, null is text.
        (with(text.clone(), "contents", json!("null")), true),
        // Contents that do not show are an error.
        (with(text.clone(), "contents", json!("")), false),
        (with(text.clone(), "contents", json!(" ")), false),
        (with(mtext.clone(), "contents", json!("")), false),
        (with(mtext.clone(), "contents", json!("\n")), false),
    ];
    let mut disagreements = Vec::new();
    for (op, accepted) in cases {
        let info = listed
            .operations
            .iter()
            .find(|o| o.name == op["op"])
            .unwrap();
        let by_schema = jsonschema::validator_for(&info.schema)
            .unwrap()
            .is_valid(&op);
        let reply = dry_run(&set, json!({"operations": [op.clone()]}));
        let why = reply.error.as_ref().map(|e| e.message.clone()).or_else(|| {
            let sheet = reply.result.as_ref()?.sheets.first()?;
            sheet.error.as_ref().map(|e| e.message.clone())
        });
        if by_schema != accepted || reply.ok != accepted {
            disagreements.push(format!(
                "{op}: expected {}, the schema {}, the change set {} ({})",
                if accepted { "accepted" } else { "refused" },
                if by_schema { "accepts" } else { "refuses" },
                if reply.ok { "lands" } else { "does not land" },
                why.unwrap_or_default(),
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// Zach decided on 2026-11-13 how a change set edits a template's layers,
/// linetypes and text styles, and that it respects a locked layer.
#[test]
fn a_templates_layers_linetypes_and_styles_are_added_changed_renamed_and_deleted() {
    let set = demo_set("tables");
    let template = set.join("templates").join("ansi-b.yaml");
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "LAYER", "sheet": "DEMO-001", "name": "markup", "color": "red", "plot": false},
            {"op": "add_linetype", "template": "templates/ansi-b.yaml", "name": "DASHED", "pattern": [0.25, -0.125]},
            {"op": "change_layer", "sheet": "DEMO-002", "name": "markup", "set": {"linetype": "DASHED"}},
            {"op": "add_line", "sheet": "DEMO-003", "from": [1, 1, 0], "to": [2, 2, 0], "layer": "trim", "id": "cut"},
            {"op": "RENAME", "sheet": "DEMO-001", "name": "trim", "to": "trim-marks"},
            {"op": "rename_linetype", "sheet": "DEMO-001", "name": "DASHED", "to": "HIDDEN"},
            {"op": "STYLE", "sheet": "DEMO-001", "name": "tags", "font": "romans.shx"},
            {"op": "delete_text_style", "sheet": "DEMO-001", "name": "tags"},
        ]}),
    );
    assert_eq!(
        report.landed, 3,
        "a template edit reaches every sheet built from it"
    );
    let text = fs::read_to_string(&template).unwrap();
    assert!(
        text.contains("  trim-marks: {color: 8, lineweight: 0.13}\n"),
        "{text}"
    );
    assert!(
        text.contains("  markup: {color: red, linetype: HIDDEN, plot: false}\n"),
        "{text}"
    );
    assert!(
        text.contains("linetypes:\n  HIDDEN: {pattern: [0.25, -0.125]}\n"),
        "{text}"
    );
    assert!(!text.contains("tags"), "{text}");
    assert!(
        text.contains("closed: true, layer: trim-marks}"),
        "the template's object follows"
    );
    assert!(
        text.starts_with("# A made-up 11 x 17 inch sheet"),
        "the rest of the file is kept"
    );
    assert!(
        row_text(&set, "DEMO-003").contains("layer: trim-marks"),
        "a row's object follows the rename"
    );

    // A delete of what is still in use is refused, listing what uses it;
    // a drawing's own names are never renamed or deleted.
    let refused = |change_set: Value| {
        let reply = apply(&set, change_set);
        assert!(!reply.ok);
        let report = reply.result.unwrap();
        report
            .operations
            .into_iter()
            .find(|o| !o.ok)
            .unwrap()
            .error
            .unwrap()
    };
    let e = refused(
        json!({"operations": [{"op": "LAYDEL", "sheet": "DEMO-001", "name": "trim-marks"}]}),
    );
    assert_eq!(e.code, ErrorCode::InUse);
    assert!(
        e.message.contains("DEMO-001/objects[0]") && e.message.contains("DEMO-003/cut"),
        "{}",
        e.message
    );
    let e = refused(
        json!({"operations": [{"op": "delete_linetype", "sheet": "DEMO-001", "name": "HIDDEN"}]}),
    );
    assert_eq!(e.code, ErrorCode::InUse);
    assert!(e.message.contains("the layer markup"), "{}", e.message);
    let e = refused(
        json!({"operations": [{"op": "rename_text_style", "sheet": "DEMO-001", "name": "Standard", "to": "plain"}]}),
    );
    assert_eq!(e.code, ErrorCode::BadValue);
    let e =
        refused(json!({"operations": [{"op": "LAYER", "sheet": "DEMO-001", "name": "MARKUP"}]}));
    assert_eq!(e.code, ErrorCode::DuplicateName);

    // A locked layer's objects are not edited, and are once it is unlocked.
    lands(
        &set,
        json!({"operations": [{"op": "change_layer", "sheet": "DEMO-001", "name": "notes", "set": {"locked": true}}]}),
    );
    let e = refused(
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/heading"}, "by": [0.1, 0, 0]}]}),
    );
    assert_eq!(e.code, ErrorCode::Locked);
    assert_eq!(e.field.as_deref(), Some("operations[0].target"));
    assert!(e.message.contains("layer `notes`"), "{}", e.message);
    // A row's value that fills a text on the locked layer is not set
    // either, and its refusal reaches only that sheet.
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/objects[2]"}, "tag": "TITLE", "value": "NEW"},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.1},
        ]}),
    );
    let report = reply.result.unwrap();
    let e = report.operations[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::Locked);
    assert!(e.message.contains("DEMO-001/heading"), "{}", e.message);
    assert_eq!(report.landed, 1, "DEMO-002 takes its own circle");
    lands(
        &set,
        json!({"operations": [
            {"op": "add_text", "sheet": "DEMO-001", "contents": "ON A LOCKED LAYER", "position": [1, 9, 0], "height": 0.1, "layer": "notes"},
            {"op": "change_layer", "sheet": "DEMO-001", "name": "notes", "set": {"locked": null}},
            {"op": "move", "target": {"id": "DEMO-001/heading"}, "by": [0.1, 0, 0]},
        ]}),
    );

    // `Standard`, which the template does not write, is changed whole.
    lands(
        &set,
        json!({"operations": [{"op": "change_text_style", "sheet": "DEMO-001", "name": "Standard", "set": {"width_factor": 0.8}}]}),
    );
    let text = fs::read_to_string(&template).unwrap();
    assert!(
        text.contains(
            "  Standard: {font: txt.shx, width_factor: 0.8}
"
        ),
        "{text}"
    );
}

/// An attribute of a block nested in another is filled in once, in the
/// outer block's definition, as AutoCAD fills it: `set_attribute` on an
/// insert of the outer block is refused, saying where its value is set.
#[test]
fn set_attribute_on_a_nested_blocks_attribute_is_refused_saying_why() {
    let set = demo_set("set-nested-attribute");
    let library = set.join("blocks.yaml");
    let mut text = fs::read_to_string(&library).unwrap();
    text.push_str(
        "  STAMP:
    objects:
      - insert: {block: MARK, position: [0, 0, 0], attributes: {LETTER: A}}
  MARK:
    objects:
      - attdef: {tag: LETTER, position: [0, 0, 0], height: 0.2}
",
    );
    fs::write(&library, text).unwrap();
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-001", "block": "STAMP", "position": [2, 2, 0], "id": "stamp"}
        ]}),
    );
    let before = fs::read(set.join("rows.yaml")).unwrap();
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/stamp"}, "tag": "LETTER", "value": "B"}
        ]}),
    );
    let report = reply.result.expect("a report");
    assert_eq!(report.failed, 1, "{report:#?}");
    let text = format!("{report:?}");
    assert!(
        text.contains(
            "the attribute `LETTER` belongs to block `MARK`, which block `STAMP` inserts"
        ),
        "{text}"
    );
    assert!(
        text.contains("set once, in block `STAMP`'s definition"),
        "{text}"
    );
    assert_eq!(fs::read(set.join("rows.yaml")).unwrap(), before);
}

/// A constant attribute's value is its definition's: an insert of its
/// block carries no value for it, `set_attribute` on it is refused, saying
/// so, and a query for its text finds the insert, as it finds a block's own
/// text.
#[test]
fn a_constant_attribute_is_refused_to_set_attribute_and_found_by_its_text() {
    let set = demo_set("constant-attribute");
    let library = set.join("blocks.yaml");
    let mut text = fs::read_to_string(&library).unwrap();
    text.push_str(
        "  BADGE:
    objects:
      - attdef: {tag: CODE, position: [0, 0, 0], height: 0.2, default: ZONE-7, constant: true}
      - attdef: {tag: NOTE, position: [0, -0.3, 0], height: 0.1}
",
    );
    fs::write(&library, text).unwrap();
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-001", "block": "BADGE", "position": [2, 2, 0], "id": "badge"}
        ]}),
    );
    let Object::Insert(badge) = &recorded(&set)["DEMO-001/badge"] else {
        panic!("an insert")
    };
    let tags: Vec<&str> = badge.attributes.iter().map(|a| a.tag.as_str()).collect();
    assert_eq!(tags, ["NOTE"]);

    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/badge"}, "tag": "CODE", "value": "X"}
        ]}),
    );
    let report = reply.result.expect("a report");
    assert_eq!(report.failed, 1, "{report:#?}");
    let said = format!("{report:?}");
    assert!(
        said.contains("the attribute `CODE` of block `BADGE` is constant: its value, `ZONE-7`"),
        "{said}"
    );

    let found = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Find(Selection {
                text: Some("ZONE-7".into()),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = found.answer else {
        panic!("a find's answer")
    };
    assert_eq!(objects.len(), 1, "{objects:#?}");
    assert_eq!(objects[0].id, "DEMO-001/badge");
    assert_eq!(objects[0].block_text[0].field, "blocks.BADGE.objects[0]");
}

/// Builds the set again from its files, after a test edited them by hand.
fn rebuild(set: &Path) {
    let built = build_set_with(
        &SetBuildRequest {
            set: set.to_owned(),
            overwrite: true,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
}

/// Appends `text` to the file at `path`.
fn append(path: &Path, text: &str) {
    let mut all = fs::read_to_string(path).unwrap();
    all.push_str(text);
    fs::write(path, all).unwrap();
}

/// The error of the first operation of a change set that did not land.
fn failed_operation(set: &Path, change_set: Value) -> agentdrafter_core::Error {
    let reply = apply(set, change_set);
    assert!(!reply.ok);
    let report = reply.result.expect("a report");
    report
        .operations
        .into_iter()
        .find(|o| !o.ok)
        .expect("an operation that did not land")
        .error
        .expect("its error")
}

/// Zach decided on 2026-11-13 that a change set adds a block to a block
/// library with `add_block`, AutoCAD's BLOCK, which reaches every sheet
/// whose template names the library.
#[test]
fn add_block_adds_a_block_that_a_later_insert_inserts() {
    let set = demo_set("add-block");
    let library = set.join("blocks.yaml");
    let before = fs::read_to_string(&library).unwrap();
    let dwg_001 = fs::read(dwg(&set, "DEMO-001")).unwrap();
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "BLOCK", "library": "blocks.yaml", "name": "BADGE", "base_point": [0, 0, 0], "objects": [
                {"circle": {"radius": 0.5, "center": [0, 0, 0], "layer": "title"}},
                {"attdef": {"height": 0.1, "position": [0, -0.8, 0], "tag": "CODE", "default": "X-1"}}
            ]},
            {"op": "insert_block", "sheet": "DEMO-002", "block": "BADGE", "position": [4, 4, 0], "id": "badge", "attributes": {"CODE": "Z-9"}}
        ]}),
    );
    assert_eq!(report.landed, 3, "it reaches every sheet of the library");
    assert_eq!(report.operations[0].sheets, SHEETS);
    assert!(report.operations[0]
        .message
        .starts_with("adds the block `BADGE` to the block library `blocks.yaml`"));
    let after = fs::read_to_string(&library).unwrap();
    assert!(after.starts_with(&before), "every other byte is kept");
    assert_eq!(
        &after[before.len()..],
        "  BADGE:
    base_point: [0, 0, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 0.5, layer: title}
      - attdef: {tag: CODE, default: X-1, position: [0, -0.8, 0], height: 0.1}
"
    );
    assert!(report
        .records
        .iter()
        .any(|r| Path::new(r).ends_with("blocks.yaml")));
    // A sheet that does not insert it builds the same DWG, which is not
    // written again.
    assert_eq!(fs::read(dwg(&set, "DEMO-001")).unwrap(), dwg_001);
    assert!(!report.sheets[0].dwg_written);
    let Object::Insert(badge) = &built(&set, "DEMO-002")["DEMO-002/badge"] else {
        panic!("an insert")
    };
    assert_eq!(badge.block, "BADGE");
    assert_eq!(badge.attributes[0].tag, "CODE");
    assert_eq!(badge.attributes[0].value, "Z-9");
}

#[test]
fn a_block_operation_on_a_name_it_cannot_take_is_refused_listing_the_blocks() {
    let set = demo_set("block-names");
    let start = snapshot(&set);
    let e = failed_operation(
        &set,
        json!({"operations": [{"op": "add_block", "name": "title_block", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1}}]}]}),
    );
    assert_eq!(e.code, ErrorCode::DuplicateName);
    assert_eq!(e.field.as_deref(), Some("operations[0].name"));
    assert!(e.message.contains("`TITLE_BLOCK` already"), "{}", e.message);
    for op in [
        json!({"op": "redefine_block", "name": "NOPE", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1}}]}),
        json!({"op": "attsync", "name": "NOPE"}),
    ] {
        let e = failed_operation(&set, json!({"operations": [op]}));
        assert_eq!(e.code, ErrorCode::UnknownName);
        assert!(
            e.message
                .contains("has no block `NOPE`; its blocks: TITLE_BLOCK"),
            "{}",
            e.message
        );
    }
    assert_eq!(snapshot(&set), start, "nothing was written");
}

/// A made-up block the redefinition tests redefine, with three attributes.
const BADGE: &str = "  BADGE:
    objects:
      - circle: {center: [0, 0, 0], radius: 0.5}
      - attdef: {tag: CODE, position: [0, -0.8, 0], height: 0.1}
      - attdef: {tag: NOTE, position: [0, -1, 0], height: 0.1}
      - attdef: {tag: KEEP, position: [0, -1.2, 0], height: 0.1}
";

/// The demo set with `BADGE` in its library, inserted by its template,
/// which takes `CODE` from the row, and by DEMO-002's row.
fn set_with_badge(test: &str) -> PathBuf {
    let set = demo_set(test);
    append(&set.join("blocks.yaml"), BADGE);
    append(
        &set.join("templates").join("ansi-b.yaml"),
        "  - insert: {block: BADGE, position: [3, 3, 0], id: badge, attributes: {CODE: {row: REV}, NOTE: fixed, KEEP: k}}\n",
    );
    rebuild(&set);
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-002", "block": "BADGE", "position": [6, 3, 0], "id": "row-badge", "attributes": {"NOTE": "row note", "KEEP": "x"}}
        ]}),
    );
    set
}

/// Zach decided on 2026-11-13 that `redefine_block`, AutoCAD's BEDIT,
/// replaces a block's whole definition and brings every insert into line
/// as it lands: a value for a tag the block no longer carries is removed
/// from its record, the reply lists each, and `undo` brings them back.
#[test]
fn redefine_block_replaces_the_definition_and_removes_values_no_longer_defined() {
    let set = set_with_badge("redefine-block");
    let start = snapshot::snapshot(&set);
    let rows_before = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "BEDIT", "name": "BADGE", "objects": [
                {"circle": {"center": [0, 0, 0], "radius": 0.6}},
                {"attdef": {"tag": "CODE", "position": [0, -0.8, 0], "height": 0.1, "default": "C-1", "constant": true}},
                {"attdef": {"tag": "KEEP", "position": [0, -1.2, 0], "height": 0.1}},
                {"attdef": {"tag": "ADDED", "position": [0, -1.4, 0], "height": 0.1, "default": "new"}}
            ]}
        ]}),
    );
    let outcome = &report.operations[0];
    assert_eq!(
        outcome.inserts,
        [
            "DEMO-001/badge",
            "DEMO-002/badge",
            "DEMO-002/row-badge",
            "DEMO-003/badge"
        ]
    );
    let removed: Vec<(String, String, Vec<String>, String, String)> = outcome
        .attributes_removed
        .iter()
        .map(|r| {
            let file = Path::new(&r.record)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (
                file,
                r.field.clone(),
                r.ids.clone(),
                r.tag.clone(),
                r.value.clone(),
            )
        })
        .collect();
    let on_every_sheet: Vec<String> = SHEETS.iter().map(|s| format!("{s}/badge")).collect();
    assert_eq!(
        removed,
        [
            (
                "ansi-b.yaml".to_owned(),
                "objects[5]".to_owned(),
                on_every_sheet.clone(),
                "CODE".to_owned(),
                "{row: REV}".to_owned()
            ),
            (
                "ansi-b.yaml".to_owned(),
                "objects[5]".to_owned(),
                on_every_sheet,
                "NOTE".to_owned(),
                "fixed".to_owned()
            ),
            (
                "rows.yaml".to_owned(),
                "rows[1].objects[0]".to_owned(),
                vec!["DEMO-002/row-badge".to_owned()],
                "NOTE".to_owned(),
                "row note".to_owned()
            ),
        ]
    );
    assert!(
        outcome.message.starts_with(
            "redefines the block `BADGE` in the block library `blocks.yaml`, reaching 4 \
             inserts; removes 3 attribute values"
        ),
        "{}",
        outcome.message
    );
    let template = fs::read_to_string(set.join("templates").join("ansi-b.yaml")).unwrap();
    assert!(
        template.ends_with(
            "  - insert: {block: BADGE, position: [3, 3, 0], id: badge, attributes: {KEEP: k}}\n"
        ),
        "{template}"
    );
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    assert!(rows.contains("attributes: {KEEP: x}"), "{rows}");
    assert_eq!(
        rows.replace(
            "attributes: {KEEP: x}",
            "attributes: {KEEP: x, NOTE: row note}"
        ),
        rows_before,
        "only the value removed changes in the rows"
    );
    let library = fs::read_to_string(set.join("blocks.yaml")).unwrap();
    assert!(
        library.contains(
            "      - attdef: {tag: ADDED, default: new, position: [0, -1.4, 0], height: 0.1}\n"
        ),
        "{library}"
    );
    let Object::Insert(badge) = &built(&set, "DEMO-002")["DEMO-002/row-badge"] else {
        panic!("an insert")
    };
    let values: Vec<(&str, &str)> = badge
        .attributes
        .iter()
        .map(|a| (a.tag.as_str(), a.value.as_str()))
        .collect();
    assert_eq!(values, [("KEEP", "x"), ("ADDED", "new")]);

    // The journal marks every insert the redefinition reached as changed.
    let entry = report.journal_entry.unwrap();
    let journal = agentdrafter_core::journal(&JournalRequest { set: set.clone() })
        .result
        .unwrap();
    let recorded = &journal
        .entries
        .iter()
        .find(|e| e.recorded.entry == entry)
        .unwrap()
        .recorded;
    let changed: Vec<&str> = recorded.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(changed, outcome.inserts);
    assert!(recorded
        .objects
        .iter()
        .all(|o| o.how == HowChanged::Changed));

    // Undo puts the library, the template and the rows back, byte for byte.
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), start);
}

/// A redefinition the library cannot read fails every sheet whose template
/// names the library, with the error placed in the operation, and writes
/// nothing.
#[test]
fn a_redefinition_that_breaks_the_library_fails_every_sheet_and_names_its_field() {
    let set = set_with_badge("redefine-breaks");
    let start = snapshot(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "redefine_block", "name": "BADGE", "objects": [
                {"line": {"from": [0, 0, 0], "to": [1, 0, 0], "layer": "nope"}}
            ]}
        ]}),
    );
    assert!(!reply.ok);
    let report = reply.result.unwrap();
    assert_eq!(report.failed, 3);
    assert_eq!(report.landed, 0);
    let e = report.operations[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(
        e.field.as_deref(),
        Some("operations[0].objects[0].line.layer"),
        "{e:#?}"
    );
    assert!(
        e.message.starts_with("operation 0 (`redefine_block`)"),
        "{}",
        e.message
    );
    assert!(report.sheets.iter().all(|s| !s.ok));
    assert_eq!(snapshot(&set), start, "nothing was written");
}

/// Zach decided on 2026-11-13 that `attsync`, AutoCAD's ATTSYNC, brings
/// inserts into line alone, for a library edited by hand: a value for a tag
/// the block no longer defines is removed, a nested insert's in another
/// block of the library among them.
#[test]
fn attsync_brings_inserts_into_line_with_a_library_edited_by_hand() {
    let set = demo_set("attsync");
    let library = set.join("blocks.yaml");
    append(
        &library,
        "  MARK:
    objects:
      - attdef: {tag: LETTER, position: [0, 0, 0], height: 0.2}
      - attdef: {tag: OLD, position: [0, 0.3, 0], height: 0.2}
  STAMP:
    objects:
      - insert: {block: MARK, position: [0, 0, 0], attributes: {LETTER: A, OLD: gone}}
",
    );
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-001", "block": "MARK", "position": [2, 2, 0], "id": "mark", "attributes": {"LETTER": "B", "OLD": "x"}},
            {"op": "INSERT", "sheet": "DEMO-003", "block": "STAMP", "position": [3, 2, 0], "id": "stamp"}
        ]}),
    );
    // The library is edited by hand: MARK no longer defines OLD, so no
    // sheet builds until its inserts are brought into line.
    let text = fs::read_to_string(&library).unwrap();
    fs::write(
        &library,
        text.replace(
            "      - attdef: {tag: OLD, position: [0, 0.3, 0], height: 0.2}\n",
            "",
        ),
    )
    .unwrap();
    let report = lands(
        &set,
        json!({"operations": [{"op": "ATTSYNC", "name": "MARK"}]}),
    );
    assert_eq!(report.landed, 3);
    let outcome = &report.operations[0];
    // DEMO-003's STAMP nests MARK, so it is reached too, with no value of
    // its own to remove.
    assert_eq!(outcome.inserts, ["DEMO-001/mark", "DEMO-003/stamp"]);
    let removed: Vec<(&str, &[String], &str, &str)> = outcome
        .attributes_removed
        .iter()
        .map(|r| {
            (
                r.field.as_str(),
                r.ids.as_slice(),
                r.tag.as_str(),
                r.value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        removed,
        [
            ("blocks.STAMP.objects[0]", &[][..], "OLD", "gone"),
            (
                "rows[0].objects[0]",
                &["DEMO-001/mark".to_owned()][..],
                "OLD",
                "x"
            ),
        ]
    );
    let text = fs::read_to_string(&library).unwrap();
    assert!(
        text.ends_with(
            "  STAMP:
    objects:
      - insert: {block: MARK, position: [0, 0, 0], attributes: {LETTER: A}}
"
        ),
        "{text}"
    );
    assert!(row_text(&set, "DEMO-001").contains("attributes: {LETTER: B}"));
    // Every block, with nothing left to bring into line, removes nothing.
    let report = lands(&set, json!({"operations": [{"op": "attsync"}]}));
    assert!(report.operations[0].attributes_removed.is_empty());
    assert!(report.records.is_empty(), "{:?}", report.records);
}

/// A block operation names its library by its path in the set, which may
/// be left out when every template names the same library.
#[test]
fn a_block_operation_names_its_library_when_the_set_has_more_than_one() {
    let set = demo_set("two-libraries");
    let templates = set.join("templates");
    fs::copy(set.join("blocks.yaml"), set.join("blocks-2.yaml")).unwrap();
    let template = fs::read_to_string(templates.join("ansi-b.yaml")).unwrap();
    fs::write(
        templates.join("ansi-b-2.yaml"),
        template.replace(
            "block_library: ../blocks.yaml",
            "block_library: ../blocks-2.yaml",
        ),
    )
    .unwrap();
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let third = rows.find("  - sheet: DEMO-003").unwrap();
    fs::write(
        set.join("rows.yaml"),
        format!(
            "{}{}",
            &rows[..third],
            rows[third..].replace("templates/ansi-b.yaml", "templates/ansi-b-2.yaml")
        ),
    )
    .unwrap();
    rebuild(&set);

    let refused = |change_set: Value| {
        let reply = apply(&set, change_set);
        assert!(!reply.ok);
        assert!(reply.result.is_none(), "refused whole");
        reply.error.unwrap()
    };
    let e = refused(json!({"operations": [{"op": "attsync"}]}));
    assert_eq!(e.code, ErrorCode::MissingKey);
    assert_eq!(e.field.as_deref(), Some("operations[0].library"));
    assert!(
        e.message.contains("`blocks.yaml`, `blocks-2.yaml`"),
        "{}",
        e.message
    );
    let e = refused(json!({"operations": [{"op": "attsync", "library": "other.yaml"}]}));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].library"));
    assert!(
        e.message.contains("`blocks.yaml`, `blocks-2.yaml`"),
        "{}",
        e.message
    );

    let report = lands(
        &set,
        json!({"operations": [{"op": "add_block", "library": "blocks-2.yaml", "name": "BADGE", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1}}]}]}),
    );
    assert_eq!(report.operations[0].sheets, ["DEMO-003"]);
    assert!(fs::read_to_string(set.join("blocks-2.yaml"))
        .unwrap()
        .contains("  BADGE:\n"));
    assert!(!fs::read_to_string(set.join("blocks.yaml"))
        .unwrap()
        .contains("BADGE"));
}

/// An insert of a block that nests the redefined block, however deep, on a
/// template or in a row, draws as the new definition does: it is reached,
/// listed under `inserts`, and marked changed in the journal.
#[test]
fn a_redefinition_reaches_the_inserts_of_every_block_that_nests_it() {
    let set = demo_set("nested-reach");
    append(
        &set.join("blocks.yaml"),
        "  INNER:
    objects:
      - circle: {center: [0, 0, 0], radius: 0.2}
  OUTER:
    objects:
      - insert: {block: INNER, position: [0, 0, 0]}
  TOP:
    objects:
      - insert: {block: OUTER, position: [1, 0, 0]}
",
    );
    append(
        &set.join("templates").join("ansi-b.yaml"),
        "  - insert: {block: OUTER, position: [3, 3, 0], id: outer}\n",
    );
    rebuild(&set);
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-002", "block": "TOP", "position": [6, 3, 0], "id": "top"}
        ]}),
    );
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "redefine_block", "name": "INNER", "objects": [{"circle": {"center": [0, 0, 0], "radius": 0.4}}]}
        ]}),
    );
    let outcome = &report.operations[0];
    assert_eq!(
        outcome.inserts,
        [
            "DEMO-001/outer",
            "DEMO-002/outer",
            "DEMO-002/top",
            "DEMO-003/outer"
        ]
    );
    assert!(
        outcome.message.contains("reaching 4 inserts"),
        "{}",
        outcome.message
    );
    let journal = agentdrafter_core::journal(&JournalRequest { set: set.clone() })
        .result
        .unwrap();
    let recorded = &journal
        .entries
        .iter()
        .find(|e| Some(e.recorded.entry) == report.journal_entry)
        .unwrap()
        .recorded;
    let changed: Vec<&str> = recorded.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(changed, outcome.inserts);
}

/// The block schema takes exactly what the block library's reader takes:
/// a block's objects carry no `id` or `name`, which only a sheet's do.
#[test]
fn a_block_operations_schema_refuses_an_id_or_a_name_on_a_blocks_object() {
    let set = demo_set("block-object-ids");
    let listed = operations();
    let objects = [
        json!({"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}),
        json!({"circle": {"center": [0, 0, 0], "radius": 1}}),
        json!({"text": {"contents": "A", "position": [0, 0, 0], "height": 1}}),
        json!({"attdef": {"tag": "CODE", "position": [0, 0, 0], "height": 1}}),
        json!({"insert": {"block": "TITLE_BLOCK", "position": [0, 0, 0]}}),
    ];
    let library_schema =
        jsonschema::validator_for(&agentdrafter_core::block_library_schema()).unwrap();
    for op in ["add_block", "redefine_block"] {
        let info = listed.operations.iter().find(|o| o.name == op).unwrap();
        let validator = jsonschema::validator_for(&info.schema).unwrap();
        let name = if op == "add_block" {
            "BADGE"
        } else {
            "TITLE_BLOCK"
        };
        for object in &objects {
            let plain = json!({"op": op, "name": name, "objects": [object]});
            assert!(validator.is_valid(&plain), "{op} takes {plain}");
            for key in ["id", "name"] {
                let mut with = object.clone();
                let kind = with.as_object().unwrap().keys().next().unwrap().clone();
                with[&kind][key] = json!("x");
                let sent = json!({"op": op, "name": name, "objects": [with.clone()]});
                assert!(!validator.is_valid(&sent), "{op}'s schema takes {sent}");
                let reply = dry_run(&set, json!({"operations": [sent]}));
                assert!(!reply.ok, "the change set takes {with}");
                let library = json!({"units": "in", "blocks": {"B": {"objects": [with]}}});
                assert!(
                    !library_schema.is_valid(&library),
                    "the library's schema takes {library}"
                );
            }
        }
    }
}

/// An error in the block `A.B` is placed in the operation that wrote
/// `A.B`, not the one that wrote `A`.
#[test]
fn an_error_in_a_block_whose_name_holds_a_dot_is_placed_in_its_own_operation() {
    let set = demo_set("dotted-names");
    append(
        &set.join("blocks.yaml"),
        "  A:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
  A.B:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
",
    );
    rebuild(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "redefine_block", "name": "A.B", "objects": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0], "layer": "nope"}}]},
            {"op": "redefine_block", "name": "A", "objects": [{"circle": {"center": [0, 0, 0], "radius": 2}}]}
        ]}),
    );
    let report = reply.result.unwrap();
    let e = report.operations[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(
        e.field.as_deref(),
        Some("operations[0].objects[0].line.layer"),
        "{e:#?}"
    );
}

/// A name the library has in another case, by the reader's own rule for
/// case, is a duplicate, beyond ASCII too.
#[test]
fn add_block_refuses_a_name_the_library_has_in_another_case_beyond_ascii() {
    let set = demo_set("non-ascii-case");
    append(
        &set.join("blocks.yaml"),
        "  ÉQUIPE:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
",
    );
    rebuild(&set);
    let e = failed_operation(
        &set,
        json!({"operations": [{"op": "add_block", "name": "équipe", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1}}]}]}),
    );
    assert_eq!(e.code, ErrorCode::DuplicateName);
    assert_eq!(e.field.as_deref(), Some("operations[0].name"));
    assert!(e.message.contains("`ÉQUIPE` already"), "{}", e.message);
}

/// A block whose name reads as a place in another block, such as
/// `A.objects[0].line`, does not take the blame for an error in that other
/// block's own objects.
#[test]
fn an_error_in_a_block_is_not_placed_in_a_block_whose_name_reads_as_its_place() {
    let set = demo_set("place-like-names");
    append(
        &set.join("blocks.yaml"),
        "  A:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
  A.objects[0].line:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
",
    );
    rebuild(&set);
    let reply = apply(
        &set,
        json!({"operations": [
            {"op": "redefine_block", "name": "A", "objects": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0], "layer": "nope"}}]}
        ]}),
    );
    let report = reply.result.unwrap();
    let e = report.operations[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(
        e.field.as_deref(),
        Some("operations[0].objects[0].line.layer"),
        "{e:#?}"
    );
}

/// A made-up block that defines `NOTE` three times, as a plant's revision
/// block may, with tags that hold punctuation, one written in lowercase.
const NOTES: &str = "  NOTES:
    objects:
      - attdef: {tag: NOTE, position: [0, 0, 0], height: 0.1, default: '-'}
      - attdef: {tag: A/B, position: [0, -0.2, 0], height: 0.1}
      - attdef: {tag: NOTE, position: [0, -0.4, 0], height: 0.1, default: '-'}
      - attdef: {tag: x.y, position: [0, -0.6, 0], height: 0.1}
      - attdef: {tag: NOTE, position: [0, -0.8, 0], height: 0.1, default: '-'}
";

/// The demo set with `NOTES` in its library, inserted by its template,
/// which takes the first `NOTE` from the row, and by DEMO-001's row.
fn set_with_notes(test: &str) -> PathBuf {
    let set = demo_set(test);
    append(&set.join("blocks.yaml"), NOTES);
    append(
        &set.join("templates").join("ansi-b.yaml"),
        "  - insert: {block: NOTES, position: [5, 5, 0], id: revs, attributes: {NOTE: [{row: REV}, kept]}}\n",
    );
    rebuild(&set);
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-001", "block": "NOTES", "position": [2, 2, 0], "id": "n", "attributes": {"NOTE": ["a"], "X.Y": "xy"}}
        ]}),
    );
    set
}

/// The values the records give the attribute `tag` of the insert `id`, in
/// the order its block defines them.
fn values_of(set: &Path, id: &str, tag: &str) -> Vec<String> {
    let Object::Insert(insert) = &recorded(set)[id] else {
        panic!("an insert")
    };
    insert
        .attributes
        .iter()
        .filter(|a| a.tag == tag)
        .map(|a| a.value.clone())
        .collect()
}

/// Zach decided on 2026-11-13 (#99) that `set_attribute` names one of a
/// tag its block defines more than once by `occurrence`, counted from 1,
/// and fails without one, listing each with its value; a row's value fills
/// one item of the list, and a tag written in lowercase is taken in
/// capitals, with a notice.
#[test]
fn set_attribute_sets_one_occurrence_of_a_repeated_tag() {
    let set = set_with_notes("repeated-tag");
    assert_eq!(values_of(&set, "DEMO-001/n", "NOTE"), ["a", "-", "-"]);
    assert_eq!(values_of(&set, "DEMO-002/revs", "NOTE"), ["0", "kept", "-"]);

    // Without an occurrence, the operation lists them.
    let e = failed_operation(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "value": "c"}
        ]}),
    );
    assert_eq!(e.code, ErrorCode::MissingKey, "{e:?}");
    assert_eq!(e.field.as_deref(), Some("operations[0].occurrence"));
    assert!(
        e.message.contains(
            "block `NOTES` defines the attribute `NOTE` 3 times; name the one to set with \
             occurrence, counted from 1 in the order the block defines them: 1 `a`, 2 `-` (its \
             default), 3 `-` (its default)"
        ),
        "{}",
        e.message
    );
    for (tag, occurrence, words) in [
        ("NOTE", 4, "so its occurrence is 1 to 3, not 4"),
        (
            "A/B",
            2,
            "defines the attribute `A/B` once, so its occurrence is 1 or left out",
        ),
    ] {
        let e = failed_operation(
            &set,
            json!({"operations": [
                {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": tag, "occurrence": occurrence, "value": "c"}
            ]}),
        );
        assert_eq!(e.code, ErrorCode::BadValue, "{e:?}");
        assert_eq!(e.field.as_deref(), Some("operations[0].occurrence"));
        assert!(e.message.contains(words), "{}", e.message);
    }

    // Occurrence 3 grows the list, the second taking its default; tags with
    // punctuation are set as they are written, and one in lowercase in
    // capitals.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "occurrence": 3, "value": "c"},
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "A/B", "occurrence": 1, "value": "ab"},
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "x.y", "value": "XY"}
        ]}),
    );
    assert!(
        report.operations[0]
            .message
            .starts_with("sets occurrence 3 of the attribute NOTE of `DEMO-001/n` to `c`"),
        "{}",
        report.operations[0].message
    );
    assert!(
        report.operations[2]
            .message
            .contains("the attribute X.Y (written `x.y`, taken in capitals"),
        "{}",
        report.operations[2].message
    );
    assert_eq!(values_of(&set, "DEMO-001/n", "NOTE"), ["a", "-", "c"]);
    assert_eq!(values_of(&set, "DEMO-001/n", "A/B"), ["ab"]);
    assert_eq!(values_of(&set, "DEMO-001/n", "X.Y"), ["XY"]);
    assert!(
        row_text(&set, "DEMO-001").contains("X.Y: XY"),
        "{}",
        row_text(&set, "DEMO-001")
    );
    // The library's `x.y` is taken in capitals, and the sheet's reply says
    // where.
    let sheet = &report.sheets[0];
    assert_eq!(sheet.sheet, "DEMO-001");
    let notice = sheet
        .notices
        .iter()
        .find(|n| n.code == agentdrafter_core::NoticeCode::TagInCapitals)
        .expect("a notice of the lowercase tag");
    assert_eq!(notice.field, "blocks.NOTES.objects[3].attdef.tag");
    assert!(notice.file.ends_with("blocks.yaml"), "{notice:?}");
    // The DWG holds each value as its own attribute, in order.
    let Object::Insert(written) = &built(&set, "DEMO-001")["DEMO-001/n"] else {
        panic!("an insert")
    };
    let values: Vec<&str> = written
        .attributes
        .iter()
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(values, ["a", "ab", "-", "XY", "c"]);

    // The template takes occurrence 1 from the row, so setting it sets that
    // sheet's row value; occurrence 2 the template writes, so every sheet
    // built from it changes.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-002/revs"}, "tag": "NOTE", "occurrence": 1, "value": "B"},
            {"op": "set_attribute", "target": {"id": "DEMO-002/revs"}, "tag": "NOTE", "occurrence": 2, "value": "every"}
        ]}),
    );
    let row_value = report.operations[0]
        .row_value
        .as_ref()
        .expect("a row value");
    assert_eq!(row_value.sheet, "DEMO-002");
    assert!(
        report.operations[0]
            .message
            .contains("since its template takes occurrence 1 of the attribute NOTE"),
        "{}",
        report.operations[0].message
    );
    assert_eq!(
        values_of(&set, "DEMO-002/revs", "NOTE"),
        ["B", "every", "-"]
    );
    assert_eq!(
        values_of(&set, "DEMO-003/revs", "NOTE"),
        ["A", "every", "-"]
    );
    assert!(row_text(&set, "DEMO-002").contains("REV: B"));
}

/// Zach decided on 2026-11-13 (#99) that bringing inserts into line keeps
/// a repeated tag's values by their occurrence: a tag the block now
/// defines fewer times loses its list's surplus items, each listed with
/// its occurrence; a tag it now defines once takes its list's first item
/// alone, and one it defines more than once again takes a single value as
/// its list's first.
#[test]
fn redefining_a_repeated_tag_keeps_its_values_by_occurrence() {
    let set = set_with_notes("repeated-redefine");
    lands(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "occurrence": 3, "value": "c"}
        ]}),
    );
    let note = |y: f64| json!({"attdef": {"tag": "NOTE", "position": [0, y, 0], "height": 0.1, "default": "-"}});
    // Twice: the row's third value goes.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "BEDIT", "name": "NOTES", "objects": [note(0.0), note(-0.4)]}
        ]}),
    );
    let removed: Vec<(String, Option<usize>, String)> = report.operations[0]
        .attributes_removed
        .iter()
        .map(|r| (r.tag.clone(), r.occurrence, r.value.clone()))
        .collect();
    assert_eq!(
        removed,
        [
            ("NOTE".to_owned(), Some(3), "c".to_owned()),
            ("X.Y".to_owned(), None, "xy".to_owned()),
        ]
    );
    assert!(
        report.operations[0]
            .message
            .contains("NOTE occurrence 3 (`c`) of `DEMO-001/n`"),
        "{}",
        report.operations[0].message
    );
    assert_eq!(values_of(&set, "DEMO-001/n", "NOTE"), ["a", "-"]);
    // Once: the template's second value goes, and the row's list is one
    // value.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "BEDIT", "name": "NOTES", "objects": [note(0.0)]}
        ]}),
    );
    let removed: Vec<(String, Option<usize>, String)> = report.operations[0]
        .attributes_removed
        .iter()
        .map(|r| (r.tag.clone(), r.occurrence, r.value.clone()))
        .collect();
    assert_eq!(removed, [("NOTE".to_owned(), Some(2), "kept".to_owned())]);
    let template = fs::read_to_string(set.join("templates").join("ansi-b.yaml")).unwrap();
    assert!(
        template.contains("attributes: {NOTE: {row: REV}}"),
        "{template}"
    );
    assert!(
        row_text(&set, "DEMO-001").contains("NOTE: a"),
        "{}",
        row_text(&set, "DEMO-001")
    );
    assert_eq!(values_of(&set, "DEMO-002/revs", "NOTE"), ["0"]);
    // Twice again: each single value becomes its list's first.
    let report = lands(
        &set,
        json!({"operations": [
            {"op": "BEDIT", "name": "NOTES", "objects": [note(0.0), note(-0.4)]}
        ]}),
    );
    assert!(report.operations[0].attributes_removed.is_empty());
    assert_eq!(values_of(&set, "DEMO-001/n", "NOTE"), ["a", "-"]);
    assert_eq!(values_of(&set, "DEMO-003/revs", "NOTE"), ["A", "-"]);
}

/// An error in one occurrence of a repeated tag is laid on the operation
/// that set that occurrence, not on a later one that set another; and an
/// occurrence written `2.0`, a whole number as JSON Schema's integer takes
/// it, is read as 2.
#[test]
fn an_occurrences_error_is_laid_on_its_own_operation_and_a_whole_float_is_taken() {
    let set = set_with_notes("occurrence-blame");
    let e = failed_operation(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "occurrence": 1, "value": "bad\nvalue"},
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "occurrence": 2, "value": "good"}
        ]}),
    );
    assert!(
        e.field
            .as_deref()
            .is_some_and(|f| f.starts_with("operations[0].")),
        "{e:#?}"
    );
    lands(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/n"}, "tag": "NOTE", "occurrence": 2.0, "value": "two"}
        ]}),
    );
    assert_eq!(values_of(&set, "DEMO-001/n", "NOTE"), ["a", "two", "-"]);
}

/// A tag written with brackets, such as `A[0]`, is a tag of its own: an
/// error in the tag `A` is not laid on an edit of `A[0]`.
#[test]
fn an_error_in_a_tag_is_not_laid_on_an_edit_of_a_bracketed_tag() {
    let set = demo_set("bracketed-tag");
    append(
        &set.join("blocks.yaml"),
        "  BRACKETS:
    objects:
      - attdef: {tag: 'A[0]', position: [0, 0, 0], height: 0.1}
",
    );
    rebuild(&set);
    lands(
        &set,
        json!({"operations": [
            {"op": "INSERT", "sheet": "DEMO-001", "block": "BRACKETS", "position": [2, 2, 0], "id": "b"}
        ]}),
    );
    let e = failed_operation(
        &set,
        json!({"operations": [
            {"op": "set_attribute", "target": {"id": "DEMO-001/b"}, "tag": "A", "value": "bad"},
            {"op": "set_attribute", "target": {"id": "DEMO-001/b"}, "tag": "A[0]", "value": "good"}
        ]}),
    );
    assert!(
        e.field
            .as_deref()
            .is_some_and(|f| f.starts_with("operations[0].")),
        "{e:#?}"
    );
}

/// A template's table written in braces with a comment is not written
/// again an entry to a line, which would lose the comment: the operation
/// that adds to it fails `write_failed`, and nothing is written.
#[test]
fn a_table_in_braces_with_a_comment_takes_no_entry_and_keeps_its_comment() {
    let set = demo_set("braces-comment");
    let template = set.join("templates").join("ansi-b.yaml");
    let text = fs::read_to_string(&template).unwrap().replace(
        "layers:\n  trim: {color: 8, lineweight: 0.13}\n  border: {color: white, lineweight: 0.7}\n  notes: {color: cyan}\n",
        "layers: {trim: {color: 8, lineweight: 0.13}, border: {color: white, lineweight: 0.7}, notes: {color: cyan}}  # the client's layers\n",
    );
    assert!(text.contains("# the client's layers"));
    fs::write(&template, &text).unwrap();
    let before = snapshot(&set);

    let reply = apply(
        &set,
        json!({"operations": [{"op": "add_layer", "template": "templates/ansi-b.yaml", "name": "markup", "color": "red"}]}),
    );

    assert!(!reply.ok);
    let error = reply
        .result
        .as_ref()
        .and_then(|r| r.operations[0].error.clone())
        .or(reply.error.clone())
        .unwrap();
    assert_eq!(error.code, ErrorCode::WriteFailed, "{error:?}");
    assert!(
        error
            .message
            .contains("layers are written in braces with a comment"),
        "{error:?}"
    );
    assert_eq!(fs::read_to_string(&template).unwrap(), text);
    assert_eq!(snapshot(&set), before, "nothing was written");
}

/// A `#` inside a quoted value of a table in braces is no comment, even
/// once the change set replaces that value.
#[test]
fn a_table_in_braces_whose_quoted_value_holds_a_hash_is_changed() {
    let set = demo_set("braces-quoted-hash");
    let template = set.join("templates").join("ansi-b.yaml");
    let text = fs::read_to_string(&template).unwrap().replace(
        "layers:
  trim: {color: 8, lineweight: 0.13}
  border: {color: white, lineweight: 0.7}
  notes: {color: cyan}
",
        "layers: {trim: {color: 8, lineweight: 0.13}, border: {color: white, lineweight: 0.7}, notes: {color: cyan, description: 'see #2'}}
",
    );
    assert!(text.contains("'see #2'"));
    fs::write(&template, &text).unwrap();

    let report = lands(
        &set,
        json!({"operations": [{"op": "change_layer", "template": "templates/ansi-b.yaml", "name": "notes", "set": {"description": "new"}}]}),
    );

    assert_eq!(report.landed, 3);
    let written = fs::read_to_string(&template).unwrap();
    assert!(
        written.contains(
            "  notes: {color: cyan, description: new}
"
        ),
        "{written}"
    );
}
