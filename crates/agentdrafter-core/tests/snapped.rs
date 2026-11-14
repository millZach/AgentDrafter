//! Snapped points in a change set: wherever an operation takes a point it
//! may take `{"snap": {...}}`, the same reference `query snap` takes, which
//! names one point. It is worked out when the operation applies, on the
//! sheet as it stands then, and the record is written with the coordinates
//! it names: a description holds points, never references. A move takes a
//! base point and a second point, as AutoCAD's MOVE does. Every DWG these
//! write was also opened and audited by AutoCAD 2026, which found each
//! written point with its own object snaps (see the pull request).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, operations, query_with, sheet_description_schema, Answer,
    ChangeReport, ChangeRequest, ErrorCode, Query, QueryRequest, Reply, SetBuildRequest, Settings,
    SnapKind, SnapRef,
};
use serde_json::{json, Value};

/// A made-up drawing set of two sheets of one template, built, in a folder
/// of its own under Cargo's scratch folder in `target/`: a strip of two
/// lugs turned 90° and twice as large, a ring and a frame on the template;
/// on the first sheet's row a lug of its own, `term`, and on each row a
/// hub, named `hub`, at a place of its own.
fn panel_set(test: &str) -> PathBuf {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-snapped")
        .join(test);
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:
  - sheet: SNAP-001
    template: templates/panel.yaml
    values: {TITLE: PANEL ONE}
    objects:
      - insert: {id: term, block: LUG, position: [120, 30, 0], rotation: 30, attributes: {NUMBER: '7'}}
      - circle: {id: hub, name: hub, center: [150, 20, 0], radius: 2}
  - sheet: SNAP-002
    template: templates/panel.yaml
    values: {TITLE: PANEL TWO}
    objects:
      - circle: {id: hub, name: hub, center: [160, 30, 0], radius: 2}
",
    );
    write(
        "blocks.yaml",
        "units: mm
layers:
  symbols: {color: yellow}
blocks:
  LUG:
    objects:
      - circle: {center: [0, 0, 0], radius: 1.5, layer: symbols}
      - line: {from: [1.5, 0, 0], to: [6, 0, 0], layer: symbols}
      - attdef: {tag: NUMBER, position: [0, 3, 0], height: 1.5, justify: middle_center}
  TAG:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
      - attdef: {tag: SNAP, position: [0, 2, 0], height: 1}
  STRIP:
    objects:
      - insert: {block: LUG, position: [0, 0, 0], attributes: {NUMBER: '1'}}
      - insert: {block: LUG, position: [0, 8, 0], attributes: {NUMBER: '2'}}
      - polyline: {points: [[-3, -3, 0], [9, -3, 0], [9, 11, 0], [-3, 11, 0]], closed: true, layer: symbols}
",
    );
    write(
        "templates/panel.yaml",
        "units: mm
block_library: ../blocks.yaml
layers:
  symbols: {color: yellow}
  wires: {color: green}
objects:
  - insert: {id: strip, block: STRIP, position: [50, 40, 0], rotation: 90, scale: 2}
  - circle: {id: ring, center: [90, 60, 0], radius: 8, layer: symbols}
  - polyline: {id: frame, points: [[0, 0, 0], [200, 0, 0], [200, 120, 0], [0, 120, 0]], closed: true}
  - text: {id: title, contents: {row: TITLE}, position: [10, 110, 0], height: 4}
",
    );
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

fn apply(set: &Path, operations: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: json!({ "operations": operations }),
            dry_run: false,
        },
        &Settings::default(),
    )
}

/// The one point `snap` names on `set`, as `query snap` gives it.
fn snapped(set: &Path, snap: SnapRef) -> [f64; 3] {
    let report = query_with(
        &QueryRequest {
            source: set.to_owned(),
            sheet: None,
            query: Query::Snap(snap),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Snap { points, .. } = report.answer else {
        panic!("not a snap")
    };
    assert_eq!(points.len(), 1, "{points:#?}");
    [points[0].at.x, points[0].at.y, points[0].at.z]
}

fn snap_of(id: &str, kind: SnapKind, which: Option<&str>) -> SnapRef {
    SnapRef {
        id: Some(id.to_owned()),
        snap: Some(kind),
        which: which.map(str::to_owned),
        ..SnapRef::default()
    }
}

fn close(got: [f64; 3], want: [f64; 3]) {
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "{got:?} is not {want:?}");
    }
}

