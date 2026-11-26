//! Rotate, scale and mirror (#67): `rotate`, AutoCAD's ROTATE, `scale`,
//! its SCALE, and `mirror`, its MIRROR, which keeps its source and makes
//! mirrored copies unless told to erase it, with text reading forwards, as
//! MIRRTEXT 0 keeps it, and an insert mirrored, its X scale negative in the
//! DWG (#242). All on a made-up set.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, operations, query_with, read_dwg, revise_with,
    undo, Answer, ChangeReport, ChangeRequest, DisplayListRequest, ErrorCode, HowChanged, Object,
    Query, QueryRequest, Reply, ReviseRequest, Selection, SetBuildRequest, Settings, UndoRequest,
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
      - dimension: {id: ord, kind: ordinate, feature: [10, 20, 0], location: [10, 40, 0], axis: x}
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
fn a_device_mirrored_with_its_tag_leader_has_its_landing_on_the_mirrored_side() {
    let set = strip_set("mirror-tag");
    // The callout's last segment runs right, so its landing does.
    let Object::MLeader(callout) = &recorded(&set)["ROT-001/callout"] else {
        panic!("a multileader")
    };
    assert_eq!(callout.side(), 1.0);
    let report = lands(
        &set,
        json!([{"op": "mirror", "targets": [{"id": "ROT-001/lug"}, {"id": "ROT-001/callout"}],
            "first_point": [100, 0, 0], "second_point": [100, 10, 0]}]),
    );
    assert_eq!(report.operations[0].copies.len(), 2);
    let objects = recorded(&set);
    let Object::MLeader(copy) = &objects["ROT-001/callout-1"] else {
        panic!("a multileader")
    };
    assert_eq!(copy.side(), -1.0, "{copy:?}");
    assert_eq!(xyz(copy.connection()), [60.0, 105.0, 0.0]);
    // One whose last segment runs straight up keeps its landing running
    // right, which the record cannot turn round, so it is refused, at its
    // target, and nothing changes.
    let before = snapshot::snapshot(&set);
    lands(
        &set,
        json!([{"op": "add_mleader", "sheet": "ROT-001", "id": "upright",
            "leaders": [[[150, 80, 0], [150, 100, 0]]], "content": {"text": "TB-2"}}]),
    );
    let after_add = snapshot::snapshot(&set);
    assert_ne!(before, after_add);
    let reply = send(
        &set,
        json!([{"op": "mirror", "target": {"id": "ROT-001/upright"},
            "first_point": [100, 0, 0], "second_point": [100, 10, 0]}]),
        false,
    );
    assert!(!reply.ok);
    let e = reply.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].target"))
    );
    assert!(e.message.contains("ROT-001/upright"), "{}", e.message);
    assert!(e.message.contains("landing"), "{}", e.message);
    assert_eq!(snapshot::snapshot(&set), after_add);
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
    assert!(
        once.mirrored && (once.rotation - 240.0).abs() < 1e-9,
        "{once:?}"
    );
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
        newest_seen: None,
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
    // An ordinate dimension is turned only by a multiple of 90 degrees.
    let e = failed(
        json!({"op": "rotate", "targets": [{"id": "ROT-001/spoke"}, {"id": "ROT-001/ord"}], "base_point": [0, 0, 0], "angle": 30}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].targets[1]"))
    );
    assert!(
        e.message.contains("multiple of 90 degrees"),
        "{}",
        e.message
    );
    // An ordinate dimension is mirrored only across 0/45/90/135 degrees.
    let e = failed(
        json!({"op": "mirror", "target": {"id": "ROT-001/ord"}, "first_point": [0, 0, 0], "second_point": [1, 2, 0]}),
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
        json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "angle": 90, "twist": true}),
    );
    assert_eq!(e.code, ErrorCode::UnknownKey);
    let e = refused(
        json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "angle": 90, "copy": "yes"}),
    );
    assert_eq!(e.code, ErrorCode::WrongType);
    let e = refused(
        json!({"op": "mirror", "target": {"id": "ROT-001/spoke"}, "first_point": [0, 0, 0], "second_point": [0, 1, 0], "erase_source": "yes"}),
    );
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn rotate_reference_and_copy_options_match_autocad() {
    let set = strip_set("rotate-ref-copy");
    let before = snapshot::snapshot(&set);

    // 1) Reference angle in degrees -> new angle in degrees (30 -> 120 = +90 exact).
    // 2) Reference angle from two snapped points -> new_point from base_point, with copy: true.
    // 3) Plain angle with copy: true keeps the source and makes a rotated copy.
    let report = lands(
        &set,
        json!([
            {
                "op": "rotate",
                "target": {"id": "ROT-001/spoke"},
                "base_point": [100, 100, 0],
                "reference_angle": 30,
                "new_angle": 120
            },
            {
                "op": "RO",
                "target": {"id": "ROT-001/spoke"},
                "base_point": [100, 100, 0],
                "reference_angle": [
                    {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "from"}},
                    {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "to"}}
                ],
                "new_point": [90, 100, 0],
                "copy": true
            },
            {
                "op": "rotate",
                "target": {"id": "ROT-001/note"},
                "base_point": [70, 10, 0],
                "reference_angle": [[0, 0, 0], [10, 0, 0]],
                "new_angle": [[0, 0, 0], [0, 10, 0]],
                "copy": true
            }
        ]),
    );

    assert_eq!(report.operations[0].transformed, ["ROT-001/spoke"]);
    assert_eq!(report.operations[1].copies.len(), 1);
    let copy1 = &report.operations[1].copies[0];
    assert_eq!(copy1.id, "ROT-001/spoke-1");
    assert_eq!(copy1.source, "ROT-001/spoke");
    assert_eq!(copy1.turned, Some(90.0));
    assert_eq!(copy1.scaled, None);
    assert_eq!(copy1.about, Some([100.0, 100.0, 0.0]));
    assert!(copy1.offset.is_none());
    assert_eq!(report.operations[1].snapped.len(), 2);

    assert_eq!(report.operations[2].copies.len(), 1);
    let copy2 = &report.operations[2].copies[0];
    assert_eq!(copy2.id, "ROT-001/note-1");
    assert_eq!(copy2.turned, Some(90.0));

    let objs = recorded(&set);
    // spoke was turned +90 (100,100)->(110,100) => (100,100)->(100,110)
    match &objs["ROT-001/spoke"] {
        Object::Line(l) => {
            assert_eq!((l.from.x, l.from.y), (100.0, 100.0));
            assert_eq!((l.to.x, l.to.y), (100.0, 110.0));
        }
        other => panic!("expected line, got {other:?}"),
    }
    // spoke-1 is a copy turned another +90 (90 -> 180) => (100,100)->(90,100)
    match &objs["ROT-001/spoke-1"] {
        Object::Line(l) => {
            assert_eq!((l.from.x, l.from.y), (100.0, 100.0));
            assert_eq!((l.to.x, l.to.y), (90.0, 100.0));
        }
        other => panic!("expected line, got {other:?}"),
    }
    // Original note stays at rotation 15; copy note-1 is at rotation 105.
    match (&objs["ROT-001/note"], &objs["ROT-001/note-1"]) {
        (Object::Text(src), Object::Text(dup)) => {
            assert!((src.rotation - 15.0).abs() < 1e-9);
            assert!((dup.rotation - 105.0).abs() < 1e-9);
        }
        other => panic!("expected texts, got {other:?}"),
    }

    // Undo puts the set back byte for byte.
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: report.journal_entry.unwrap(),
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn scale_reference_and_copy_options_match_autocad() {
    let set = strip_set("scale-ref-copy");
    let before = snapshot::snapshot(&set);

    // 1) Reference length in units -> new length in units (10 mm -> 25 mm = 2.5x).
    // 2) Reference length from snapped endpoints -> new_point from base_point with copy: true.
    // 3) Reference length from two points -> new_length from two points with copy: true on mleader.
    let report = lands(
        &set,
        json!([
            {
                "op": "scale",
                "target": {"id": "ROT-001/spoke"},
                "base_point": [100, 100, 0],
                "reference_length": "10 mm",
                "new_length": "25 mm"
            },
            {
                "op": "SC",
                "target": {"id": "ROT-001/spoke"},
                "base_point": [100, 100, 0],
                "reference_length": [
                    {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "from"}},
                    {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "to"}}
                ],
                "new_point": [150, 100, 0],
                "copy": true
            },
            {
                "op": "scale",
                "target": {"id": "ROT-001/bend"},
                "base_point": [30, 30, 0],
                "reference_length": [[0, 0, 0], [4, 0, 0]],
                "new_length": [[0, 0, 0], [0, 12, 0]],
                "copy": true
            }
        ]),
    );

    assert_eq!(report.operations[0].transformed, ["ROT-001/spoke"]);
    assert_eq!(report.operations[1].copies.len(), 1);
    let copy1 = &report.operations[1].copies[0];
    assert_eq!(copy1.id, "ROT-001/spoke-1");
    assert_eq!(copy1.source, "ROT-001/spoke");
    assert_eq!(copy1.turned, None);
    assert_eq!(copy1.scaled, Some(2.0));
    assert_eq!(copy1.about, Some([100.0, 100.0, 0.0]));
    assert!(copy1.offset.is_none());

    assert_eq!(report.operations[2].copies.len(), 1);
    let copy2 = &report.operations[2].copies[0];
    assert_eq!(copy2.id, "ROT-001/bend-1");
    assert_eq!(copy2.scaled, Some(3.0));

    let objs = recorded(&set);
    // spoke scaled 2.5x: (100,100)->(125,100); spoke-1 copied and scaled 2x: (100,100)->(150,100).
    match (&objs["ROT-001/spoke"], &objs["ROT-001/spoke-1"]) {
        (Object::Line(src), Object::Line(dup)) => {
            assert!((src.to.x - 125.0).abs() < 1e-9);
            assert!((dup.to.x - 150.0).abs() < 1e-9);
        }
        other => panic!("expected lines, got {other:?}"),
    }
    // bend stays radius 4; bend-1 copy has radius 12.
    match (&objs["ROT-001/bend"], &objs["ROT-001/bend-1"]) {
        (Object::Arc(src), Object::Arc(dup)) => {
            assert!((src.radius - 4.0).abs() < 1e-9);
            assert!((dup.radius - 12.0).abs() < 1e-9);
        }
        other => panic!("expected arcs, got {other:?}"),
    }

    // Undo puts the set back byte for byte.
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: report.journal_entry.unwrap(),
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn rotate_and_scale_reference_and_copy_validation_errors() {
    let set = strip_set("ref-copy-errors");
    let before = snapshot::snapshot(&set);
    let refused = |op: Value| {
        let reply = send(&set, json!([op]), false);
        assert!(!reply.ok, "expected refusal, got {reply:#?}");
        reply.error.unwrap()
    };
    let failed = |op: Value| {
        let report = send(&set, json!([op]), false).result.unwrap();
        assert_eq!(report.landed, 0);
        report.operations[0].error.clone().unwrap()
    };

    // Mutually exclusive plain + reference options.
    assert_eq!(
        refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "angle": 90, "reference_angle": 0, "new_angle": 90})).code,
        ErrorCode::Usage
    );
    assert_eq!(
        refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_angle": 0, "new_angle": 90, "new_point": [1, 0, 0]})).code,
        ErrorCode::Usage
    );
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "factor": 2, "reference_length": 1, "new_length": 2})).code,
        ErrorCode::Usage
    );
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_length": 1, "new_length": 2, "new_point": [2, 0, 0]})).code,
        ErrorCode::Usage
    );
    // Missing half of Reference pair.
    assert_eq!(
        refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_angle": 10})).code,
        ErrorCode::MissingKey
    );
    assert_eq!(
        refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "new_angle": 90})).code,
        ErrorCode::MissingKey
    );
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_length": 5})).code,
        ErrorCode::MissingKey
    );
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "new_length": 10})).code,
        ErrorCode::MissingKey
    );
    // Degenerate / non-positive reference values.
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_length": 0, "new_length": 10})).code,
        ErrorCode::BadValue
    );
    assert_eq!(
        refused(json!({"op": "scale", "target": {"id": "ROT-001/spoke"}, "base_point": [0, 0, 0], "reference_length": [[1, 1, 0], [1, 1, 0]], "new_length": 10})).code,
        ErrorCode::BadValue
    );
    assert_eq!(
        refused(json!({"op": "rotate", "target": {"id": "ROT-001/spoke"}, "base_point": [5, 5, 0], "reference_angle": 0, "new_point": [5, 5, 2]})).code,
        ErrorCode::BadValue
    );
    // An ordinate dimension turned off 90 degrees is still refused with copy: true.
    assert_eq!(
        failed(json!({"op": "rotate", "target": {"id": "ROT-001/ord"}, "base_point": [0, 0, 0], "angle": 30, "copy": true})).code,
        ErrorCode::BadValue
    );
    // Snapped points resolving to coincidence fail cleanly.
    assert_eq!(
        failed(json!({
            "op": "scale",
            "target": {"id": "ROT-001/spoke"},
            "base_point": [100, 100, 0],
            "reference_length": 10,
            "new_point": {"snap": {"id": "ROT-001/spoke", "snap": "endpoint", "which": "from"}}
        }))
        .code,
        ErrorCode::BadValue
    );
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn rotate_and_scale_reference_and_copy_work_on_original_dwg() {
    let set = strip_set("original-ref-copy");
    let dwg = set.join("dwg").join("ROT-001.dwg");
    let req: ReviseRequest = serde_json::from_value(json!({
        "set": dwg,
        "change_set": {
            "operations": [
                {
                    "op": "rotate",
                    "target": {"id": "ROT-001/spoke"},
                    "base_point": [100, 100, 0],
                    "reference_angle": [[0, 0, 0], [10, 0, 0]],
                    "new_angle": [[0, 0, 0], [0, 10, 0]],
                    "copy": true
                },
                {
                    "op": "scale",
                    "target": {"id": "ROT-001/spoke"},
                    "base_point": [100, 100, 0],
                    "reference_length": 10,
                    "new_length": 25,
                    "copy": true
                }
            ]
        }
    }))
    .unwrap();
    let reply = revise_with(&req, &Settings::default());
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!(report.operations.len(), 2);
    assert_eq!(report.operations[0].copies.len(), 1);
    assert_eq!(report.operations[0].copies[0].turned, Some(90.0));
    assert_eq!(report.operations[0].copies[0].scaled, None);
    assert_eq!(report.operations[1].copies.len(), 1);
    assert_eq!(report.operations[1].copies[0].scaled, Some(2.5));
    assert_eq!(report.operations[1].copies[0].turned, None);
}

