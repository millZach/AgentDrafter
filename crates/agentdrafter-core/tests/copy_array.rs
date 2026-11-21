//! Copy and array (#66): `copy`, AutoCAD's COPY, and `array`, its ARRAY,
//! ARRAYRECT, ARRAYPOLAR and ARRAYPATH, as plain copies (Zach,
//! 2026-11-21). Each copy is an ordinary object with an ID of its own,
//! written where its source is, keeping its source's keys, an insert's
//! attribute values among them, but its name. All on a made-up set.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, query_with, undo, Answer, ChangeReport,
    ChangeRequest, DisplayListRequest, ErrorCode, HowChanged, Object, Query, QueryRequest, Reply,
    Selection, SetBuildRequest, Settings, UndoRequest,
};
use serde_json::{json, Value};

#[path = "support/snapshot.rs"]
mod snapshot;

const TEMPLATE: &str = "units: mm
block_library: ../blocks.yaml
layers:
  symbols: {color: yellow}
  fixed: {locked: true}
objects:
  - insert: {id: term, name: first-terminal, block: TERM, position: [20, 50, 0], attributes: {NUMBER: '1'}}
  - line: {id: rail, from: [20, 20, 0], to: [180, 20, 0]}
  - circle: {id: ring, center: [150, 70, 0], radius: 20, layer: symbols}
  - polyline: {id: bus, points: [[20, 40, 0], {point: [60, 40, 0], bulge: 1}, [100, 40, 0]]}
  - text: {id: title, contents: {row: TITLE}, position: [10, 110, 0], height: 4}
  - table: {id: sched, position: [120, 140, 0], columns: [20, 20], rows: [6], cells: [[A, B]]}
";

/// Every kind of object a sheet's row may hold, each with an id, on the
/// first sheet's row; and on the second's a circle whose id a copy of the
/// template's ring would otherwise take.
const ROWS: &str = "rows:
  - sheet: ARR-001
    template: templates/strip.yaml
    values: {TITLE: STRIP ONE}
    objects:
      - line: {id: spoke, from: [100, 100, 0], to: [110, 100, 0]}
      - polyline: {id: wire, points: [[0, 0, 0], {point: [5, 0, 0], bulge: 0.5}, [5, 5, 0]], width: 0.5}
      - arc: {id: bend, center: [30, 30, 0], radius: 4, start_angle: 10, end_angle: 100}
      - circle: {id: hole, center: [100, 60, 0], radius: 3}
      - ellipse: {id: lens, center: [60, 70, 0], major_axis: [6, 0, 0], ratio: 0.5}
      - spline: {id: curl, control_points: [[0, 80, 0], [5, 85, 0], [10, 80, 0], [15, 85, 0]]}
      - hatch: {id: fill, pattern: SOLID, boundary: [{polyline: {points: [[40, 0, 0], [45, 0, 0], [45, 5, 0]]}}]}
      - text: {id: note, contents: NOTE 1, position: [70, 10, 0], height: 2.5, rotation: 15}
      - mtext: {id: memo, contents: \"LINE ONE\\nLINE TWO\", position: [70, 90, 0], height: 2.5}
      - donut: {id: dot, center: [90, 30, 0], outside_diameter: 2}
      - solid: {id: pad, corners: [[120, 0, 0], [125, 0, 0], [125, 5, 0]]}
      - insert: {id: lug, block: TERM, position: [10, 60, 0], rotation: 30, attributes: {NUMBER: '7'}}
      - table: {id: list, position: [10, 150, 0], columns: [10], rows: [5], cells: [[X]]}
      - mleader: {id: callout, leaders: [[[130, 100, 0], [140, 105, 0]]], content: {text: TB-1}}
      - leader: {id: arrow, points: [[150, 100, 0], [160, 110, 0]], annotation: {text: SEE NOTE}}
      - dimension: {id: width, kind: linear, angle: 0, points: [[20, 20, 0], [180, 20, 0]], location: [100, 10, 0]}
      - circle: {id: pin, center: [5, 5, 0], radius: 1, layer: fixed}
  - sheet: ARR-002
    template: templates/strip.yaml
    values: {TITLE: STRIP TWO}
    objects:
      - circle: {id: ring-1, center: [10, 10, 0], radius: 1}
";

/// The made-up set, built, in a folder of its own under Cargo's scratch
/// folder: one template of two sheets, a block library of one terminal.
fn strip_set(test: &str) -> PathBuf {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-copy-array")
        .join(test);
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write("rows.yaml", ROWS);
    write(
        "blocks.yaml",
        "units: mm
blocks:
  TERM:
    objects:
      - polyline: {points: [[-2, -3, 0], [2, -3, 0], [2, 3, 0], [-2, 3, 0]], closed: true}
      - attdef: {tag: NUMBER, position: [0, 0, 0], height: 2, justify: middle_center}
",
    );
    write("templates/strip.yaml", TEMPLATE);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    set
}

fn send(set: &Path, operations: Value, dry_run: bool) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: json!({ "operations": operations }),
            dry_run,
        },
        &Settings::default(),
    )
}