fn rows(set: &Path) -> String {
    fs::read_to_string(set.join("rows.yaml")).unwrap()
}

/// The error a change set that failed on its sheets gives for `operation`.
fn op_error(reply: &Reply<ChangeReport>, operation: usize) -> agentdrafter_core::Error {
    let result = reply.result.as_ref().expect("a reply with a result");
    result.operations[operation]
        .error
        .clone()
        .unwrap_or_else(|| panic!("operation {operation} did not fail: {result:#?}"))
}

#[test]
fn a_snapped_point_is_written_as_the_coordinates_it_names() {
    let set = panel_set("written");
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "sheet": "SNAP-001", "id": "wire", "layer": "wires",
            "from": {"snap": {"id": "SNAP-001/strip", "snap": "endpoint", "which": "objects[0].objects[1].to"}},
            "to": {"snap": {"id": "snap-001/RING", "snap": "quadrant", "which": "90"}},
        }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let result = reply.result.unwrap();
    let op = &result.operations[0];
    // The record holds the points, in the declared unit, never the snap.
    assert!(
        rows(&set).contains("- line: {from: [50, 52, 0], to: [90, 68, 0], layer: wires, id: wire}"),
        "{}",
        rows(&set)
    );
    assert_eq!(
        op.message,
        "adds a line, `SNAP-001/wire`, to sheet `SNAP-001`, in its row; `from` is the endpoint \
         `objects[0].objects[1].to` of `SNAP-001/strip`, [50, 52, 0]; `to` is the quadrant `90` \
         of `SNAP-001/ring`, [90, 68, 0]"
    );
    // The reply says what each resolved to: what query snap gives.
    assert_eq!(op.snapped.len(), 2);
    assert_eq!(op.snapped[0].field, "from");
    assert_eq!(op.snapped[0].id, "SNAP-001/strip");
    assert_eq!(op.snapped[0].snap, SnapKind::Endpoint);
    assert_eq!(
        op.snapped[0].which.as_deref(),
        Some("objects[0].objects[1].to")
    );
    let stub = snapped(
        &set,
        snap_of(
            "SNAP-001/strip",
            SnapKind::Endpoint,
            Some("objects[0].objects[1].to"),
        ),
    );
    let at = op.snapped[0].at;
    assert_eq!([at.x, at.y, at.z], stub);
    assert_eq!(op.snapped[1].field, "to");
    let at = op.snapped[1].at;
    close([at.x, at.y, at.z], [90.0, 68.0, 0.0]);
}