#[test]
fn table_turns_scales_and_mirrors_across_slanting_line_and_round_trips_dwg() {
    let set = strip_set("table-253");
    // 1. Scale table by 2 about [10, 150, 0] (its top-left corner).
    lands(
        &set,
        json!([{
            "op": "scale",
            "target": {"id": "ROT-001/list"},
            "base_point": [10, 150, 0],
            "factor": 2
        }]),
    );
    let objects = recorded(&set);
    let Object::Table(scaled) = &objects["ROT-001/list"] else {
        panic!("expected table");
    };
    assert_eq!(xyz(scaled.position), [10.0, 150.0, 0.0]);
    assert_eq!(scaled.columns, vec![20.0]);
    assert_eq!(scaled.rows, vec![10.0]);
    // Standard metric data row text height is 4.5 mm, doubled to 9.0 mm.
    assert_eq!(scaled.cells[0][0].height, Some(9.0));

    // 2. Rotate table by 90° about [10, 150, 0], plus make a 45° copy.
    let rep = lands(
        &set,
        json!([
            {
                "op": "rotate",
                "target": {"id": "ROT-001/list"},
                "base_point": [10, 150, 0],
                "angle": 90
            },
            {
                "op": "rotate",
                "target": {"id": "ROT-001/list"},
                "base_point": [10, 150, 0],
                "angle": 45,
                "copy": true
            }
        ]),
    );
    let copy_id = rep.operations[1].copies[0].id.clone();
    let objects = recorded(&set);
    let Object::Table(turned) = &objects["ROT-001/list"] else {
        panic!("expected table");
    };
    assert_eq!(turned.rotation, 90.0);
    let Object::Table(turned_copy) = &objects[&copy_id] else {
        panic!("expected table copy");
    };
    assert_eq!(turned_copy.rotation, 135.0);

    // 3. Mirror the 90°-rotated table across a slanting line (y = 2x).
    let mirror_rep = lands(
        &set,
        json!([{
            "op": "mirror",
            "target": {"id": "ROT-001/list"},
            "first_point": [0, 0, 0],
            "second_point": [1, 2, 0]
        }]),
    );
    let mirror_id = mirror_rep.operations[0].copies[0].id.clone();
    let objects = recorded(&set);
    let Object::Table(mirrored) = &objects[&mirror_id] else {
        panic!("expected mirrored table");
    };
    assert!(
        mirrored.rotation > 0.0 && mirrored.rotation < 90.0,
        "expected forward-reading slanted rotation, got {}",
        mirrored.rotation
    );

    // 4. Verify DWG round-trip reads back rotated/scaled/mirrored tables.
    let read = read_dwg(&set.join("dwg").join("ROT-001.dwg")).unwrap();
    let tables: Vec<&agentdrafter_core::Table> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Table(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 3);
    assert!(tables.iter().any(|t| (t.rotation - 90.0).abs() < 1e-6));
    assert!(tables.iter().any(|t| (t.rotation - 135.0).abs() < 1e-6));
    assert!(tables
        .iter()
        .any(|t| (t.rotation - mirrored.rotation).abs() < 1e-6));

    // 5. Display list renders without error.
    let mut dl_req = DisplayListRequest::new(&set);
    dl_req.sheet = Some("ROT-001".to_owned());
    let _ = display_list_with(&dl_req, &Settings::default()).unwrap();
}

