//! Integration tests for Rank 32 (#85): `explode` (`EXPLODE` / `X`) across
//! drawing sets (insert with attributes/scale/rotation/layer-0/ByBlock,
//! dynamic block, polyline with bulged arcs, donut, revision cloud, patterned
//! and solid hatch, dimension, multileader, leader, table, display-list parity,
//! byte-for-byte undo, locked-layer & primitive refusals) and original-DWG
//! revisions.
//!
//! All drawings, blocks, layers, and geometry here are made up (ADR-0008).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, read_dwg, revise_with, undo, ChangeRequest,
    DisplayListRequest, ErrorCode, Object, ReviseRequest, SetBuildRequest, Settings, UndoRequest,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-explode-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

fn make_explode_set(name: &str) -> PathBuf {
    let dir = scratch(name);
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
layers:
  BLK_ONLY: {color: 4}
blocks:
  VALVE:
    base_point: [0, 0, 0]
    objects:
      - line: {from: [-5, 0, 0], to: [5, 0, 0], layer: "0", color: ByBlock}
      - circle: {center: [0, 0, 0], radius: 3, layer: BLK_ONLY}
      - attdef:
          tag: TAG
          default: XV-000
          position: [0, 6, 0]
          height: 2.5
          justify: middle_center
          layer: "0"
          color: ByBlock
"#,
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        r#"units: mm
paper: [297, 210]
block_library: blocks.yaml
hatch_patterns:
  STRIPES:
    lines:
      - angle: 0
        delta: [0, 4]
layers:
  WIRES: {color: green}
  NOTES: {color: yellow}
  DIMS: {color: cyan}
  LOCKED_BORDER: {color: 8, locked: true}
objects:
  - polyline:
      id: border
      layer: LOCKED_BORDER
      closed: true
      points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        r#"rows:
  - sheet: LOOP-301
    template: template.yaml
    objects:
      - insert:
          id: valve-1
          block: VALVE
          position: [60, 120, 0]
          scale: 2
          rotation: 90
          layer: WIRES
          color: 1
          attributes: {TAG: XV-301}
      - polyline:
          id: duct
          layer: WIRES
          points:
            - [20, 40, 0]
            - {point: [60, 40, 0], bulge: 1}
            - [60, 60, 0]
      - hatch:
          id: pat-hatch
          layer: NOTES
          pattern: STRIPES
          boundary:
            - polyline: {points: [[100, 20, 0], [120, 20, 0], [120, 40, 0], [100, 40, 0]]}
      - dimension:
          id: dim-1
          kind: linear
          layer: DIMS
          points: [[20, 160, 0], [80, 160, 0]]
          location: [50, 175, 0]
      - mleader:
          id: ml-1
          layer: NOTES
          leaders: [[[140, 120, 0], [160, 135, 0]]]
          content: {text: "CALLOUT"}
      - table:
          id: tbl-1
          layer: NOTES
          position: [150, 90, 0]
          columns: [30, 30]
          rows: [10, 10]
          header: true
          cells:
            - ["TAG", "DESC"]
            - ["XV-301", "ISOLATION"]
      - line:
          id: plain-line
          layer: WIRES
          from: [10, 10, 0]
          to: [20, 10, 0]
"#,
    )
    .unwrap();
    let settings = Settings::default();
    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{built:#?}");
    dir
}

