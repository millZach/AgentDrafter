//! Integration tests for Rank 31 (#84): `stretch` (`STRETCH` / `S`) across
//! drawing sets (box from coordinates and from a named object's snapped points,
//! stretching connected wires + enclosure polyline while moving devices inside
//! the box whole, listing every touched object in `transformed`, skipping
//! locked layers, display-list rendering, byte-for-byte undo) and original-DWG
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
        "agentdrafter-stretch-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

fn make_loop_set(name: &str) -> PathBuf {
    let dir = scratch(name);
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  DEVICE:
    base_point: [0, 0, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 5}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        r#"units: mm
paper: [200, 140]
block_library: blocks.yaml
layers:
  wires: {color: green}
  enclosure: {color: white}
  locked_border: {color: 8, locked: true}
objects:
  - polyline:
      id: border
      layer: locked_border
      closed: true
      points: [[0, 0, 0], [200, 0, 0], [200, 140, 0], [0, 140, 0]]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        r#"rows:
  - sheet: LOOP-201
    template: template.yaml
    objects:
      - polyline:
          id: panel-box
          layer: enclosure
          closed: true
          points: [[20, 20, 0], [100, 20, 0], [100, 100, 0], [20, 100, 0]]
      - line: {id: wire-top, layer: wires, from: [30, 80, 0], to: [90, 80, 0]}
      - line: {id: wire-bot, layer: wires, from: [30, 40, 0], to: [90, 40, 0]}
      - insert: {id: dev-right, block: DEVICE, position: [90, 60, 0]}
      - insert: {id: dev-left, block: DEVICE, position: [30, 60, 0]}
      - text: {id: tag-right, layer: wires, contents: "PT-201", position: [85, 68, 0], height: 3}
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
fn stretch_makes_room_on_loop_sheet_using_snapped_box_lists_touched_and_undoes() {
    let dir = make_loop_set("room");
    let settings = Settings::default();
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    // Stretch right side of the enclosure + right ends of both wires + dev-right + tag-right
    // to the right by 30 mm, defining the box from a coordinate and a snapped corner of panel-box!
    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "STRETCH",
                        "sheet": "LOOP-201",
                        "box": [
                            [80, 15],
                            {"snap": {"id": "LOOP-201/panel-box", "snap": "endpoint", "which": "points[2]"}}
                        ],
                        "by": ["30 mm", 0, 0]
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

    let op = &report.operations[0];
    assert_eq!(op.snapped.len(), 1);
    // Reply lists EVERY object it touched (and does NOT touch dev-left or locked border!).
    assert_eq!(
        op.transformed,
        vec![
            "LOOP-201/panel-box",
            "LOOP-201/wire-top",
            "LOOP-201/wire-bot",
            "LOOP-201/dev-right",
            "LOOP-201/tag-right",
        ]
    );

    // Verify rebuilt DWG geometry.
    let sheet = read_dwg(&dir.join("dwg/LOOP-201.dwg")).unwrap();
    // panel-box stretched from x=20..100 to x=20..130!
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p)
            if p.closed
                && p.points.len() == 4
                && (p.points[0].x - 20.0).abs() < 1e-3
                && (p.points[1].x - 130.0).abs() < 1e-3
                && (p.points[2].x - 130.0).abs() < 1e-3
                && (p.points[3].x - 20.0).abs() < 1e-3
    )));
    // wire-top stretched from 30..90 to 30..120
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 30.0).abs() < 1e-3
                && (l.to.x - 120.0).abs() < 1e-3
                && (l.from.y - 80.0).abs() < 1e-3
    )));
    // dev-right moved whole from x=90 to x=120; dev-left stayed at x=30
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Insert(i) if (i.position.x - 120.0).abs() < 1e-3 && (i.position.y - 60.0).abs() < 1e-3
    )));
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Insert(i) if (i.position.x - 30.0).abs() < 1e-3 && (i.position.y - 60.0).abs() < 1e-3
    )));
    // tag-right moved whole from x=85 to x=115
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Text(t) if t.contents == "PT-201" && (t.position.x - 115.0).abs() < 1e-3
    )));

    // Display list renders cleanly
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "LOOP-201"})).unwrap();
    assert!(display_list_with(&dl_req, &settings).is_ok());

    // Undo restores rows.yaml byte for byte
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: report.journal_entry.unwrap(),
        dry_run: false,
    });
    assert!(undone.ok, "{undone:#?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn stretch_refuses_explicit_locked_target_and_degenerate_box() {
    let dir = make_loop_set("refusals");
    let settings = Settings::default();

    // Explicit locked target refused with Locked
    let locked = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "S",
                        "sheet": "LOOP-201",
                        "targets": [{"id": "LOOP-201/border"}],
                        "box": [[150, 0], [210, 150]],
                        "by": [10, 0, 0]
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    let rep = locked.result.unwrap();
    assert_eq!(rep.failed, 1);
    assert_eq!(
        rep.operations[0].error.as_ref().unwrap().code,
        ErrorCode::Locked
    );

    // Zero-width box refused
    let flat = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "stretch",
                        "sheet": "LOOP-201",
                        "box": [[80, 15], [80, 105]],
                        "by": [10, 0, 0]
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    let rep2 = flat.result.unwrap();
    assert_eq!(rep2.failed, 1);
    assert_eq!(
        rep2.operations[0].error.as_ref().unwrap().code,
        ErrorCode::BadValue
    );
}

#[test]
fn stretch_works_on_original_dwg_revisions() {
    let dir = make_loop_set("orig");
    let settings = Settings::default();
    let original = dir.join("LOOP-201.dwg");
    fs::copy(dir.join("dwg/LOOP-201.dwg"), &original).unwrap();
    let orig_bytes = fs::read(&original).unwrap();
    let rev = dir.join("LOOP-201.rev1.dwg");

    let reply = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "S",
                        "box": [[80, 15], [105, 105]],
                        "from": [90, 80, 0],
                        "to": [120, 80, 0]
                    }
                ]
            }),
            output: Some(rev.clone()),
            allow_loss: false,
            dry_run: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(reply.ok, "{reply:#?}");
    let rep = reply.result.unwrap();
    assert!(
        rep.unexpected_changes.is_empty(),
        "{:?}",
        rep.unexpected_changes
    );
    assert!(
        rep.requested_failures.is_empty(),
        "{:?}",
        rep.requested_failures
    );
    assert_eq!(rep.operations[0].transformed.len(), 5);
    assert_eq!(fs::read(&original).unwrap(), orig_bytes);

    let revised = read_dwg(&rev).unwrap();
    let stretched_wire = revised
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Line(l) if (l.from.y - 80.0).abs() < 1e-3 => Some(l),
            _ => None,
        })
        .expect("stretched top wire");
    close(stretched_wire.from.x, 30.0);
    close(stretched_wire.to.x, 120.0);
}