#[test]
fn mleader_with_block_content_mirrors_and_round_trips_dwg() {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-transform")
        .join("mleader-block-253");
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "blocks.yaml",
        "units: mm
blocks:
  FLAG:
    objects:
      - polyline: {points: [[0, -4, 0], [10, -4, 0], [10, 4, 0], [0, 4, 0]], closed: true}
      - attdef: {tag: TAG, position: [5, 0, 0], height: 2}
",
    );
    write(
        "templates/base.yaml",
        "units: mm\nblock_library: ../blocks.yaml\nobjects: []\n",
    );
    write(
        "rows.yaml",
        "rows:
  - sheet: ML-001
    template: templates/base.yaml
    objects:
      - mleader:
          id: call
          leaders: [[[10, 20, 0], [30, 30, 0]]]
          content: {block: FLAG, attributes: {TAG: 'V-1'}}
",
    );
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);

    // Mirror across vertical line x = 50 (keep source).
    let rep = lands(
        &set,
        json!([{
            "op": "mirror",
            "target": {"id": "ML-001/call"},
            "first_point": [50, 0, 0],
            "second_point": [50, 100, 0]
        }]),
    );
    let copy_id = rep.operations[0].copies[0].id.clone();
    let objects = recorded(&set);
    let Object::MLeader(mirrored) = &objects[&copy_id] else {
        panic!("expected mleader copy");
    };
    let agentdrafter_core::MLeaderContent::Block(b) = &mirrored.content else {
        panic!("expected block content");
    };
    assert!(b.mirrored);
    assert!(!b.readable_attributes);
    assert_eq!(b.rotation, 0.0);

    // Verify DWG read-back preserves negative X scale -> mirrored == true.
    let read = read_dwg(&set.join("dwg").join("ML-001.dwg")).unwrap();
    let mirrored_in_dwg = read.objects.iter().any(|o| {
        matches!(
            o,
            Object::MLeader(m)
                if matches!(&m.content, agentdrafter_core::MLeaderContent::Block(b) if b.mirrored)
        )
    });
    assert!(
        mirrored_in_dwg,
        "mirrored mleader block did not round-trip DWG"
    );

    // Mirroring the mirrored copy back clears `mirrored` and sets `readable_attributes: true`.
    let rep2 = lands(
        &set,
        json!([{
            "op": "mirror",
            "target": {"id": copy_id},
            "first_point": [50, 0, 0],
            "second_point": [50, 100, 0],
            "erase_source": true
        }]),
    );
    assert_eq!(rep2.operations[0].transformed, [copy_id.as_str()]);
    let objects = recorded(&set);
    let Object::MLeader(unmirrored) = &objects[&copy_id] else {
        panic!("expected mleader");
    };
    let agentdrafter_core::MLeaderContent::Block(b2) = &unmirrored.content else {
        panic!("expected block content");
    };
    assert!(!b2.mirrored);
    assert!(b2.readable_attributes);
}

