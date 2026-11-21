//! Rotate, scale and mirror (#67): `rotate`, AutoCAD's ROTATE, `scale`,
//! its SCALE, and `mirror`, its MIRROR, which keeps its source and makes
//! mirrored copies unless told to erase it, with text reading forwards, as
//! MIRRTEXT 0 keeps it, and an insert mirrored, its X scale negative in the
//! DWG (#242). All on a made-up set.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, operations, query_with, read_dwg, undo, Answer,
    ChangeReport, ChangeRequest, DisplayListRequest, ErrorCode, HowChanged, Object, Query,
    QueryRequest, Reply, Selection, SetBuildRequest, Settings, UndoRequest,
};
use serde_json::{json, Value};

#[path = "support/snapshot.rs"]
mod snapshot;

const TEMPLATE: &str = "units: mm
block_library: ../blocks.yaml
layers:
  fixed: {locked: true}
objects:
  - insert: {id: term, block: TERM, position: [20, 50, 0], attributes: {NUMBER: '1'}}
  - text: {id: title, contents: {row: TITLE}, position: [10, 110, 0], height: 4}
";

const ROWS: &str = "rows:
  - sheet: ROT-001
    template: templates/strip.yaml
    values: {TITLE: STRIP ONE}
    objects:
      - line: {id: spoke, from: [100, 100, 0], to: [110, 100, 0]}
      - arc: {id: bend, center: [30, 30, 0], radius: 4, start_angle: 10, end_angle: 100}
      - text: {id: note, contents: NOTE 1, position: [70, 10, 0], height: 2.5, rotation: 15}
      - insert: {id: lug, block: TERM, position: [10, 60, 0], rotation: 30, attributes: {NUMBER: '7'}}
      - table: {id: list, position: [10, 150, 0], columns: [10], rows: [5], cells: [[X]]}
      - mleader: {id: callout, leaders: [[[130, 100, 0], [140, 105, 0]]], content: {text: TB-1}}
      - circle: {id: pin, center: [5, 5, 0], radius: 1, layer: fixed}
  - sheet: ROT-002
    template: templates/strip.yaml
    values: {TITLE: STRIP TWO}
";

