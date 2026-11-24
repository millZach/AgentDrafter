//! Integration tests for `fillet` (`FILLET`, `F`) and `chamfer` (`CHAMFER`,
//! `CHA`) across drawing sets (including two-object mode, radius 0 squaring,
//! no-trim mode, polyline-wide mode, display-list rendering, journal & undo,
//! locked layers) and original-DWG revisions (#82).
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
        "agentdrafter-fillet-chamfer-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

fn make_corner_set(name: &str) -> PathBuf {
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
  - sheet: PNL-001
    template: template.yaml
    objects:
      - line: {id: w1, layer: wires, from: [10, 20, 0], to: [45, 20, 0]}
      - line: {id: w2, layer: wires, from: [50, 25, 0], to: [50, 60, 0]}
      - line: {id: s1, layer: wires, from: [10, 80, 0], to: [35, 80, 0]}
      - line: {id: s2, layer: wires, from: [40, 85, 0], to: [40, 110, 0]}
      - line: {id: c1, layer: wires, from: [70, 20, 0], to: [100, 20, 0]}
      - line: {id: c2, layer: wires, from: [100, 20, 0], to: [100, 50, 0]}
      - polyline:
          id: box_f
          layer: enclosure
          closed: true
          points: [[120, 20, 0], [160, 20, 0], [160, 50, 0], [120, 50, 0]]
      - polyline:
          id: box_c
          layer: enclosure
          closed: true
          points: [[120, 70, 0], [160, 70, 0], [160, 100, 0], [120, 100, 0]]
      - line: {id: lock1, layer: locked_l, from: [70, 80, 0], to: [90, 80, 0]}
      - line: {id: lock2, layer: locked_l, from: [90, 80, 0], to: [90, 100, 0]}
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
fn fillet_and_chamfer_two_objects_radius_zero_polylines_display_list_and_undo() {
    let dir = make_corner_set("all");
    let settings = Settings::default();
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    // 1. Fillet two disjoint wires with radius 5 mm (AutoCAD alias `F`).
                    {
                        "op": "F",
                        "first": {"id": "PNL-001/w1"},
                        "second": {"id": "PNL-001/w2"},
                        "radius": "5 mm",
                        "new_id": "wire-bend"
                    },
                    // 2. Fillet two disjoint wires with radius 0 to bring them to a sharp corner.
                    {
                        "op": "FILLET",
                        "first": {"id": "PNL-001/s1"},
                        "second": {"id": "PNL-001/s2"},
                        "radius": 0
                    },
                    // 3. Chamfer two lines with asymmetric distances 6 and 4 (AutoCAD alias `CHA`).
                    {
                        "op": "CHA",
                        "first": {"id": "PNL-001/c1"},
                        "second": {"id": "PNL-001/c2"},
                        "distance1": 6,
                        "distance2": 4,
                        "new_id": "corner-bevel"
                    },
                    // 4. Fillet every corner of a closed enclosure polyline (radius 5).
                    {
                        "op": "fillet",
                        "target": {"id": "PNL-001/box_f"},
                        "radius": 5
                    },
                    // 5. Chamfer every corner of a closed enclosure polyline (equal distance 4).
                    {
                        "op": "chamfer",
                        "target": {"id": "PNL-001/box_c"},
                        "distance": 4
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
    assert_eq!(
        report.operations[0].id.as_deref(),
        Some("PNL-001/wire-bend")
    );
    assert_eq!(report.operations[0].transformed.len(), 2);
    assert_eq!(report.operations[1].id, None);
    assert_eq!(report.operations[1].transformed.len(), 2);
    assert_eq!(
        report.operations[2].id.as_deref(),
        Some("PNL-001/corner-bevel")
    );

    // Read the rebuilt DWG and verify every modified/created object.
    let sheet = read_dwg(&dir.join("dwg/PNL-001.dwg")).unwrap();

    // Fillet arc centered at (45, 25), radius 5, 270..0 deg.
    let arc = sheet
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Arc(a)
                if (a.center.x - 45.0).abs() < 1e-3 && (a.center.y - 25.0).abs() < 1e-3 =>
            {
                Some(a)
            }
            _ => None,
        })
        .expect("fillet arc in rebuilt DWG");
    close(arc.radius, 5.0);
    close(arc.start_angle, 270.0);
    close(arc.end_angle, 0.0);

    // Radius 0 squared corner at (40, 80)
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 10.0).abs() < 1e-3
                && (l.from.y - 80.0).abs() < 1e-3
                && (l.to.x - 40.0).abs() < 1e-3
                && (l.to.y - 80.0).abs() < 1e-3
    )));
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 40.0).abs() < 1e-3
                && (l.from.y - 80.0).abs() < 1e-3
                && (l.to.x - 40.0).abs() < 1e-3
                && (l.to.y - 110.0).abs() < 1e-3
    )));

    // Asymmetric chamfer bevel from (94, 20) to (100, 24)
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Line(l)
            if (l.from.x - 94.0).abs() < 1e-3
                && (l.from.y - 20.0).abs() < 1e-3
                && (l.to.x - 100.0).abs() < 1e-3
                && (l.to.y - 24.0).abs() < 1e-3
    )));

    // Filleted polyline has 8 vertices and 4 arc bulges
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p)
            if p.closed
                && p.points.len() == 8
                && p.bulges.iter().filter(|b| b.abs() > 1e-6).count() == 4
    )));

    // Chamfered polyline has 8 vertices and no bulges
    assert!(sheet.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p)
            if p.closed
                && p.points.len() == 8
                && p.bulges.iter().all(|b| b.abs() <= 1e-9)
    )));

    // Display list renders cleanly through the single pipeline
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "PNL-001"})).unwrap();
    let dl = display_list_with(&dl_req, &settings);
    assert!(dl.is_ok(), "{dl:#?}");

    // Undo restores rows.yaml byte for byte
    let u = undo(&UndoRequest {
        set: dir.clone(),
        change_set: report.journal_entry.unwrap(),
        dry_run: false,
    });
    assert!(u.ok, "{u:#?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn fillet_and_chamfer_refuse_locked_layers_and_bad_inputs() {
    let dir = make_corner_set("refusals");
    let settings = Settings::default();

    // Locked layer refused when trim is true
    let locked = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "fillet",
                        "first": {"id": "PNL-001/lock1"},
                        "second": {"id": "PNL-001/lock2"},
                        "radius": 2
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

    // No-trim (`trim: false`) on locked lines still works (does not edit locked objects!)
    let no_trim = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "fillet",
                        "first": {"id": "PNL-001/lock1"},
                        "second": {"id": "PNL-001/lock2"},
                        "radius": 2,
                        "trim": false,
                        "layer": "wires",
                        "new_id": "nt-arc"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(no_trim.ok, "{no_trim:#?}");
    assert_eq!(no_trim.result.unwrap().landed, 1);

    // Radius too large for legs refused
    let too_big = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "fillet",
                        "first": {"id": "PNL-001/c1"},
                        "second": {"id": "PNL-001/c2"},
                        "radius": 500
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    let rep2 = too_big.result.unwrap();
    assert_eq!(rep2.failed, 1);
    assert_eq!(
        rep2.operations[0].error.as_ref().unwrap().code,
        ErrorCode::BadValue
    );
}