#[test]
fn viewport_turns_scales_refuses_mirror_and_round_trips_dwg() {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-transform")
        .join("viewport-253");
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "templates/layout.yaml",
        "units: mm
paper: ISO A3 landscape
layout: {name: Sheet1}
objects:
  - line: {id: part, from: [0, 0, 0], to: [100, 0, 0]}
",
    );
    write(
        "rows.yaml",
        "rows:
  - sheet: VP-001
    template: templates/layout.yaml
    objects:
      - viewport: {id: main, space: paper, center: [100, 100, 0], width: 80, height: 40, view_center: [50, 0, 0], scale: '1:4'}
      - viewport: {id: locked, space: paper, center: [260, 100, 0], width: 60, height: 40, view_center: [50, 0, 0], scale: '1:4', locked: true}
      - viewport: {id: round, space: paper, clip: {center: [350, 200, 0], radius: 30}, view_center: [50, 0, 0], scale: '1:2'}
",
    );
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);

    // 1. Rotate rectangular unlocked viewport by 90°, make a 30° copy, and scale by 2.
    let rep = lands(
        &set,
        json!([
            {
                "op": "rotate",
                "target": {"id": "VP-001/main"},
                "base_point": [100, 100, 0],
                "angle": 90
            },
            {
                "op": "rotate",
                "target": {"id": "VP-001/main"},
                "base_point": [100, 100, 0],
                "angle": 30,
                "copy": true
            },
            {
                "op": "scale",
                "target": {"id": "VP-001/main"},
                "base_point": [100, 100, 0],
                "factor": 2
            }
        ]),
    );
    let clipped_copy_id = rep.operations[1].copies[0].id.clone();
    let objects = recorded(&set);
    let Object::Viewport(main) = &objects["VP-001/main"] else {
        panic!("expected viewport");
    };
    // Originally 80x40 -> quarter-turned to 40x80 -> doubled to 80x160; scale 0.25 -> 0.5; rotation 90°.
    assert_eq!(main.width, 80.0);
    assert_eq!(main.height, 160.0);
    assert_eq!(main.scale, 0.5);
    assert_eq!(main.rotation, 90.0);
    assert!(main.clip.is_none());

    let Object::Viewport(clipped_copy) = &objects[&clipped_copy_id] else {
        panic!("expected clipped viewport copy");
    };
    assert!(clipped_copy.clip.is_some());
    assert_eq!(clipped_copy.rotation, 120.0);

    // 2. Rotate & scale locked viewport: frame turns/scales, view scale & twist stay locked.
    lands(
        &set,
        json!([
            {
                "op": "rotate",
                "target": {"id": "VP-001/locked"},
                "base_point": [260, 100, 0],
                "angle": 90
            },
            {
                "op": "scale",
                "target": {"id": "VP-001/locked"},
                "base_point": [260, 100, 0],
                "factor": 1.5
            }
        ]),
    );
    let objects = recorded(&set);
    let Object::Viewport(locked) = &objects["VP-001/locked"] else {
        panic!("expected locked viewport");
    };
    assert_eq!(locked.width, 60.0);
    assert_eq!(locked.height, 90.0);
    assert_eq!(locked.scale, 0.25);
    assert_eq!(locked.rotation, 0.0);

    // 3. Rotate & scale circular clipped viewport.
    lands(
        &set,
        json!([
            {
                "op": "rotate",
                "target": {"id": "VP-001/round"},
                "base_point": [350, 200, 0],
                "angle": 45
            },
            {
                "op": "scale",
                "target": {"id": "VP-001/round"},
                "base_point": [350, 200, 0],
                "factor": 0.5
            }
        ]),
    );
    let objects = recorded(&set);
    let Object::Viewport(round) = &objects["VP-001/round"] else {
        panic!("expected round viewport");
    };
    assert_eq!(round.rotation, 45.0);
    assert_eq!(round.scale, 0.25);
    assert!(matches!(
        round.clip,
        Some(agentdrafter_core::Clip::Circle { radius, .. }) if (radius - 15.0).abs() < 1e-9
    ));

    // 4. Mirroring any viewport is refused.
    let reply = send(
        &set,
        json!([{
            "op": "mirror",
            "target": {"id": "VP-001/main"},
            "first_point": [0, 0, 0],
            "second_point": [0, 100, 0]
        }]),
        false,
    );
    let err = reply.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);
    assert!(err.message.contains("viewport"), "{}", err.message);

    // 5. DWG round-trip preserves viewport twist angles (`rotation`).
    let read = read_dwg(&set.join("dwg").join("VP-001.dwg")).unwrap();
    let vps: Vec<&agentdrafter_core::Viewport> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Viewport(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(vps.len(), 4);
    assert!(vps.iter().any(|v| (v.rotation - 90.0).abs() < 1e-6));
    assert!(vps.iter().any(|v| (v.rotation - 120.0).abs() < 1e-6));
    assert!(vps.iter().any(|v| (v.rotation - 45.0).abs() < 1e-6));

    // 6. Display list renders rotated/clipped viewports cleanly.
    let mut dl_req = DisplayListRequest::new(&set);
    dl_req.sheet = Some("VP-001".to_owned());
    let _ = display_list_with(&dl_req, &Settings::default()).unwrap();
}