/// The made-up set, built, in a folder of its own under Cargo's scratch
/// folder: one template of two sheets, a block library of one terminal.
fn strip_set(test: &str) -> PathBuf {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-transform")
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
      - arc: {center: [0, 3, 0], radius: 2, start_angle: 0, end_angle: 90}
      - attdef: {tag: NUMBER, position: [1, 0, 0], height: 2}
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

fn xyz(p: agentdrafter_core::Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

#[test]
fn rotate_scale_and_mirror_are_listed_with_their_autocad_aliases() {
    let listed = operations();
    for (name, alias, short) in [
        ("rotate", "ROTATE", "RO"),
        ("scale", "SCALE", "SC"),
        ("mirror", "MIRROR", "MI"),
    ] {
        let op = listed
            .operations
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"));
        assert_eq!(op.alias, alias);
        assert_eq!(op.other_aliases, [short]);
        assert!(
            op.does.contains(&format!("AutoCAD's {alias}")),
            "{}",
            op.does
        );
    }
}

#[test]
fn a_rotate_turns_a_rows_objects_in_place_exactly_keeping_their_ids() {
    let set = strip_set("rotate");
    let report = lands(
        &set,
        json!([{"op": "RO", "targets": [
            {"id": "ROT-001/spoke"}, {"id": "ROT-001/bend"}, {"id": "ROT-001/note"},
            {"id": "ROT-001/lug"}, {"id": "ROT-001/callout"}
        ], "base_point": [100, 100, 0], "angle": 90}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.op, "rotate");
    assert_eq!(op.sheets, ["ROT-001"]);
    assert!(op.record.as_deref().unwrap().ends_with("rows.yaml"));
    assert_eq!(
        op.transformed,
        [
            "ROT-001/spoke",
            "ROT-001/bend",
            "ROT-001/note",
            "ROT-001/lug",
            "ROT-001/callout"
        ]
    );
    assert!(
        op.message
            .starts_with("turns `ROT-001/spoke`, `ROT-001/bend`"),
        "{}",
        op.message
    );
    let objects = recorded(&set);
    let Object::Line(spoke) = &objects["ROT-001/spoke"] else {
        panic!("a line")
    };
    // Exactly, with no noise of a sine.
    assert_eq!(xyz(spoke.from), [100.0, 100.0, 0.0]);
    assert_eq!(xyz(spoke.to), [100.0, 110.0, 0.0]);
    let Object::Arc(bend) = &objects["ROT-001/bend"] else {
        panic!("an arc")
    };
    assert_eq!(xyz(bend.center), [170.0, 30.0, 0.0]);
    assert_eq!((bend.start_angle, bend.end_angle), (100.0, 190.0));
    let Object::Text(note) = &objects["ROT-001/note"] else {
        panic!("a text")
    };
    assert_eq!(xyz(note.position), [190.0, 70.0, 0.0]);
    assert_eq!(note.rotation, 105.0);
    let lug = insert(&objects, "ROT-001/lug");
    assert_eq!(xyz(lug.position), [140.0, 10.0, 0.0]);
    assert_eq!(lug.rotation, 120.0);
    let Object::MLeader(callout) = &objects["ROT-001/callout"] else {
        panic!("a multileader")
    };
    assert_eq!(
        callout.leaders[0]
            .iter()
            .map(|p| xyz(*p))
            .collect::<Vec<_>>(),
        [[100.0, 130.0, 0.0], [95.0, 140.0, 0.0]]
    );
    // The template's objects did not move.
    assert_eq!(
        xyz(insert(&objects, "ROT-002/term").position),
        [20.0, 50.0, 0.0]
    );
}

#[test]
fn a_template_object_scaled_about_a_snapped_point_reaches_every_sheet_built_from_it() {
    let set = strip_set("scale-template");
    let report = lands(
        &set,
        json!([{"op": "sc", "target": {"id": "ROT-002/term"},
            "base_point": {"snap": {"id": "ROT-002/term", "snap": "insertion", "which": "position"}}, "factor": 2.5}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.op, "scale");
    assert_eq!(op.sheets, ["ROT-001", "ROT-002"]);
    assert!(op.record.as_deref().unwrap().ends_with("strip.yaml"));
    assert_eq!(op.id.as_deref(), Some("ROT-002/term"));
    assert_eq!(op.snapped.len(), 1);
    assert!(
        op.after.as_deref().unwrap().contains("scale: 2.5"),
        "{:?}",
        op.after
    );
    let objects = recorded(&set);
    for sheet in ["ROT-001", "ROT-002"] {
        let term = insert(&objects, &format!("{sheet}/term"));
        assert_eq!((term.scale, xyz(term.position)), (2.5, [20.0, 50.0, 0.0]));
    }
    // A multileader's lengths from its style are written as its own.
    let report = lands(
        &set,
        json!([{"op": "scale", "target": {"id": "ROT-001/callout"}, "base_point": [130, 100, 0], "factor": 2}]),
    );
    let after = report.operations[0].after.clone().unwrap();
    for key in ["height: ", "arrow_size: ", "landing: ", "gap: "] {
        assert!(after.contains(key), "{after}");
    }
}

#[test]
fn a_mirror_keeps_its_source_and_makes_mirrored_copies_reading_forwards() {
    let set = strip_set("mirror");
    let report = lands(
        &set,
        json!([{"op": "MI", "targets": [
            {"id": "ROT-001/note"}, {"id": "ROT-001/lug"}, {"id": "ROT-001/bend"}
        ], "first_point": [50, 0, 0], "second_point": [50, 10, 0]}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.op, "mirror");
    let made: Vec<(&str, &str)> = op
        .copies
        .iter()
        .map(|c| (c.id.as_str(), c.source.as_str()))
        .collect();
    assert_eq!(
        made,
        [
            ("ROT-001/note-1", "ROT-001/note"),
            ("ROT-001/lug-1", "ROT-001/lug"),
            ("ROT-001/bend-1", "ROT-001/bend"),
        ]
    );
    assert!(op
        .copies
        .iter()
        .all(|c| c.offset.is_none()
            && c.mirrored_across == Some([[50.0, 0.0, 0.0], [50.0, 10.0, 0.0]])));
    assert!(op.message.starts_with("mirrors "), "{}", op.message);
    let objects = recorded(&set);
    // The sources stay as they were.
    let Object::Text(note) = &objects["ROT-001/note"] else {
        panic!("a text")
    };
    assert_eq!(
        (xyz(note.position), note.rotation),
        ([70.0, 10.0, 0.0], 15.0)
    );
    assert!(!insert(&objects, "ROT-001/lug").mirrored);
    // The text's copy reads forwards along its mirrored line.
    let Object::Text(copy) = &objects["ROT-001/note-1"] else {
        panic!("a text")
    };
    assert_eq!(
        (xyz(copy.position), copy.rotation),
        ([30.0, 10.0, 0.0], 345.0)
    );
    // The insert's copy is mirrored, its attribute reading forwards.
    let lug = insert(&objects, "ROT-001/lug-1");
    assert!(lug.mirrored);
    assert_eq!(
        (xyz(lug.position), lug.rotation),
        ([90.0, 60.0, 0.0], 330.0)
    );
    assert_eq!(lug.attributes[0].value, "7");
    let Object::Arc(bend) = &objects["ROT-001/bend-1"] else {
        panic!("an arc")
    };
    assert_eq!(xyz(bend.center), [70.0, 30.0, 0.0]);
    assert_eq!((bend.start_angle, bend.end_angle), (80.0, 170.0));
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    assert!(rows.contains("copy_of: lug"), "{rows}");
}

#[test]
fn a_mirror_that_erases_its_source_mirrors_it_in_place() {
    let set = strip_set("mirror-erase");
    let report = lands(
        &set,
        json!([{"op": "mirror", "target": {"id": "ROT-001/lug"},
            "first_point": [0, 60, 0], "second_point": [1, 60, 0], "erase_source": true}]),
    );
    let op = &report.operations[0];
    assert!(op.copies.is_empty());
    assert_eq!(op.transformed, ["ROT-001/lug"]);
    let objects = recorded(&set);
    assert!(!objects.contains_key("ROT-001/lug-1"));
    let lug = insert(&objects, "ROT-001/lug");
    // Across a line that runs across the sheet: mirrored, and turned a half
    // turn, as AutoCAD's MIRROR leaves an insert.
    assert!(lug.mirrored);
    assert_eq!(
        (xyz(lug.position), lug.rotation),
        ([10.0, 60.0, 0.0], 150.0)
    );
}

#[test]
fn a_mirrored_insert_is_built_with_a_negative_x_scale_and_reads_back() {
    let set = strip_set("mirror-dwg");
    let report = lands(
        &set,
        json!([{"op": "mirror", "target": {"id": "ROT-001/lug"},
            "first_point": [50, 0, 0], "second_point": [50, 10, 0], "erase_source": true}]),
    );
    let sheet = report.sheets.iter().find(|s| s.sheet == "ROT-001").unwrap();
    let dwg = &sheet.result.as_ref().unwrap().dwg;
    let read = read_dwg(Path::new(dwg)).unwrap();
    let inserts: Vec<&agentdrafter_core::Insert> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Insert(i) => Some(i),
            _ => None,
        })
        .collect();
    let lug = inserts
        .iter()
        .find(|i| i.attributes[0].value == "7")
        .expect("the mirrored insert reads back");
    assert!(lug.mirrored);
    assert_eq!(lug.scale, 1.0);
    assert!((lug.rotation - 330.0).abs() < 1e-9, "{}", lug.rotation);
    assert_eq!(xyz(lug.position), [90.0, 60.0, 0.0]);
    // The unmirrored one reads back unmirrored.
    assert!(inserts.iter().any(|i| !i.mirrored));
}

#[test]
fn an_insert_mirrored_twice_keeps_its_attributes_reading_forwards() {
    let set = strip_set("mirror-twice");
    // Turned to 120, then mirrored across a line up the sheet twice: 240
    // mirrored, then 120 and not mirrored, where its attribute, defined
    // across the block, would read backwards at 120 were it turned with it.
    let mirror = json!({"op": "mirror", "target": {"id": "ROT-001/lug"},
        "first_point": [50, 0, 0], "second_point": [50, 10, 0], "erase_source": true});
    lands(
        &set,
        json!([{"op": "rotate", "target": {"id": "ROT-001/lug"},
            "base_point": [10, 60, 0], "angle": 90}, mirror.clone()]),
    );
    let once = insert(&recorded(&set), "ROT-001/lug");
    assert!(once.mirrored && (once.rotation - 240.0).abs() < 1e-9, "{once:?}");
    let report = lands(&set, json!([mirror]));
    let twice = insert(&recorded(&set), "ROT-001/lug");
    assert!(!twice.mirrored && twice.readable_attributes, "{twice:?}");
    assert!((twice.rotation - 120.0).abs() < 1e-9, "{twice:?}");
    assert_eq!(xyz(twice.position), [10.0, 60.0, 0.0]);
    // Its DWG holds the attribute reading forwards, at 300, and reads back so.
    let sheet = report.sheets.iter().find(|s| s.sheet == "ROT-001").unwrap();
    let dwg = &sheet.result.as_ref().unwrap().dwg;
    let read = read_dwg(Path::new(dwg)).unwrap();
    let lug = read
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Insert(i) if i.attributes[0].value == "7" => Some(i),
            _ => None,
        })
        .expect("the insert reads back");
    assert!(!lug.mirrored && lug.readable_attributes, "{lug:?}");
    let term = read.blocks.iter().find(|b| b.name == "TERM").unwrap();
    let definition = term.attribute_definitions().next().unwrap();
    let text = agentdrafter_core::attribute_text(lug, term, definition, "7");
    assert!((text.rotation - 300.0).abs() < 1e-9, "{}", text.rotation);
}

#[test]
fn a_dry_run_writes_nothing_and_undo_puts_the_set_back_byte_for_byte() {
    let set = strip_set("dry-run-undo");
    let before = snapshot::snapshot(&set);
    let operations = json!([
        {"op": "rotate", "target": {"id": "ROT-001/term"}, "base_point": [0, 0, 0], "angle": 45},
        {"op": "scale", "target": {"id": "ROT-001/note"}, "base_point": [70, 10, 0], "factor": 0.5},
        {"op": "mirror", "target": {"id": "ROT-001/spoke"}, "first_point": [0, 0, 0], "second_point": [1, 1, 0]},
    ]);
    let dry = send(&set, operations.clone(), true).result.unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.operations[0].sheets, ["ROT-001", "ROT-002"]);
    assert!(
        dry.operations[1]
            .after
            .as_deref()
            .unwrap()
            .contains("height: 1.25"),
        "{:?}",
        dry.operations[1].after
    );
    assert_eq!(dry.operations[2].copies.len(), 1);
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
fn a_template_rotate_is_drawn_before_it_lands_as_changed_on_every_sheet() {
    let set = strip_set("preview");
    let before = snapshot::snapshot(&set);
    let mut request = DisplayListRequest::new(&set);
    request.sheet = Some("ROT-002".to_owned());
    request.change_set = Some(json!({"operations": [
        {"op": "rotate", "target": {"id": "ROT-001/term"}, "base_point": [20, 50, 0], "angle": 90}
    ]}));
    let report = display_list_with(&request, &Settings::default()).unwrap();
    let preview = report.preview.as_ref().unwrap();
    let changed: Vec<(&str, HowChanged)> = preview
        .objects
        .iter()
        .map(|o| (o.id.as_str(), o.how))
        .collect();
    assert_eq!(
        changed,
        [
            ("ROT-001/term", HowChanged::Changed),
            ("ROT-002/term", HowChanged::Changed),
        ]
    );
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn what_cannot_be_held_or_is_wrong_is_refused_and_changes_nothing() {
    let set = strip_set("refused");
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
    // A table has no rotation of its own.
    let e = failed(
        json!({"op": "rotate", "targets": [{"id": "ROT-001/spoke"}, {"id": "ROT-001/list"}], "base_point": [0, 0, 0], "angle": 30}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].targets[1]"))
    );
    assert!(
        e.message.contains("no rotation of its own"),
        "{}",
        e.message
    );
    // A table is mirrored only upright.
    let e = failed(
        json!({"op": "mirror", "target": {"id": "ROT-001/list"}, "first_point": [0, 0, 0], "second_point": [1, 2, 0]}),
    );
    assert_eq!(e.code, ErrorCode::BadValue);
    // An object on a locked layer is not turned.
    let e = failed(
        json!({"op": "rotate", "target": {"id": "ROT-001/pin"}, "base_point": [0, 0, 0], "angle": 90}),
    );
    assert_eq!(e.code, ErrorCode::Locked);
    assert!(e.message.contains("not turned"), "{}", e.message);
    // Objects of two sheets.
    let e = failed(
        json!({"op": "scale", "targets": [{"id": "ROT-001/spoke"}, {"id": "ROT-002/term"}], "base_point": [0, 0, 0], "factor": 2}),
    );
    assert!(
        e.message.contains("one scale scales objects of one sheet"),
        "{}",
        e.message
    );
    // A snapped mirror line whose points are one.
    let e = failed(json!({"op": "mirror", "target": {"id": "ROT-001/note"},
            "first_point": {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "from"}},
            "second_point": [100, 100, 5]}));
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].second_point"))
    );
    let e = refused(
        json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "factor": 0}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].factor"))
    );
    let e = refused(
        json!({"op": "mirror", "target": {"id": "ROT-001/spoke"}, "first_point": [1, 1, 0], "second_point": [1, 1, 3]}),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].second_point"));
    let e = refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "angle": 90}));
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::MissingKey, Some("operations[0].base_point"))
    );
    let e = refused(
        json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "angle": 90, "copy": true}),
    );
    assert_eq!(e.code, ErrorCode::UnknownKey);
    let e = refused(
        json!({"op": "mirror", "target": {"id": "ROT-001/spoke"}, "first_point": [0, 0, 0], "second_point": [0, 1, 0], "erase_source": "yes"}),
    );
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(snapshot::snapshot(&set), before);
}