#[test]
fn fillet_and_chamfer_work_on_original_dwg_revisions() {
    let dir = make_corner_set("orig");
    let settings = Settings::default();
    let original = dir.join("PNL-001.dwg");
    fs::copy(dir.join("dwg/PNL-001.dwg"), &original).unwrap();
    let orig_bytes = fs::read(&original).unwrap();
    let rev = dir.join("PNL-001.rev1.dwg");

    let reply = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "F",
                        "first": {"id": "PNL-001/w1"},
                        "second": {"id": "PNL-001/w2"},
                        "radius": 5,
                        "new_id": "rev-bend"
                    },
                    {
                        "op": "CHA",
                        "target": {"id": "PNL-001/box_c"},
                        "length": 5,
                        "angle": 45
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
    assert_eq!(rep.operations[0].id.as_deref(), Some("PNL-001/rev-bend"));
    assert_eq!(fs::read(&original).unwrap(), orig_bytes);
    assert!(rev.exists());

    let revised = read_dwg(&rev).unwrap();
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Arc(a)
            if (a.center.x - 45.0).abs() < 1e-3
                && (a.center.y - 25.0).abs() < 1e-3
                && (a.radius - 5.0).abs() < 1e-3
    )));
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p) if p.closed && p.points.len() == 8
    )));
}