/// Applies operations that land, and gives the reply.
fn lands(set: &Path, operations: Value) -> ChangeReport {
    let reply = send(set, operations, false);
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
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

fn insert(objects: &BTreeMap<String, Object>, id: &str) -> agentdrafter_core::Insert {
    match &objects[id] {
        Object::Insert(insert) => insert.clone(),
        other => panic!("{id} is {other:?}"),
    }
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-9)
}

fn xyz(p: agentdrafter_core::Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

#[test]
fn a_copy_of_each_kind_of_object_is_the_object_moved_with_an_id_of_its_own() {
    let ids = [
        "spoke", "wire", "bend", "hole", "lens", "curl", "fill", "note", "memo", "dot", "pad",
        "lug", "list", "callout", "arrow", "width",
    ];
    let copied = strip_set("each-kind-copied");
    let moved = strip_set("each-kind-moved");
    let targets: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": format!("ARR-001/{id}") }))
        .collect();
    let report = lands(
        &copied,
        json!([{"op": "COPY", "targets": targets, "by": [5, -2, 0]}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.op, "copy");
    assert_eq!(op.sheets, ["ARR-001"]);
    assert!(op.record.as_deref().unwrap().ends_with("rows.yaml"));
    let made: Vec<(&str, &str)> = op
        .copies
        .iter()
        .map(|c| (c.id.as_str(), c.source.as_str()))
        .collect();
    let wanted: Vec<(String, String)> = ids
        .iter()
        .map(|id| (format!("ARR-001/{id}-1"), format!("ARR-001/{id}")))
        .collect();
    let wanted: Vec<(&str, &str)> = wanted
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(made, wanted);
    assert!(op
        .copies
        .iter()
        .all(|c| c.offset == Some(0) && c.by == [5.0, -2.0, 0.0]));
    // Each copy is its source as `move` leaves it, on a set of its own.
    let moves: Vec<Value> = ids
        .iter()
        .map(
            |id| json!({"op": "move", "target": {"id": format!("ARR-001/{id}")}, "by": [5, -2, 0]}),
        )
        .collect();
    lands(&moved, Value::Array(moves));
    let (copies, sources) = (recorded(&copied), recorded(&moved));
    for id in ids {
        assert_eq!(
            copies[&format!("ARR-001/{id}-1")],
            sources[&format!("ARR-001/{id}")],
            "{id}"
        );
        // And the source stays where it was.
        assert_ne!(
            copies[&format!("ARR-001/{id}")],
            sources[&format!("ARR-001/{id}")]
        );
    }
}

#[test]
fn a_copy_keeps_an_inserts_attribute_values_but_not_its_name() {
    let set = strip_set("attributes");
    let report = lands(
        &set,
        json!([{"op": "copy", "target": {"name": "first-terminal", "sheet": "ARR-001"}, "offsets": [[10, 0, 0], [20, 0, 0]]}]),
    );
    let ids: Vec<&str> = report.operations[0]
        .copies
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["ARR-001/term-1", "ARR-001/term-2"]);
    let objects = recorded(&set);
    for (id, x) in [("term-1", 30.0), ("term-2", 40.0)] {
        for sheet in ["ARR-001", "ARR-002"] {
            let copy = insert(&objects, &format!("{sheet}/{id}"));
            assert_eq!(copy.attributes[0].tag, "NUMBER");
            assert_eq!(copy.attributes[0].value, "1");
            assert_eq!(xyz(copy.position), [x, 50.0, 0.0]);
        }
    }
    let template = fs::read_to_string(set.join("templates/strip.yaml")).unwrap();
    assert!(template.contains(
        "  - insert: {id: term-1, block: TERM, position: [30, 50, 0], attributes: {NUMBER: '1'}}"
    ), "{template}");
    assert_eq!(template.matches("first-terminal").count(), 1);
}

#[test]
fn a_rectangular_array_of_a_templates_terminal_lands_in_the_template_on_every_sheet() {
    let set = strip_set("rectangular");
    let rows_before = fs::read(set.join("rows.yaml")).unwrap();
    let report = lands(
        &set,
        json!([{"op": "ARRAYRECT", "target": {"id": "ARR-002/term"}, "rows": 2, "columns": 40, "row_spacing": -10, "column_spacing": "4 mm"}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.op, "array");
    assert_eq!(op.sheets, ["ARR-001", "ARR-002"]);
    assert_eq!(op.copies.len(), 79);
    assert!(
        op.message
            .contains("in the template `templates/strip.yaml`, so on"),
        "{}",
        op.message
    );
    // Row by row from the terminal's own: row 0's columns 1 to 39, then row 1.
    assert_eq!(op.copies[0].id, "ARR-002/term-1");
    assert_eq!((op.copies[0].row, op.copies[0].column), (Some(0), Some(1)));
    assert_eq!(
        (op.copies[39].row, op.copies[39].column),
        (Some(1), Some(0))
    );
    assert_eq!(op.copies[78].id, "ARR-002/term-79");
    let objects = recorded(&set);
    let last = insert(&objects, "ARR-001/term-79");
    assert_eq!(xyz(last.position), [20.0 + 39.0 * 4.0, 40.0, 0.0]);
    assert_eq!(last.attributes[0].value, "1");
    // The rows are untouched: the copies are the template's.
    assert_eq!(fs::read(set.join("rows.yaml")).unwrap(), rows_before);
    // Each copy's ID is its own, and on both sheets.
    let mut ids: Vec<&str> = op.copies.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 79);
    assert!(objects.contains_key("ARR-001/term-40") && objects.contains_key("ARR-002/term-40"));
}

#[test]
fn a_polar_array_turns_its_copies_round_the_centre_or_moves_them_unturned() {
    let set = strip_set("polar");
    let report = lands(
        &set,
        json!([
            {"op": "array", "kind": "polar", "target": {"id": "ARR-001/spoke"}, "center": {"snap": {"id": "ARR-001/spoke", "snap": "endpoint", "which": "from"}}, "count": 4},
            {"op": "ARRAYPOLAR", "target": {"id": "ARR-001/lug"}, "center": [10, 50, 0], "count": 3, "angle": 180},
            {"op": "arraypolar", "target": {"id": "ARR-001/note"}, "center": [70, 0, 0], "count": 2, "angle": 90, "rotate": false, "base_point": [70, 10, 0]},
        ]),
    );
    let spokes: Vec<(Option<usize>, Option<f64>)> = report.operations[0]
        .copies
        .iter()
        .map(|c| (c.item, c.turned))
        .collect();
    assert_eq!(
        spokes,
        [
            (Some(1), Some(90.0)),
            (Some(2), Some(180.0)),
            (Some(3), Some(270.0))
        ]
    );
    assert_eq!(report.operations[0].snapped.len(), 1);
    let objects = recorded(&set);
    let ends = |id: &str| match &objects[id] {
        Object::Line(line) => (xyz(line.from), xyz(line.to)),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        ends("ARR-001/spoke-1"),
        ([100.0, 100.0, 0.0], [100.0, 110.0, 0.0])
    );
    assert_eq!(
        ends("ARR-001/spoke-2"),
        ([100.0, 100.0, 0.0], [90.0, 100.0, 0.0])
    );
    assert_eq!(
        ends("ARR-001/spoke-3"),
        ([100.0, 100.0, 0.0], [100.0, 90.0, 0.0])
    );
    // Half a turn in 3 items: 90 apart, each insert turned with it.
    let lug = insert(&objects, "ARR-001/lug-2");
    assert!(
        close(xyz(lug.position), [10.0, 40.0, 0.0]),
        "{:?}",
        lug.position
    );
    assert_eq!(lug.rotation, 210.0);
    assert_eq!(lug.attributes[0].value, "7");
    // Unturned, the base point goes round and the text keeps its rotation.
    let Object::Text(note) = &objects["ARR-001/note-1"] else {
        panic!()
    };
    assert!(
        close(xyz(note.position), [60.0, 0.0, 0.0]),
        "{:?}",
        note.position
    );
    assert_eq!(note.rotation, 15.0);
}

#[test]
fn a_path_array_runs_along_a_line_a_circle_and_a_polyline_with_an_arc() {
    let set = strip_set("path");
    let report = lands(
        &set,
        json!([
            {"op": "ARRAYPATH", "target": {"id": "ARR-001/hole"}, "path": {"id": "ARR-001/rail"}, "count": 5},
            {"op": "array", "kind": "path", "target": {"id": "ARR-001/lug"}, "path": {"id": "ARR-001/ring"}, "count": 4},
            {"op": "arraypath", "target": {"id": "ARR-001/dot"}, "path": {"id": "ARR-001/bus"}, "count": 3},
            {"op": "arraypath", "target": {"id": "ARR-001/hole"}, "path": {"id": "ARR-001/rail"}, "count": 3, "spacing": 7, "align": false},
        ]),
    );
    let objects = recorded(&set);
    let center = |id: &str| match &objects[id] {
        Object::Circle(c) => xyz(c.center),
        Object::Donut(d) => xyz(d.center),
        other => panic!("{other:?}"),
    };
    // Along the line, 160 long: 40 apart, unturned by a straight path.
    assert_eq!(center("ARR-001/hole-4"), [260.0, 60.0, 0.0]);
    assert_eq!(report.operations[0].copies[3].along, Some(160.0));
    assert_eq!(report.operations[0].copies[3].turned, None);
    // Round the circle, from its angle 0: a quarter turn each, the insert
    // riding with it.
    let quarter = insert(&objects, "ARR-001/lug-1");
    assert_eq!(quarter.rotation, 120.0);
    assert!(
        close(xyz(quarter.position), [160.0, -70.0, 0.0]),
        "{:?}",
        quarter.position
    );
    // Along the polyline, its last item at its end, turned up with the arc.
    let last = &report.operations[2].copies[1];
    assert_eq!(last.by, [80.0, 0.0, 0.0]);
    assert_eq!(last.turned, Some(90.0));
    assert!(
        close(center("ARR-001/dot-2"), [110.0, 110.0, 0.0]),
        "{:?}",
        center("ARR-001/dot-2")
    );
    // At a spacing, and the IDs go on from those taken.
    assert_eq!(report.operations[3].copies[1].id, "ARR-001/hole-6");
    assert_eq!(center("ARR-001/hole-6"), [114.0, 60.0, 0.0]);
}

#[test]
fn a_copys_id_skips_one_taken_on_any_sheet_it_reaches_and_a_place_id_takes_its_type() {
    let set = strip_set("ids");
    // ARR-002's row has a ring-1 already, so the template's ring's copy,
    // which is on both sheets, is ring-2.
    let report = lands(
        &set,
        json!([
            {"op": "copy", "target": {"id": "ARR-001/ring"}, "by": [0, 30, 0]},
            {"op": "add_line", "sheet": "ARR-002", "from": [0, 0, 0], "to": [1, 0, 0], "id": null},
            {"op": "copy", "target": {"id": "ARR-002/ring-1"}, "by": [1, 0, 0]},
        ]),
    );
    assert_eq!(report.operations[0].copies[0].id, "ARR-001/ring-2");
    assert_eq!(report.operations[2].copies[0].id, "ARR-002/ring-1-1");
    let objects = recorded(&set);
    assert!(objects.contains_key("ARR-001/ring-2") && objects.contains_key("ARR-002/ring-2"));
    assert!(!objects.contains_key("ARR-001/ring-1-1"));
    // An object whose ID is its place gives its copy its type.
    fs::write(
        set.join("rows.yaml"),
        fs::read_to_string(set.join("rows.yaml"))
            .unwrap()
            .replace("      - circle: {id: ring-1, center: [10, 10, 0], radius: 1}\n", "      - circle: {center: [10, 10, 0], radius: 1}\n      - circle: {id: ring-1, center: [10, 10, 0], radius: 1}\n"),
    )
    .unwrap();
    let report = lands(
        &set,
        json!([{"op": "copy", "target": {"id": "ARR-002/rows[1].objects[0]"}, "by": [1, 0, 0]}]),
    );
    assert_eq!(report.operations[0].copies[0].id, "ARR-002/circle-1");
}

#[test]
fn a_row_objects_copy_lands_in_the_row_and_a_templates_in_the_template() {
    let set = strip_set("ownership");
    let template_before = fs::read(set.join("templates/strip.yaml")).unwrap();
    let report = lands(
        &set,
        json!([{"op": "array", "kind": "rectangular", "target": {"id": "ARR-001/hole"}, "columns": 3, "column_spacing": 8}]),
    );
    assert_eq!(report.operations[0].sheets, ["ARR-001"]);
    assert!(report.operations[0]
        .message
        .contains("in sheet `ARR-001`'s row"));
    assert_eq!(
        fs::read(set.join("templates/strip.yaml")).unwrap(),
        template_before
    );
    let sheets: Vec<&str> = report.sheets.iter().map(|s| s.sheet.as_str()).collect();
    assert_eq!(sheets, ["ARR-001"]);
    let report = lands(
        &set,
        json!([{"op": "copy", "targets": [{"id": "ARR-001/ring"}, {"id": "ARR-001/hole"}], "by": [0, -5, 0]}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.sheets, ["ARR-001", "ARR-002"]);
    assert!(
        op.message.contains("in the template") && op.message.contains("row"),
        "{}",
        op.message
    );
    let objects = recorded(&set);
    assert!(objects.contains_key("ARR-002/ring-2"));
    assert!(objects.contains_key("ARR-001/hole-3"));
    assert!(!objects.contains_key("ARR-002/hole-3"));
}

#[test]
fn a_copy_or_an_array_that_cannot_land_says_why_and_changes_nothing() {
    let set = strip_set("errors");
    let before = snapshot::snapshot(&set);
    let failed = |operation: Value| {
        let reply = send(&set, json!([operation]), false);
        assert!(!reply.ok);
        let report = reply.result.expect("a sheet failed");
        assert_eq!(report.landed, 0);
        report.operations[0].error.clone().unwrap()
    };
    let refused = |operation: Value| {
        let reply = send(&set, json!([operation]), false);
        assert!(!reply.ok && reply.result.is_none(), "{reply:#?}");
        reply.error.unwrap()
    };
    let e = failed(
        json!({"op": "copy", "targets": [{"id": "ARR-001/hole"}, {"id": "ARR-001/nope"}], "by": [1, 0, 0]}),
    );
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].targets[1].id"));
    let e = refused(
        json!({"op": "arraypolar", "target": {"id": "ARR-001/hole"}, "center": [0, 0, 0], "count": 0}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].count"))
    );
    let e = refused(
        json!({"op": "arrayrect", "target": {"id": "ARR-001/hole"}, "rows": 1, "columns": 1}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].rows"))
    );
    let e = refused(
        json!({"op": "arrayrect", "kind": "path", "target": {"id": "ARR-001/hole"}, "rows": 2}),
    );
    assert_eq!(e.code, ErrorCode::Usage);
    let e = failed(
        json!({"op": "arraypath", "target": {"id": "ARR-001/hole"}, "path": {"id": "ARR-001/note"}, "count": 3}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].path"))
    );
    assert!(
        e.message
            .contains("is a text; a path array runs along a line"),
        "{}",
        e.message
    );
    let e = failed(
        json!({"op": "arraypath", "target": {"id": "ARR-001/hole"}, "path": {"id": "ARR-001/rail"}, "count": 3, "spacing": 100}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].spacing"))
    );
    let e = failed(
        json!({"op": "arraypolar", "target": {"id": "ARR-001/list"}, "center": [0, 0, 0], "count": 3}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].target"))
    );
    assert!(e.message.contains("give rotate: false"), "{}", e.message);
    let e = failed(json!({"op": "copy", "target": {"id": "ARR-001/pin"}, "by": [1, 0, 0]}));
    assert_eq!(e.code, ErrorCode::Locked);
    let e = failed(
        json!({"op": "arraypath", "target": {"id": "ARR-001/term"}, "path": {"id": "ARR-001/hole"}, "count": 3}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].path"))
    );
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn undo_puts_the_set_back_byte_for_byte_and_a_dry_run_writes_nothing() {
    let set = strip_set("undo");
    let before = snapshot::snapshot(&set);
    let operations = json!([
        {"op": "arrayrect", "target": {"id": "ARR-001/term"}, "columns": 5, "column_spacing": 6},
        {"op": "arraypolar", "target": {"id": "ARR-001/spoke"}, "center": [100, 100, 0], "count": 6},
        {"op": "arraypath", "target": {"id": "ARR-001/lug"}, "path": {"id": "ARR-001/ring"}, "count": 5},
        {"op": "copy", "target": {"id": "ARR-001/hole"}, "by": [3, 3, 0]},
    ]);
    let dry = send(&set, operations.clone(), true).result.unwrap();
    assert!(dry.dry_run && dry.records.len() == 2);
    assert_eq!(dry.operations[0].copies.len(), 4);
    assert_eq!(snapshot::snapshot(&set), before);
    let entry = lands(&set, operations).journal_entry.unwrap();
    assert_ne!(snapshot::snapshot(&set), before);
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn a_template_array_is_drawn_before_it_lands_with_its_copies_added_on_every_sheet() {
    let set = strip_set("preview");
    let before = snapshot::snapshot(&set);
    let mut request = DisplayListRequest::new(&set);
    request.sheet = Some("ARR-002".to_owned());
    request.change_set = Some(json!({"operations": [
        {"op": "arrayrect", "target": {"id": "ARR-001/term"}, "columns": 3, "column_spacing": 6}
    ]}));
    let report = display_list_with(&request, &Settings::default()).unwrap();
    let preview = report.preview.as_ref().unwrap();
    let added: Vec<(&str, HowChanged)> = preview
        .objects
        .iter()
        .map(|o| (o.id.as_str(), o.how))
        .collect();
    assert_eq!(
        added,
        [
            ("ARR-001/term-1", HowChanged::Added),
            ("ARR-002/term-1", HowChanged::Added),
            ("ARR-001/term-2", HowChanged::Added),
            ("ARR-002/term-2", HowChanged::Added),
        ]
    );
    assert!(!preview.added.is_empty());
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn every_kind_a_turn_turns_is_turned_with_its_points_and_its_directions() {
    let set = strip_set("turned");
    let ids = [
        "spoke", "wire", "bend", "hole", "lens", "curl", "fill", "note", "memo", "dot", "pad",
        "lug",
    ];
    let targets: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": format!("ARR-001/{id}") }))
        .collect();
    let report = lands(
        &set,
        json!([{"op": "arraypolar", "targets": targets, "center": [0, 0, 0], "count": 2, "angle": 180}]),
    );
    assert_eq!(report.operations[0].copies.len(), ids.len());
    assert!(report.sheets.iter().all(|s| s.ok));
    let objects = recorded(&set);
    let Object::Arc(bend) = &objects["ARR-001/bend-1"] else {
        panic!()
    };
    assert!(close(xyz(bend.center), [-30.0, -30.0, 0.0]));
    assert_eq!((bend.start_angle, bend.end_angle), (190.0, 280.0));
    let Object::Ellipse(lens) = &objects["ARR-001/lens-1"] else {
        panic!()
    };
    assert!(
        close(xyz(lens.major_axis), [-6.0, 0.0, 0.0]),
        "{:?}",
        lens.major_axis
    );
    let Object::Text(note) = &objects["ARR-001/note-1"] else {
        panic!()
    };
    assert_eq!(note.rotation, 195.0);
    let Object::MText(memo) = &objects["ARR-001/memo-1"] else {
        panic!()
    };
    assert_eq!(memo.rotation, 180.0);
    assert_eq!(insert(&objects, "ARR-001/lug-1").rotation, 210.0);
    let Object::Polyline(wire) = &objects["ARR-001/wire-1"] else {
        panic!()
    };
    assert!(close(xyz(wire.points[1]), [-5.0, 0.0, 0.0]));
    assert_eq!(wire.bulges[1], 0.5);
    // A solid hatch takes no angle.
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let fill = rows.lines().find(|l| l.contains("id: fill-1")).unwrap();
    assert!(!fill.contains("angle"), "{fill}");
}