#[test]
fn a_move_takes_a_base_point_and_a_second_point() {
    let set = panel_set("move");
    // Move the lug so its stub's end lands on the ring's quadrant.
    let reply = apply(
        &set,
        json!([{
            "op": "MOVE", "target": {"id": "SNAP-001/term"},
            "from": {"snap": {"id": "SNAP-001/term", "snap": "endpoint", "which": "objects[1].to"}},
            "to": {"snap": {"id": "SNAP-001/ring", "snap": "quadrant", "which": "0"}},
        }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let op = &reply.result.unwrap().operations[0];
    assert!(
        op.message.starts_with("moves `SNAP-001/term` by ["),
        "{}",
        op.message
    );
    assert_eq!(op.snapped.len(), 2);
    let end = snapped(
        &set,
        snap_of("SNAP-001/term", SnapKind::Endpoint, Some("objects[1].to")),
    );
    close(end, [98.0, 60.0, 0.0]);
    // A point may be given as coordinates on either side.
    let reply = apply(
        &set,
        json!([{
            "op": "move", "target": {"id": "SNAP-001/term"},
            "from": {"snap": {"id": "SNAP-001/term", "snap": "insertion", "which": "position"}},
            "to": [100, 50, 0],
        }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let insertion = snapped(
        &set,
        snap_of("SNAP-001/term", SnapKind::Insertion, Some("position")),
    );
    close(insertion, [100.0, 50.0, 0.0]);
}

#[test]
fn a_move_takes_by_or_from_and_to() {
    let set = panel_set("move-keys");
    let target = json!({"id": "SNAP-001/term"});
    for (op, code, field) in [
        (
            json!({"op": "move", "target": target, "by": [1, 0, 0], "from": [0, 0, 0], "to": [1, 0, 0]}),
            ErrorCode::Usage,
            "operations[0]",
        ),
        (
            json!({"op": "move", "target": target, "from": [0, 0, 0]}),
            ErrorCode::MissingKey,
            "operations[0].to",
        ),
        (
            json!({"op": "move", "target": target}),
            ErrorCode::MissingKey,
            "operations[0].by",
        ),
        (
            json!({"op": "move", "target": target, "from": "here", "to": [1, 0, 0]}),
            ErrorCode::WrongType,
            "operations[0].from",
        ),
    ] {
        let reply = apply(&set, json!([op]));
        let error = reply.error.expect("refused whole");
        assert!(reply.result.is_none(), "refused before anything is applied");
        assert_eq!(
            (error.code, error.field.as_deref()),
            (code, Some(field)),
            "{op}"
        );
    }
}

#[test]
fn a_snapped_point_sees_the_operations_before_it() {
    let set = panel_set("earlier");
    let reply = apply(
        &set,
        json!([
            {"op": "add_circle", "sheet": "SNAP-001", "id": "pin", "center": [30, 90, 0], "radius": 3},
            {"op": "move", "target": {"id": "SNAP-001/pin"}, "by": [5, 0, 0]},
            {"op": "add_line", "sheet": "SNAP-001",
             "from": {"snap": {"id": "SNAP-001/pin", "snap": "center"}},
             "to": {"snap": {"id": "SNAP-001/pin", "snap": "quadrant", "which": "270"}}},
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    assert!(
        rows(&set).contains("- line: {from: [35, 90, 0], to: [35, 87, 0], id: line-1}"),
        "{}",
        rows(&set)
    );
}

#[test]
fn a_multileaders_arrow_and_a_new_leader_line_take_snapped_points() {
    let set = panel_set("leaders");
    let reply = apply(
        &set,
        json!([
            {"op": "add_mleader", "sheet": "SNAP-001", "id": "callout",
             "leaders": [[{"snap": {"id": "SNAP-001/ring", "snap": "center"}}, [140, 90, 0]]],
             "content": {"text": "RING"}},
            {"op": "add_leader_line", "target": {"id": "SNAP-001/callout"},
             "points": [{"snap": {"id": "SNAP-001/frame", "snap": "endpoint", "which": "points[2]"}}]},
            {"op": "add_leader", "sheet": "SNAP-002",
             "points": [{"snap": {"name": "hub", "snap": "quadrant", "which": "90"}}, [180, 60, 0]]},
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let rows = rows(&set);
    assert!(
        rows.contains("leaders: [[[90, 60, 0], [140, 90, 0]], [[200, 120, 0], [140, 90, 0]]]"),
        "{rows}"
    );
    assert!(
        rows.contains("points: [[160, 32, 0], [180, 60, 0]]"),
        "{rows}"
    );
    let result = reply.result.unwrap();
    assert_eq!(result.operations[1].snapped[0].field, "points[0]");
    assert_eq!(result.operations[2].snapped[0].field, "points[0]");
}

#[test]
fn a_template_edit_snaps_on_every_sheet_it_reaches_and_refuses_a_point_that_differs() {
    let set = panel_set("template");
    // The ring is the template's, so it is at the same point on both sheets;
    // an ID names the object of its place on every sheet the edit reaches.
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "template": "templates/panel.yaml", "id": "tie",
            "from": {"snap": {"id": "SNAP-002/ring", "snap": "center"}},
            "to": {"snap": {"id": "SNAP-001/frame", "snap": "perpendicular", "which": "0",
                            "from": {"id": "SNAP-001/ring", "snap": "center"}}},
        }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let template = fs::read_to_string(set.join("templates/panel.yaml")).unwrap();
    assert!(
        template.contains("- line: {from: [90, 60, 0], to: [90, 0, 0], id: tie}"),
        "{template}"
    );
    let before = (rows(&set), template);

    // Each row's hub is at a place of its own: a template's object cannot
    // be drawn through both.
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "template": "templates/panel.yaml",
            "from": [0, 0, 0], "to": {"snap": {"name": "hub", "snap": "center"}},
        }]),
    );
    assert!(!reply.ok);
    let error = op_error(&reply, 0);
    assert_eq!(error.code, ErrorCode::BadValue);
    assert_eq!(error.field.as_deref(), Some("operations[0].to"));
    assert!(
        error.message.contains(
            "the snapped point is [150, 20, 0] on sheet `SNAP-001` and [160, 30, 0] on sheet \
             `SNAP-002`"
        ),
        "{}",
        error.message
    );
    // The lug is the first sheet's own: it is not on the second.
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "template": "templates/panel.yaml", "from": [0, 0, 0],
            "to": {"snap": {"id": "SNAP-001/term", "snap": "insertion", "which": "position"}},
        }]),
    );
    let error = op_error(&reply, 0);
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert_eq!(error.field.as_deref(), Some("operations[0].to.snap.id"));
    assert!(
        error
            .message
            .contains("no object on sheet `SNAP-002` has the ID `SNAP-002/term`"),
        "{}",
        error.message
    );
    // Nothing landed: all or nothing, byte for byte.
    let template = fs::read_to_string(set.join("templates/panel.yaml")).unwrap();
    assert_eq!((rows(&set), template), before);
}

#[test]
fn a_snapped_point_that_names_no_point_or_several_is_refused_at_its_key() {
    let set = panel_set("refused");
    let line =
        |to: Value| json!([{"op": "add_line", "sheet": "SNAP-001", "from": [0, 0, 0], "to": to}]);
    // Several, listed with where each is.
    let reply = apply(
        &set,
        line(
            json!({"snap": {"id": "SNAP-001/frame", "snap": "perpendicular",
                             "from": {"id": "SNAP-001/ring", "snap": "center"}}}),
        ),
    );
    let error = op_error(&reply, 0);
    assert_eq!(error.code, ErrorCode::BadValue);
    assert_eq!(error.field.as_deref(), Some("operations[0].to.snap.which"));
    assert!(
        error.message.contains(
            "the polyline `SNAP-001/frame` has 4 perpendiculars; a snapped point is one: name it \
             by `which`, one of `0` at [90, 0, 0], `1` at"
        ),
        "{}",
        error.message
    );
    // None.
    let reply = apply(
        &set,
        line(json!({"snap": {"id": "SNAP-001/ring", "snap": "endpoint"}})),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].to.snap"))
    );
    assert!(
        error.message.contains("a circle has no endpoint snap"),
        "{}",
        error.message
    );
    // An object the sheet does not hold, listing its IDs; a which it has not.
    let reply = apply(
        &set,
        line(json!({"snap": {"id": "SNAP-001/nope", "snap": "center"}})),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::UnknownName, Some("operations[0].to.snap.id"))
    );
    assert!(
        error
            .message
            .contains("its IDs: SNAP-001/strip, SNAP-001/ring"),
        "{}",
        error.message
    );
    let reply = apply(
        &set,
        line(json!({"snap": {"id": "SNAP-001/ring", "snap": "quadrant", "which": "45"}})),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::UnknownName, Some("operations[0].to.snap.which"))
    );
    // An object on a sheet the operation does not reach.
    let reply = apply(
        &set,
        line(json!({"snap": {"id": "SNAP-002/ring", "snap": "center"}})),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (ErrorCode::BadValue, Some("operations[0].to.snap.id"))
    );
    // A snap that is wrong in itself refuses the change set whole: no snap
    // named, a key a snap does not take, a snapped point where a direction
    // is taken.
    for (op, code, field) in [
        (
            line(json!({"snap": {"id": "SNAP-001/ring"}})),
            ErrorCode::BadValue,
            "operations[0].to.snap.snap",
        ),
        (
            line(json!({"snap": {"id": "SNAP-001/ring", "snap": "center", "near": 1}})),
            ErrorCode::WrongType,
            "operations[0].to.snap",
        ),
        (
            line(json!({"snap": {"id": "SNAP-001/ring", "snap": "center"}, "at": 1})),
            ErrorCode::UnknownKey,
            "operations[0].to.at",
        ),
        (
            line(json!({"snap": {"id": "SNAP-001/ring", "snap": "intersection"}})),
            ErrorCode::BadValue,
            "operations[0].to.snap.with",
        ),
        (
            json!([{"op": "add_ellipse", "sheet": "SNAP-001", "center": [0, 0, 0], "ratio": 0.5,
                    "major_axis": {"snap": {"id": "SNAP-001/ring", "snap": "center"}}}]),
            ErrorCode::BadValue,
            "operations[0].major_axis",
        ),
    ] {
        let reply = apply(&set, op.clone());
        assert!(reply.result.is_none(), "{op}: refused whole");
        let error = reply.error.unwrap();
        assert_eq!(
            (error.code, error.field.as_deref()),
            (code, Some(field)),
            "{op}: {error:#?}"
        );
    }
}