#[test]
fn mirrored_text_mtext_and_attributes_stay_inside_their_mirrored_boxes() {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-transform")
        .join("mirror-text-251");
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    // A 30x10 cell block with a left-justified attribute near its left wall (x = 2).
    // When mirrored across x = 50, the cell moves to [70, 100] and the attribute's
    // anchor moves to x = 98; flipping justification to `right` keeps the attribute
    // inside [70, 98] instead of overflowing past x = 100!
    write(
        "blocks.yaml",
        "units: mm
blocks:
  CELL:
    objects:
      - polyline: {points: [[0, 0, 0], [30, 0, 0], [30, 10, 0], [0, 10, 0]], closed: true}
      - attdef: {tag: TAG, position: [2, 2, 0], height: 3, justify: left}
",
    );
    write(
        "templates/sheet.yaml",
        "units: mm\nblock_library: ../blocks.yaml\nobjects: []\n",
    );
    write(
        "rows.yaml",
        "rows:
  - sheet: MT-001
    template: templates/sheet.yaml
    objects:
      - text: {id: t_edge, contents: EDGE-NOTE, position: [5, 20, 0], height: 4, justify: left}
      - text: {id: t_slant, contents: SLANT, position: [10, 40, 0], height: 4, rotation: 30, justify: top_left}
      - mtext: {id: mt_box, contents: \"LINE 1\\nLINE 2\", position: [5, 60, 0], height: 3, width: 25, justify: top_left}
      - insert: {id: cell, block: CELL, position: [0, 0, 0], attributes: {TAG: 'FT-101'}}
",
    );
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);

    // 1. Mirror all four across vertical line x = 50.
    let rep_v = lands(
        &set,
        json!([{
            "op": "mirror",
            "targets": [
                {"id": "MT-001/t_edge"},
                {"id": "MT-001/t_slant"},
                {"id": "MT-001/mt_box"},
                {"id": "MT-001/cell"}
            ],
            "first_point": [50, 0, 0],
            "second_point": [50, 100, 0]
        }]),
    );
    let objects = recorded(&set);
    let Object::Text(t_edge_v) = &objects[&rep_v.operations[0].copies[0].id] else {
        panic!("expected text");
    };
    assert_eq!(xyz(t_edge_v.position), [95.0, 20.0, 0.0]);
    assert_eq!(t_edge_v.rotation, 0.0);
    assert_eq!(t_edge_v.justify, agentdrafter_core::Justify::Right);

    let Object::Text(t_slant_v) = &objects[&rep_v.operations[0].copies[1].id] else {
        panic!("expected text");
    };
    assert_eq!(xyz(t_slant_v.position), [90.0, 40.0, 0.0]);
    assert_eq!(t_slant_v.rotation, 330.0);
    assert_eq!(t_slant_v.justify, agentdrafter_core::Justify::TopRight);

    let Object::MText(mt_v) = &objects[&rep_v.operations[0].copies[2].id] else {
        panic!("expected mtext");
    };
    assert_eq!(xyz(mt_v.position), [95.0, 60.0, 0.0]);
    assert_eq!(mt_v.rotation, 0.0);
    assert_eq!(mt_v.justify, agentdrafter_core::Attachment::TopRight);

    // 2. Mirror text, mtext, and cell across horizontal line y = 50.
    let rep_h = lands(
        &set,
        json!([{
            "op": "mirror",
            "targets": [
                {"id": "MT-001/t_edge"},
                {"id": "MT-001/mt_box"},
                {"id": "MT-001/cell"}
            ],
            "first_point": [0, 50, 0],
            "second_point": [100, 50, 0]
        }]),
    );
    let objects = recorded(&set);
    let Object::Text(t_edge_h) = &objects[&rep_h.operations[0].copies[0].id] else {
        panic!("expected text");
    };
    assert_eq!(xyz(t_edge_h.position), [5.0, 80.0, 0.0]);
    assert_eq!(t_edge_h.rotation, 0.0);
    assert_eq!(t_edge_h.justify, agentdrafter_core::Justify::TopLeft);

    let Object::MText(mt_h) = &objects[&rep_h.operations[0].copies[1].id] else {
        panic!("expected mtext");
    };
    assert_eq!(xyz(mt_h.position), [5.0, 40.0, 0.0]);
    assert_eq!(mt_h.rotation, 0.0);
    assert_eq!(mt_h.justify, agentdrafter_core::Attachment::BottomLeft);

    // 3. Verify DWG round-trip reads back ALL mirrored text, mtext, and mirrored inserts!
    let read = read_dwg(&set.join("dwg").join("MT-001.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{:#?}", read.not_read);
    let cell_block = read.blocks.iter().find(|b| b.name == "CELL").unwrap();
    let att_def = cell_block.attribute_definitions().next().unwrap();
    let mirrored_inserts: Vec<&agentdrafter_core::Insert> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Insert(ins) if ins.mirrored => Some(ins),
            _ => None,
        })
        .collect();
    assert_eq!(mirrored_inserts.len(), 2);
    // Vertically mirrored insert (rotation 0) flips Left -> Right;
    // horizontally mirrored insert (rotation 180) flips Left -> TopLeft.
    let v_ins = mirrored_inserts
        .iter()
        .find(|i| (i.rotation - 0.0).abs() < 1e-6)
        .unwrap();
    let v_att = agentdrafter_core::attribute_text(v_ins, cell_block, att_def, "FT-101");
    assert_eq!(v_att.justify, agentdrafter_core::Justify::Right);
    assert_eq!(xyz(v_att.position), [98.0, 2.0, 0.0]);

    let h_ins = mirrored_inserts
        .iter()
        .find(|i| (i.rotation - 180.0).abs() < 1e-6)
        .unwrap();
    let h_att = agentdrafter_core::attribute_text(h_ins, cell_block, att_def, "FT-101");
    assert_eq!(h_att.justify, agentdrafter_core::Justify::TopLeft);
    assert_eq!(xyz(h_att.position), [2.0, 98.0, 0.0]);

    // 4. Verify rendered extents of the mirrored text box and mirrored insert
    // attribute stay inside their exact mirrored boxes!
    let mut dl_req = DisplayListRequest::new(&set);
    dl_req.sheet = Some("MT-001".to_owned());
    let dl = display_list_with(&dl_req, &Settings::default()).unwrap();
    let run_text = |r: &agentdrafter_core::TextRun| -> String {
        r.characters.iter().map(|(c, _)| c).collect()
    };
    let run_x_span = |r: &agentdrafter_core::TextRun| -> (f64, f64) {
        let [lo, hi] = r.drawn.expect("drawn box");
        let x0 = r.origin[0] + lo[0] * r.across[0];
        let x1 = r.origin[0] + hi[0] * r.across[0];
        (x0.min(x1), x0.max(x1))
    };
    let orig_run = dl
        .display_list
        .text
        .iter()
        .find(|r| {
            run_text(r) == "EDGE-NOTE"
                && (r.origin[0] - 5.0).abs() < 1e-3
                && (r.origin[1] - 20.0).abs() < 1e-3
        })
        .expect("original EDGE-NOTE run");
    let mirr_v_run = dl
        .display_list
        .text
        .iter()
        .find(|r| {
            run_text(r) == "EDGE-NOTE" && (r.origin[1] - 20.0).abs() < 1e-3 && r.origin[0] > 50.0
        })
        .expect("vertically mirrored EDGE-NOTE run");
    // Original ink box spans [orig_l, orig_r] starting at pen x = 5;
    // vertically mirrored across x = 50 (Right-justified at x = 95) has the
    // exact same glyph width and ends at x = 95 (within 1 left-side bearing
    // of 100 - orig_r, never overflowing past x = 95).
    let (orig_l, orig_r) = run_x_span(orig_run);
    let (mirr_l, mirr_r) = run_x_span(mirr_v_run);
    let left_bearing = orig_l - 5.0;
    assert!(
        (mirr_r - 95.0).abs() < 1e-6
            && ((mirr_r - mirr_l) - (orig_r - orig_l)).abs() < 1e-6
            && (mirr_l - (100.0 - orig_r + left_bearing)).abs() < 1e-6,
        "mirrored text ink [{mirr_l}, {mirr_r}] did not match mirrored [{orig_l}, {orig_r}] across x=50"
    );
    // And the vertically mirrored CELL insert's attribute `FT-101` must sit
    // strictly inside the mirrored cell [70, 100] (not overflowing past 100!).
    let mirr_att_run = dl
        .display_list
        .text
        .iter()
        .find(|r| run_text(r) == "FT-101" && r.origin[0] > 50.0 && r.origin[1] < 50.0)
        .expect("vertically mirrored FT-101 attribute run");
    let (att_l, att_r) = run_x_span(mirr_att_run);
    assert!(
        att_l >= 70.0 && att_r <= 98.01,
        "mirrored attribute ink [{att_l}, {att_r}] overflowed mirrored cell [70, 100]"
    );
}

