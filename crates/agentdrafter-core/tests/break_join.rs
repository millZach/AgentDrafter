//! Integration tests for Rank 30 (#83): `break` (`BREAK`, `BR`,
//! `BREAKATPOINT`) and `join` (`JOIN`, `J`) across drawing sets (wire-crossing
//! gap via intersection snap, split at point, circle break, collinear join
//! across gap, mixed line+arc polyline join, closed loop join, ID carry-on and
//! deleted reporting, display list, undo, refusals) and original-DWG revisions.
//!
//! All drawings, layers, and geometry here are made up (ADR-0008).

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
        "agentdrafter-break-join-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

fn make_wire_set(name: &str) -> PathBuf {
    let dir = scratch(name);
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        r#"units: mm
paper: [200, 140]
layers:
  wires: {color: green}
  enclosure: {color: white}
  locked_l: {color: red, locked: true}
objects:
  - polyline:
      id: border
      layer: enclosure
      closed: true
      points: [[0, 0, 0], [200, 0, 0], [200, 140, 0], [0, 140, 0]]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        r#"rows:
  - sheet: LOOP-101
    template: template.yaml
    objects:
      - line: {id: wire-h, layer: wires, from: [10, 40, 0], to: [90, 40, 0]}
      - line: {id: wire-v, layer: wires, from: [50, 10, 0], to: [50, 70, 0]}
      - line: {id: wire-split, layer: wires, from: [10, 90, 0], to: [90, 90, 0]}
      - circle: {id: ring, layer: enclosure, center: [140, 40, 0], radius: 15}
      - line: {id: seg-a, layer: wires, from: [110, 80, 0], to: [130, 80, 0]}
      - arc: {id: seg-arc, layer: wires, center: [130, 85, 0], radius: 5, start_angle: 270, end_angle: 0}
      - line: {id: seg-b, layer: wires, from: [135, 105, 0], to: [135, 85, 0]}
      - line: {id: gap-l, layer: wires, from: [110, 120, 0], to: [140, 120, 0]}
      - line: {id: gap-r, layer: wires, from: [150, 120, 0], to: [180, 120, 0]}
      - line: {id: locked-w, layer: locked_l, from: [10, 120, 0], to: [80, 120, 0]}
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
fn break_wire_crossing_with_snap_split_circle_join_and_undo() {
    let dir = make_wire_set("all");
    let settings = Settings::default();
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    // 1. Break wire-h around its intersection with wire-v using a 6 mm gap.
                    {
                        "op": "BREAK",
                        "target": {"id": "LOOP-101/wire-h"},
                        "at": {
                            "snap": {
                                "id": "LOOP-101/wire-h",
                                "snap": "intersection",
                                "with": {"id": "LOOP-101/wire-v"}
                            }
                        },
                        "gap": "6 mm",
                        "new_id": "wire-h-right"
                    },
                    // 2. BREAKATPOINT splits wire-split at its midpoint (50, 90, 0).
                    {
                        "op": "BREAKATPOINT",
                        "target": {"id": "LOOP-101/wire-split"},
                        "at": {"snap": {"id": "LOOP-101/wire-split", "snap": "midpoint"}},
                        "new_id": "wire-split-2"
                    },
                    // 3. Two-point BR turns circle `ring` into a 270 deg arc (removes 0..90 deg).
                    {
                        "op": "BR",
                        "target": {"id": "LOOP-101/ring"},
                        "from": [155, 40, 0],
                        "to": [140, 55, 0]
                    },
                    // 4. JOIN out-of-order Line + Line + Arc into one Polyline with bulge.
                    {
                        "op": "J",
                        "targets": [
                            {"id": "LOOP-101/seg-a"},
                            {"id": "LOOP-101/seg-b"},
                            {"id": "LOOP-101/seg-arc"}
                        ]
                    },
                    // 5. JOIN two collinear lines across a 10 mm gap back into one Line!
                    {
                        "op": "JOIN",
                        "targets": [
                            {"id": "LOOP-101/gap-l"},
                            {"id": "LOOP-101/gap-r"}
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

    // Check ID carry-on, new split_id, and deleted reporting.
    assert_eq!(report.operations[0].id.as_deref(), Some("LOOP-101/wire-h"));
    assert_eq!(
        report.operations[0].split_id.as_deref(),
        Some("LOOP-101/wire-h-right")
    );
    assert_eq!(report.operations[0].snapped.len(), 1);
    assert_eq!(
        report.operations[1].id.as_deref(),
        Some("LOOP-101/wire-split")
    );
    assert_eq!(
        report.operations[1].split_id.as_deref(),
        Some("LOOP-101/wire-split-2")
    );
    assert_eq!(report.operations[2].id.as_deref(), Some("LOOP-101/ring"));
    assert_eq!(report.operations[2].split_id, None);
    assert_eq!(report.operations[3].id.as_deref(), Some("LOOP-101/seg-a"));
    assert_eq!(
        report.operations[3].deleted,
        vec!["LOOP-101/seg-b", "LOOP-101/seg-arc"]
    );
    assert_eq!(report.operations[4].id.as_deref(), Some("LOOP-101/gap-l"));
    assert_eq!(report.operations[4].deleted, vec!["LOOP-101/gap-r"]);

    // Verify rebuilt DWG geometry.
    let sheet = read_dwg(&dir.join("dwg/LOOP-101.dwg")).unwrap();
    // wire-h broken into [10..47] and [53..90] at y = 40
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 10.0).abs() < 1e-3
                && (l.to.x - 47.0).abs() < 1e-3
                && (l.from.y - 40.0).abs() < 1e-3
    )));
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 53.0).abs() < 1e-3
                && (l.to.x - 90.0).abs() < 1e-3
                && (l.from.y - 40.0).abs() < 1e-3
    )));
    // ring converted from circle to arc (90..0 deg)
    let ring_arc = sheet
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Arc(a)
                if (a.center.x - 140.0).abs() < 1e-3 && (a.center.y - 40.0).abs() < 1e-3 =>
            {
                Some(a)
            }
            _ => None,
        })
        .expect("broken circle is now an arc");
    close(ring_arc.start_angle, 90.0);
    close(ring_arc.end_angle, 0.0);
    // Joined polyline seg-a + seg-arc + seg-b has 4 vertices and 1 non-zero bulge
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p)
            if !p.closed
                && p.points.len() == 4
                && p.bulges.iter().filter(|b| b.abs() > 1e-6).count() == 1
    )));
    // Joined collinear line spans 110..180 at y = 120
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 110.0).abs() < 1e-3
                && (l.to.x - 180.0).abs() < 1e-3
                && (l.from.y - 120.0).abs() < 1e-3
    )));

    // One drawing pipeline renders cleanly
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "LOOP-101"})).unwrap();
    assert!(display_list_with(&dl_req, &settings).is_ok());

    // Undo restores rows.yaml byte for byte
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: report.journal_entry.unwrap(),
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:#?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn break_and_join_refuse_locked_layers_and_disconnected_objects() {
    let dir = make_wire_set("refusals");
    let settings = Settings::default();

    let locked = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "break",
                        "target": {"id": "LOOP-101/locked-w"},
                        "at": [40, 120, 0]
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

    // Disconnected non-collinear lines refused by join
    let dis = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "join",
                        "targets": [
                            {"id": "LOOP-101/wire-h"},
                            {"id": "LOOP-101/seg-a"}
                        ]
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    let rep2 = dis.result.unwrap();
    assert_eq!(rep2.failed, 1);
    assert_eq!(
        rep2.operations[0].error.as_ref().unwrap().code,
        ErrorCode::BadValue
    );
}