#[test]
fn the_operations_schemas_take_a_snapped_point_wherever_a_point_is_and_a_description_does_not() {
    let listed = operations();
    let schema = |name: &str| {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        jsonschema::validator_for(&info.schema).unwrap()
    };
    let snap = json!({"snap": {"id": "S/ring", "snap": "quadrant", "which": "90"}});
    let from = json!({"snap": {"name": "frame", "snap": "nearest",
                               "from": {"id": "S/ring", "snap": "center"}}});
    for (name, op) in [
        (
            "add_line",
            json!({"op": "add_line", "sheet": "S", "from": snap, "to": from}),
        ),
        (
            "add_polyline",
            json!({"op": "add_polyline", "sheet": "S", "points": [snap, {"point": snap, "bulge": 0.5}, [0, 0, 0]]}),
        ),
        (
            "add_circle",
            json!({"op": "add_circle", "sheet": "S", "center": snap, "radius": 1}),
        ),
        (
            "add_ellipse",
            json!({"op": "add_ellipse", "sheet": "S", "center": snap, "major_axis": [1, 0, 0], "ratio": 0.5}),
        ),
        (
            "add_hatch",
            json!({"op": "add_hatch", "sheet": "S", "pattern": "SOLID", "boundary": [{"edges": [{"line": {"from": snap, "to": [1, 0, 0]}}, {"line": {"from": [1, 0, 0], "to": snap}}]}]}),
        ),
        (
            "add_text",
            json!({"op": "add_text", "sheet": "S", "contents": "A", "position": snap, "height": 1}),
        ),
        (
            "insert_block",
            json!({"op": "insert_block", "template": "t.yaml", "block": "LUG", "position": snap}),
        ),
        (
            "add_mleader",
            json!({"op": "add_mleader", "sheet": "S", "leaders": [[snap, [1, 1, 0]]], "content": {"text": "A"}}),
        ),
        (
            "add_leader",
            json!({"op": "add_leader", "sheet": "S", "points": [snap, [1, 1, 0]]}),
        ),
        (
            "add_leader_line",
            json!({"op": "add_leader_line", "target": {"id": "S/m"}, "points": [snap]}),
        ),
        (
            "move",
            json!({"op": "move", "target": {"id": "S/m"}, "from": snap, "to": [1, 0, 0]}),
        ),
    ] {
        assert!(schema(name).is_valid(&op), "{name}: {op}");
    }
    for (name, op) in [
        (
            "add_ellipse",
            json!({"op": "add_ellipse", "sheet": "S", "center": [0, 0, 0], "major_axis": snap, "ratio": 0.5}),
        ),
        (
            "add_line",
            json!({"op": "add_line", "sheet": "S", "from": {"snap": {"id": "S/ring"}}, "to": [1, 0, 0]}),
        ),
        (
            "add_line",
            json!({"op": "add_line", "sheet": "S", "from": {"snap": {"id": "S/ring", "snap": "center"}, "at": 1}, "to": [1, 0, 0]}),
        ),
        (
            "move",
            json!({"op": "move", "target": {"id": "S/m"}, "by": [1, 0, 0], "to": [1, 0, 0]}),
        ),
        (
            "move",
            json!({"op": "move", "target": {"id": "S/m"}, "from": snap}),
        ),
        (
            "move",
            json!({"op": "move", "target": {"id": "S/m"}, "by": snap}),
        ),
    ] {
        assert!(!schema(name).is_valid(&op), "{name} takes {op}");
    }
    // A sheet description holds coordinates, never a snap.
    let description = jsonschema::validator_for(&sheet_description_schema()).unwrap();
    assert!(description.is_valid(
        &json!({"units": "mm", "objects": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}]})
    ));
    assert!(!description
        .is_valid(&json!({"units": "mm", "objects": [{"line": {"from": snap, "to": [1, 0, 0]}}]})));
}