#[test]
fn tiny_block_scale_below_1e_12_is_refused() {
    let set = strip_set("tiny_block_scale");
    // A mirrored insert written directly with scale < 1e-12 would be silently
    // clamped by acadrust to +1e-12, dropping the negative X scale sign (#250).
    let sheet_path = set.join("tiny-mirrored.yaml");
    fs::write(
        &sheet_path,
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: TERM, position: [10, 60, 0], scale: 5e-13, mirrored: true}\n",
    )
    .unwrap();
    let err = agentdrafter_core::build_with(
        &agentdrafter_core::BuildRequest {
            description: sheet_path,
            output: Some(set.join("tiny-mirrored.dwg")),
            overwrite: true,
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadValue);
    assert!(
        err.message.contains("1e-12"),
        "expected 1e-12 scale refusal, got: {}",
        err.message
    );
    // Scaling a mirrored insert below 1e-12 via `change` is also refused cleanly.
    let reply = send(
        &set,
        json!([
            {
                "op": "mirror",
                "targets": [{"id": "ROT-001/lug"}],
                "first_point": [10, 60, 0],
                "second_point": [10, 70, 0],
                "erase_source": true
            },
            {
                "op": "scale",
                "targets": [{"id": "ROT-001/lug"}],
                "base_point": [10, 60, 0],
                "factor": 5e-13
            }
        ]),
        false,
    );
    assert!(!reply.ok);
    let report = reply.result.expect("a sheet failed");
    assert_eq!(report.landed, 0);
    let err = report.sheets[0]
        .error
        .as_ref()
        .expect("sheet failure error");
    assert_eq!(err.code, ErrorCode::BadValue);
}

