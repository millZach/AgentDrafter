//! Integration tests for Rank 28 (#81): `trim` (`TRIM` / `TR`) and `extend`
//! (`EXTEND` / `EX`) across drawing sets, block-insert boundaries, splits,
//! undo, and original `.dwg` revisions.
//!
//! All drawings, blocks, and geometry here are made up (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, read_dwg, revise_with, undo, ChangeRequest,
    DisplayListRequest, ErrorCode, Object, ReviseRequest, SetBuildRequest, Settings, UndoRequest,
};

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-trim-extend-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default()
}

fn write_loop_set(root: &Path) {
    fs::write(
        root.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        root.join("blocks.yaml"),
        r#"units: mm
blocks:
  DEVICE:
    base_point: [0, 0, 0]
    objects:
      - polyline: {points: [[-10, -10, 0], [10, -10, 0], [10, 10, 0], [-10, 10, 0]], closed: true}
"#,
    )
    .unwrap();
    fs::write(
        root.join("template.yaml"),
        r#"units: mm
paper: [210, 148]
block_library: blocks.yaml
layers:
  0: {}
  wires: {color: 4}
  locked_wires: {locked: true}
objects:
  - polyline: {id: border, points: [[0, 0, 0], [210, 0, 0], [210, 148, 0], [0, 148, 0]], closed: true}
"#,
    )
    .unwrap();
    fs::write(
        root.join("rows.yaml"),
        r#"rows:
  - sheet: LOOP-001
    template: template.yaml
    objects:
      - line: {id: bus-left, from: [40, 20, 0], to: [40, 120, 0]}
      - line: {id: bus-right, from: [140, 20, 0], to: [140, 120, 0]}
      - insert: {id: transmitter, block: DEVICE, position: [90, 90, 0]}
      - line: {id: wire-through, from: [20, 90, 0], to: [160, 90, 0], layer: wires}
      - line: {id: wire-short, from: [60, 60, 0], to: [110, 60, 0], layer: wires}
      - line: {id: wire-long, from: [20, 40, 0], to: [160, 40, 0], layer: wires}
      - circle: {id: ring, center: [90, 40, 0], radius: 10, layer: wires}
      - polyline: {id: jumper, points: [[55, 110, 0], [125, 110, 0], [125, 105, 0]], closed: false, layer: wires}
      - line: {id: locked-wire, from: [20, 30, 0], to: [160, 30, 0], layer: locked_wires}
"#,
    )
    .unwrap();
}