#[test]
fn a_mapping_that_is_not_a_point_is_never_taken_for_a_snapped_point() {
    let set = panel_set("not-a-point");
    // An attribute tagged SNAP, written as an agent may write a tag.
    let reply = apply(
        &set,
        json!([{
            "op": "insert_block", "sheet": "SNAP-001", "id": "tag", "block": "TAG",
            "position": [10, 10, 0], "attributes": {"snap": "7"},
        }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    assert!(
        rows(&set).contains(
            "insert: {block: TAG, position: [10, 10, 0], attributes: {snap: 7}, id: tag}"
        ),
        "{}",
        rows(&set)
    );
    assert!(reply.result.unwrap().operations[0].snapped.is_empty());
}

#[test]
fn an_object_a_snapped_points_from_names_that_is_not_there_is_named_at_its_place() {
    let set = panel_set("nested-missing");
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "sheet": "SNAP-001", "from": [0, 0, 0],
            "to": {"snap": {"id": "SNAP-001/ring", "snap": "nearest",
                            "from": {"id": "SNAP-001/frame", "snap": "intersection",
                                     "with": {"id": "SNAP-001/missing"}}}},
        }]),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        (error.code, error.field.as_deref()),
        (
            ErrorCode::UnknownName,
            Some("operations[0].to.snap.from.with.id")
        )
    );
    assert!(
        error
            .message
            .contains("has the ID `SNAP-001/missing`, the object to intersect with"),
        "{}",
        error.message
    );
    // The object to snap from, missing, is named as that.
    let reply = apply(
        &set,
        json!([{
            "op": "add_line", "sheet": "SNAP-001", "from": [0, 0, 0],
            "to": {"snap": {"id": "SNAP-001/ring", "snap": "nearest",
                            "from": {"name": "nowhere", "snap": "center"}}},
        }]),
    );
    let error = op_error(&reply, 0);
    assert_eq!(
        error.field.as_deref(),
        Some("operations[0].to.snap.from.name")
    );
    assert!(
        error
            .message
            .contains("has the name `nowhere`, the object to snap from"),
        "{}",
        error.message
    );
}