#[test]
fn scale_and_mirror_do_not_read_intermediate_sheet_for_plain_objects() {
    let set = strip_set("skip_intermediate_sheet");
    // Op 1 temporarily puts `note` on a layer that does not exist yet, making
    // the intermediate sheet unreadable until Op 4 puts it back on layer `0`.
    // Scaling and mirroring `spoke` (a plain line) must not read the sheet (#250).
    let report = lands(
        &set,
        json!([
            {
                "op": "change",
                "target": {"id": "ROT-001/note"},
                "set": { "layer": "not_yet_defined" }
            },
            {
                "op": "scale",
                "targets": [{"id": "ROT-001/spoke"}],
                "base_point": [100, 100, 0],
                "factor": 2
            },
            {
                "op": "mirror",
                "targets": [{"id": "ROT-001/spoke"}],
                "first_point": [100, 100, 0],
                "second_point": [100, 110, 0],
                "erase_source": true
            },
            {
                "op": "change",
                "target": {"id": "ROT-001/note"},
                "set": { "layer": "0" }
            }
        ]),
    );
    assert_eq!(report.sheets.len(), 1);
    let objects = recorded(&set);
    let Object::Line(spoke) = &objects["ROT-001/spoke"] else {
        panic!("expected line spoke");
    };
    // [100, 100] -> [110, 100], scaled by 2 -> [120, 100], mirrored across x=100 -> [80, 100].
    assert!((spoke.to.x - 80.0).abs() < 1e-6, "{spoke:?}");
    assert!((spoke.to.y - 100.0).abs() < 1e-6, "{spoke:?}");
}
