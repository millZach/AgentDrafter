//! Integration tests for Rank 27 (#80): `offset` (`OFFSET` / `O`) across
//! drawing sets, the one drawing pipeline, and original `.dwg` revisions.
//!
//! All drawings and geometry here are made up (ADR-0008).

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
        "agentdrafter-offset-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default()
}

fn write_panel_set(root: &Path) {
    fs::write(
        root.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        root.join("template.yaml"),
        r#"units: mm
paper: [210, 148]
layers:
  0: {}
  enclosure: {color: 7}
  wireway: {color: 4}
  locked_layer: {locked: true}
objects:
  - polyline: {id: border, points: [[0, 0, 0], [210, 0, 0], [210, 148, 0], [0, 148, 0]], closed: true}
"#,
    )
    .unwrap();
    fs::write(
        root.join("rows.yaml"),
        r#"rows:
  - sheet: PNL-001
    template: template.yaml
    objects:
      - polyline:
          id: cabinet
          layer: enclosure
          points: [[20, 20, 0], [120, 20, 0], [120, 100, 0], [20, 100, 0]]
          closed: true
      - polyline:
          id: duct
          layer: wireway
          points: [[30, 40, 0], {point: [60, 40, 0], bulge: 1.0}, [60, 60, 0], [30, 60, 0]]
          closed: false
      - line: {id: rail, from: [30, 80, 0], to: [110, 80, 0], layer: wireway}
      - circle: {id: knockout, center: [160, 70, 0], radius: 12, layer: enclosure}
      - arc: {id: swing, center: [160, 110, 0], radius: 15, start_angle: 0, end_angle: 90, layer: enclosure}
      - ellipse: {id: grommet, center: [160, 35, 0], major_axis: [12, 0, 0], ratio: 0.5, layer: enclosure}
      - spline: {id: harness, fit_points: [[30, 115, 0], [60, 128, 0], [90, 115, 0]], layer: wireway}
      - text: {id: label, position: [25, 130, 0], height: 4, contents: "PANEL P1", layer: locked_layer}
"#,
    )
    .unwrap();
}