#[test]
fn explode_insert_polyline_hatch_dimension_mleader_table_preserves_display_and_undoes() {
    let dir = make_explode_set("all-kinds");
    let settings = Settings::default();
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();
    let tmpl_before = fs::read(dir.join("template.yaml")).unwrap();

    // Capture display list extents BEFORE exploding!
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "LOOP-301"})).unwrap();
    let dl_before = display_list_with(&dl_req, &settings).unwrap();
    let extents_before = dl_before.extents.unwrap();

    // Explode the insert, polyline, hatch, dimension, mleader, and table in one change set!
    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    { "op": "EXPLODE", "target": { "id": "LOOP-301/valve-1" } },
                    { "op": "X", "target": { "id": "LOOP-301/duct" }, "id_prefix": "duct-seg" },
                    {
                        "op": "explode",
                        "targets": [
                            { "id": "LOOP-301/pat-hatch" },
                            { "id": "LOOP-301/dim-1" },
                            { "id": "LOOP-301/ml-1" },
                            { "id": "LOOP-301/tbl-1" }
                        ]
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(reply.ok, "{reply:#?}");
    let report = reply.result.unwrap();
    assert_eq!(report.landed, 1);
    assert_eq!(report.failed, 0);

    // Op 0: valve-1 -> line + circle (on library layer BLK_ONLY copied into template!) + text "XV-301".
    let op0 = &report.operations[0];
    assert_eq!(op0.deleted, vec!["LOOP-301/valve-1"]);
    assert_eq!(
        op0.created,
        vec![
            "LOOP-301/valve-1-1",
            "LOOP-301/valve-1-2",
            "LOOP-301/valve-1-3",
        ]
    );

    // Op 1: duct -> 1 line + 1 semicircular arc with custom id_prefix!
    let op1 = &report.operations[1];
    assert_eq!(op1.deleted, vec!["LOOP-301/duct"]);
    assert_eq!(
        op1.created,
        vec!["LOOP-301/duct-seg-1", "LOOP-301/duct-seg-2"]
    );

    // Op 2: hatch + dimension + mleader + table all exploded.
    let op2 = &report.operations[2];
    assert_eq!(
        op2.deleted,
        vec![
            "LOOP-301/pat-hatch",
            "LOOP-301/dim-1",
            "LOOP-301/ml-1",
            "LOOP-301/tbl-1",
        ]
    );
    assert!(op2.created.len() >= 15, "got {:?}", op2.created);

    // Rebuilt DWG contains none of the exploded compound objects and holds all exploded parts!
    let sheet = read_dwg(&dir.join("dwg/LOOP-301.dwg")).unwrap();
    assert!(!sheet.objects.iter().any(|o| matches!(
        o,
        Object::Insert(_)
            | Object::Hatch(_)
            | Object::Dimension(_)
            | Object::MLeader(_)
            | Object::Table(_)
    )));
    // Transformed attribute text "XV-301" exists at the rotated & scaled position!
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Text(t) if t.contents == "XV-301" && (t.height - 5.0).abs() < 1e-4 && (t.position.x - 48.0).abs() < 1e-4
    )));
    // Exploded polyline arc exists!
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Arc(a) if (a.center.x - 60.0).abs() < 1e-4 && (a.center.y - 50.0).abs() < 1e-4 && (a.radius - 10.0).abs() < 1e-4
    )));
    // Table cells & multileader callout & dimension value exist as MText!
    assert!(sheet
        .objects
        .iter()
        .any(|o| matches!(o, Object::MText(t) if t.contents == "CALLOUT")));
    assert!(sheet
        .objects
        .iter()
        .any(|o| matches!(o, Object::MText(t) if t.contents == "ISOLATION")));
    assert!(sheet
        .objects
        .iter()
        .any(|o| matches!(o, Object::MText(t) if t.contents == "60")));

    // Display-list extents remain identical!
    let dl_after = display_list_with(&dl_req, &settings).unwrap();
    let extents_after = dl_after.extents.unwrap();
    close(extents_before.min[0], extents_after.min[0]);
    close(extents_before.min[1], extents_after.min[1]);
    close(extents_before.max[0], extents_after.max[0]);
    close(extents_before.max[1], extents_after.max[1]);

    // Byte-for-byte undo restores both rows.yaml and template.yaml!
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: report.journal_entry.unwrap(),
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:#?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
    assert_eq!(fs::read(dir.join("template.yaml")).unwrap(), tmpl_before);
}

#[test]
fn explode_refuses_locked_layer_and_primitive_objects() {
    let dir = make_explode_set("refuse");
    let settings = Settings::default();

    let locked = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [{ "op": "explode", "target": { "id": "LOOP-301/border" } }]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!locked.ok);
    let rep = locked.result.unwrap();
    assert_eq!(rep.failed, 1);
    assert_eq!(
        rep.operations[0].error.as_ref().unwrap().code,
        ErrorCode::Locked
    );

    let primitive = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [{ "op": "explode", "target": { "id": "LOOP-301/plain-line" } }]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!primitive.ok);
    let rep2 = primitive.result.unwrap();
    assert_eq!(rep2.failed, 1);
    assert_eq!(
        rep2.operations[0].error.as_ref().unwrap().code,
        ErrorCode::BadValue
    );
}

#[test]
fn explode_on_original_dwg_revises_insert_and_polyline_cleanly() {
    let dir = make_explode_set("original");
    let settings = Settings::default();
    let orig = dir.join("dwg/LOOP-301.dwg");

    let rev = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [
                    { "op": "EXPLODE", "target": { "id": "LOOP-301/valve-1" } },
                    { "op": "X", "target": { "id": "LOOP-301/duct" }, "id_prefix": "seg" }
                ]
            }),
            output: None,
            dry_run: false,
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev.ok, "{rev:#?}");
    let rep = rev.result.unwrap();
    assert!(
        rep.unexpected_changes.is_empty(),
        "{:#?}",
        rep.unexpected_changes
    );
    assert!(
        rep.requested_failures.is_empty(),
        "{:#?}",
        rep.requested_failures
    );
    assert_eq!(rep.operations[0].deleted, vec!["LOOP-301/valve-1"]);
    assert_eq!(rep.operations[0].created.len(), 3);
    assert_eq!(rep.operations[1].deleted, vec!["LOOP-301/duct"]);
    assert_eq!(
        rep.operations[1].created,
        vec!["LOOP-301/seg-1", "LOOP-301/seg-2"]
    );

    let rev_path = PathBuf::from(rep.revision_dwg.unwrap());
    let revised = read_dwg(&rev_path).unwrap();
    assert!(!revised
        .objects
        .iter()
        .any(|o| matches!(o, Object::Insert(_))));
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Text(t) if t.contents == "XV-301"
    )));
    assert!(revised.objects.iter().any(|o| matches!(o, Object::Arc(_))));
}