#[test]
fn a_snaps_schema_takes_what_its_reader_takes() {
    let listed = operations();
    let info = listed
        .operations
        .iter()
        .find(|o| o.name == "add_line")
        .unwrap();
    let validator = jsonschema::validator_for(&info.schema).unwrap();
    let line = |snap: Value| json!({"op": "add_line", "sheet": "S", "from": [0, 0, 0], "to": {"snap": snap}});
    let center = json!({"id": "S/ring", "snap": "center"});
    for (snap, taken) in [
        (center.clone(), true),
        (json!({"name": "ring", "snap": "center"}), true),
        (json!({"snap": "center"}), false),
        (
            json!({"id": "S/ring", "name": "ring", "snap": "center"}),
            false,
        ),
        (
            json!({"id": "S/a", "snap": "intersection", "with": {"id": "S/b"}}),
            true,
        ),
        (json!({"id": "S/a", "snap": "intersection"}), false),
        (
            json!({"id": "S/a", "snap": "intersection", "with": {}}),
            false,
        ),
        (
            json!({"id": "S/a", "snap": "endpoint", "with": {"id": "S/b"}}),
            false,
        ),
        (
            json!({"id": "S/a", "snap": "nearest", "from": center}),
            true,
        ),
        (json!({"id": "S/a", "snap": "nearest"}), false),
        (
            json!({"id": "S/a", "snap": "center", "from": center}),
            false,
        ),
        (
            json!({"id": "S/a", "snap": "tangent", "from": {"id": "S/b", "snap": "endpoint", "with": {"id": "S/c"}}}),
            false,
        ),
    ] {
        let op = line(snap.clone());
        assert_eq!(validator.is_valid(&op), taken, "the schema on {snap}");
        // The reader agrees: a snap it refuses is refused whole.
        let set = panel_set("snap-schema");
        let mut sent = op.clone();
        sent["sheet"] = json!("SNAP-001");
        let reply = apply(&set, json!([sent]));
        assert_eq!(
            reply.result.is_some(),
            taken,
            "the reader on {snap}: {:#?}",
            reply.error
        );
    }
}