#[test]
fn break_and_join_work_on_original_dwg_revisions() {
    let dir = make_wire_set("orig");
    let settings = Settings::default();
    let original = dir.join("LOOP-101.dwg");
    fs::copy(dir.join("dwg/LOOP-101.dwg"), &original).unwrap();
    let orig_bytes = fs::read(&original).unwrap();
    let rev = dir.join("LOOP-101.rev1.dwg");

    let reply = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "BREAK",
                        "target": {"id": "LOOP-101/wire-h"},
                        "at": [50, 40, 0],
                        "gap": 6,
                        "new_id": "wire-h-right"
                    },
                    {
                        "op": "JOIN",
                        "targets": [
                            {"id": "LOOP-101/gap-l"},
                            {"id": "LOOP-101/gap-r"}
                        ]
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
    assert_eq!(
        rep.operations[0].split_id.as_deref(),
        Some("LOOP-101/wire-h-right")
    );
    assert_eq!(rep.operations[1].deleted, vec!["LOOP-101/gap-r"]);
    assert_eq!(fs::read(&original).unwrap(), orig_bytes);

    let revised = read_dwg(&rev).unwrap();
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 110.0).abs() < 1e-3
                && (l.to.x - 180.0).abs() < 1e-3
                && (l.from.y - 120.0).abs() < 1e-3
    )));
}