#[test]
fn trim_split_block_boundary_extend_and_undo() {
    let dir = scratch("trim-extend-set");
    write_loop_set(&dir);
    let settings = no_system_fonts();
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    // 1. Trim wire-through against transmitter block insert with keep: "ends" (splits wire in two!).
    // 2. Extend wire-short at both ends to bus-left (x=40) and bus-right (x=140).
    // 3. Trim wire-long to keep "between" bus-left and bus-right.
    // 4. Trim circle ring against wire-long (horizontal y=40) keeping top semicircle ("start").
    // 5. Extend jumper start back to bus-left (x=40).
    let applied = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "TR",
                        "target": {"id": "LOOP-001/wire-through"},
                        "boundary": {"id": "LOOP-001/transmitter"},
                        "keep": "ends",
                        "new_id": "wire-through-right"
                    },
                    {
                        "op": "EX",
                        "target": {"id": "LOOP-001/wire-short"},
                        "boundaries": [{"id": "LOOP-001/bus-left"}, {"id": "LOOP-001/bus-right"}],
                        "end": "both"
                    },
                    {
                        "op": "TRIM",
                        "target": {"id": "LOOP-001/wire-long"},
                        "boundaries": [{"id": "LOOP-001/bus-left"}, {"id": "LOOP-001/bus-right"}],
                        "keep": "between"
                    },
                    {
                        "op": "trim",
                        "target": {"id": "LOOP-001/ring"},
                        "boundary": {"id": "LOOP-001/wire-long"},
                        "keep": "start"
                    },
                    {
                        "op": "extend",
                        "target": {"id": "LOOP-001/jumper"},
                        "boundary": {"id": "LOOP-001/bus-left"},
                        "end": "start"
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(applied.ok, "{applied:?}");
    let report = applied.result.unwrap();
    let entry = report.journal_entry.expect("journal entry");
    assert_eq!(report.landed, 1);
    assert_eq!(
        report.operations[0].id.as_deref(),
        Some("LOOP-001/wire-through")
    );
    assert_eq!(
        report.operations[0].split_id.as_deref(),
        Some("LOOP-001/wire-through-right")
    );

    // Check DWG geometry!
    let dwg = read_dwg(&dir.join("out/LOOP-001.dwg")).unwrap();
    // wire-through kept left [20..80], split right [100..160] (transmitter box is x=[80..100]).
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Line(l) if (l.from.y - 90.0).abs() < 1e-4 && (l.from.x - 20.0).abs() < 1e-4 && (l.to.x - 80.0).abs() < 1e-4
    )));
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Line(l) if (l.from.y - 90.0).abs() < 1e-4 && (l.from.x - 100.0).abs() < 1e-4 && (l.to.x - 160.0).abs() < 1e-4
    )));
    // wire-short extended both ends to [40..140].
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Line(l) if (l.from.y - 60.0).abs() < 1e-4 && (l.from.x - 40.0).abs() < 1e-4 && (l.to.x - 140.0).abs() < 1e-4
    )));
    // wire-long trimmed between [40..140].
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Line(l) if (l.from.y - 40.0).abs() < 1e-4 && (l.from.x - 40.0).abs() < 1e-4 && (l.to.x - 140.0).abs() < 1e-4
    )));
    // ring trimmed into top semicircular Arc (0..180 deg).
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Arc(a) if (a.radius - 10.0).abs() < 1e-4 && a.start_angle.abs() < 1e-3 && (a.end_angle - 180.0).abs() < 1e-3
    )));
    // jumper start extended back to x=40.
    assert!(dwg.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p) if !p.closed && (p.points[0].x - 40.0).abs() < 1e-4 && (p.points[0].y - 110.0).abs() < 1e-4
    )));

    // One drawing pipeline renders display list cleanly.
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "LOOP-001"})).unwrap();
    let dl = display_list_with(&dl_req, &settings);
    assert!(dl.is_ok(), "{dl:?}");

    // Undo restores rows.yaml byte-for-byte.
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn trim_and_extend_refuse_locked_layer_and_missing_crossings() {
    let dir = scratch("refusals");
    write_loop_set(&dir);
    let settings = no_system_fonts();

    let locked = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "trim",
                        "target": {"id": "LOOP-001/locked-wire"},
                        "boundary": {"id": "LOOP-001/bus-left"},
                        "keep": "start"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!locked.ok);
    let err = locked.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(err.code, ErrorCode::Locked);

    let no_hit = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "extend",
                        "target": {"id": "LOOP-001/ring"},
                        "boundary": {"id": "LOOP-001/bus-left"},
                        "end": "end"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!no_hit.ok);
    let err = no_hit.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);
}

#[test]
fn trim_and_extend_work_on_original_dwg() {
    let dir = scratch("original-dwg");
    write_loop_set(&dir);
    let settings = no_system_fonts();

    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: false,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{built:?}");

    let original = dir.join("LOOP-001.dwg");
    fs::copy(dir.join("out/LOOP-001.dwg"), &original).unwrap();
    let orig_bytes = fs::read(&original).unwrap();

    let rev = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "TRIM",
                        "target": {"id": "LOOP-001/wire-through"},
                        "boundary": {"id": "LOOP-001/transmitter"},
                        "keep": "ends",
                        "new_id": "wire-right"
                    },
                    {
                        "op": "EXTEND",
                        "target": {"id": "LOOP-001/wire-short"},
                        "boundaries": [{"id": "LOOP-001/bus-left"}, {"id": "LOOP-001/bus-right"}],
                        "end": "both"
                    }
                ]
            }),
            output: Some(dir.join("LOOP-001.rev1.dwg")),
            allow_loss: false,
            dry_run: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev.ok, "{rev:?}");
    let report = rev.result.unwrap();
    assert!(
        report.unexpected_changes.is_empty(),
        "{:?}",
        report.unexpected_changes
    );
    assert!(
        report.requested_failures.is_empty(),
        "{:?}",
        report.requested_failures
    );
    assert_eq!(
        report.operations[0].split_id.as_deref(),
        Some("LOOP-001/wire-right")
    );
    assert_eq!(fs::read(&original).unwrap(), orig_bytes);

    let revised = read_dwg(&dir.join("LOOP-001.rev1.dwg")).unwrap();
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Line(l) if (l.from.y - 60.0).abs() < 1e-4 && (l.from.x - 40.0).abs() < 1e-4 && (l.to.x - 140.0).abs() < 1e-4
    )));
}