#[test]
fn offset_all_six_curve_kinds_builds_draws_and_undoes() {
    let dir = scratch("all-six");
    write_panel_set(&dir);
    let settings = no_system_fonts();

    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    let cs = json!({
        "operations": [
            {
                "op": "OFFSET",
                "target": {"id": "PNL-001/cabinet"},
                "distance": "5 mm",
                "side": "inside",
                "id": "inner-wall"
            },
            {
                "op": "O",
                "target": {"id": "PNL-001/duct"},
                "distance": 3,
                "side": "right",
                "id": "duct-outer"
            },
            {
                "op": "offset",
                "target": {"id": "PNL-001/rail"},
                "through": [70, 86, 0],
                "id": "rail-top",
                "layer": "enclosure"
            },
            {
                "op": "offset",
                "target": {"id": "PNL-001/knockout"},
                "distance": 2,
                "side": "outside",
                "id": "knockout-outer"
            },
            {
                "op": "offset",
                "target": {"id": "PNL-001/swing"},
                "distance": 3,
                "side": "inside",
                "id": "swing-inner"
            },
            {
                "op": "offset",
                "target": {"id": "PNL-001/grommet"},
                "distance": 2,
                "side": "outside",
                "id": "grommet-outer"
            },
            {
                "op": "offset",
                "target": {"id": "PNL-001/harness"},
                "distance": 2.5,
                "side": "left",
                "id": "harness-left"
            }
        ]
    });

    let applied = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs,
            dry_run: false,
        },
        &settings,
    );
    assert!(applied.ok, "{applied:?}");
    let report = applied.result.unwrap();
    let entry = report.journal_entry.expect("journal entry");
    assert_eq!(report.landed, 1);
    assert_eq!(report.operations.len(), 7);
    assert_eq!(
        report.operations[0].id.as_deref(),
        Some("PNL-001/inner-wall")
    );
    assert_eq!(report.operations[2].id.as_deref(), Some("PNL-001/rail-top"));

    // Verify DWG reads back cleanly with all offset curves.
    let dwg = read_dwg(&dir.join("out/PNL-001.dwg")).unwrap();
    let inner = dwg
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Polyline(p)
                if p.closed && p.points.len() == 4 && (p.points[0].x - 25.0).abs() < 1e-4 =>
            {
                Some(p)
            }
            _ => None,
        })
        .expect("offset inner cabinet polyline");
    assert!((inner.points[2].x - 115.0).abs() < 1e-4);
    assert!((inner.points[2].y - 95.0).abs() < 1e-4);

    let knockout_outer = dwg
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Circle(c) if (c.radius - 14.0).abs() < 1e-4 => Some(c),
            _ => None,
        })
        .expect("offset outer circle");
    assert!((knockout_outer.center.x - 160.0).abs() < 1e-4);

    let swing_inner = dwg
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Arc(a) if (a.radius - 12.0).abs() < 1e-4 => Some(a),
            _ => None,
        })
        .expect("offset inner arc");
    assert!((swing_inner.end_angle - 90.0).abs() < 1e-4);

    // One drawing pipeline renders display list cleanly.
    let dl_req: DisplayListRequest =
        serde_json::from_value(json!({"source": dir, "sheet": "PNL-001"})).unwrap();
    let dl = display_list_with(&dl_req, &settings);
    assert!(dl.is_ok(), "{dl:?}");

    // Undo restores rows.yaml byte-for-byte.
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn offset_erase_source_snapped_side_and_refusals() {
    let dir = scratch("erase-refusals");
    write_panel_set(&dir);
    let settings = no_system_fonts();

    // 1. Offset with erase_source: true and a snapped side point.
    let ok = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "offset",
                        "target": {"id": "PNL-001/rail"},
                        "distance": 4,
                        "side": {"snap": {"id": "PNL-001/knockout", "snap": "center"}},
                        "id": "rail-shifted",
                        "erase_source": true
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(ok.ok, "{ok:?}");
    let rep = ok.result.unwrap();
    assert_eq!(rep.operations[0].snapped.len(), 1);
    let dwg = read_dwg(&dir.join("out/PNL-001.dwg")).unwrap();
    let lines: Vec<_> = dwg
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1);
    // Knockout center is at y=70 (below rail y=80) -> rail shifted down to y=76.
    assert!((lines[0].from.y - 76.0).abs() < 1e-4);

    // 2. Unsupported object type (text) is refused cleanly.
    let bad_kind = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "offset",
                        "target": {"id": "PNL-001/label"},
                        "distance": 2,
                        "side": "left"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!bad_kind.ok);
    let err = bad_kind.result.unwrap().operations[0]
        .error
        .clone()
        .unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);

    // 3. Inward collapse of circle is refused cleanly.
    let collapse = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "offset",
                        "target": {"id": "PNL-001/knockout"},
                        "distance": 15,
                        "side": "inside"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!collapse.ok);
    let err = collapse.result.unwrap().operations[0]
        .error
        .clone()
        .unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);
}

#[test]
fn offset_works_on_original_dwg() {
    let dir = scratch("original-dwg");
    write_panel_set(&dir);
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

    let original = dir.join("PNL-001.dwg");
    fs::copy(dir.join("out/PNL-001.dwg"), &original).unwrap();
    let orig_bytes = fs::read(&original).unwrap();

    let rev = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "OFFSET",
                        "target": {"id": "PNL-001/cabinet"},
                        "distance": 5,
                        "side": "inside",
                        "id": "inner-cabinet"
                    },
                    {
                        "op": "O",
                        "target": {"id": "PNL-001/knockout"},
                        "distance": 3,
                        "side": "outside",
                        "id": "outer-knockout"
                    }
                ]
            }),
            output: Some(dir.join("PNL-001.rev1.dwg")),
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
    assert_eq!(fs::read(&original).unwrap(), orig_bytes);

    let revised = read_dwg(&dir.join("PNL-001.rev1.dwg")).unwrap();
    assert!(revised
        .objects
        .iter()
        .any(|o| matches!(o, Object::Circle(c) if (c.radius - 15.0).abs() < 1e-4)));
    assert!(revised.objects.iter().any(|o| matches!(
        o,
        Object::Polyline(p) if p.closed && p.points.len() == 4 && (p.points[0].x - 25.0).abs() < 1e-4
    )));
}
