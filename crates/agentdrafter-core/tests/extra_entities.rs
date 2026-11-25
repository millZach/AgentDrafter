//! End-to-end tests for Rank 40 (#94): points (`POINT`, `PDMODE`, `PDSIZE`),
//! construction lines (`XLINE`), rays (`RAY`), multilines (`MLINE`, `MLSTYLE`),
//! GD&T tolerances (`TOLERANCE`), centre marks (`CENTERMARK`), centre lines
//! (`CENTERLINE`), and compiled SHX shapes (`SHAPE`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_with_format, change_with, display_list_with, plot_with, query_with, read_original,
    render_with, revise_with, undo, Answer, BuildRequest, ChangeRequest, DisplayListRequest,
    DrawingFormat, Object, PlotRequest, Query, QueryRequest, ReadRequest, RenderRequest,
    ReviseRequest, Selection, Settings, Shape, UndoRequest,
};
use serde_json::json;

static NEXT: AtomicU64 = AtomicU64::new(1);

fn temp_dir(tag: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-extra-{tag}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds a made-up `AutoCAD-86 shapes 1.0` `.shx` shape file in memory with
/// two vector shapes: #1 `DIAMOND` (closed diamond) and #2 `TICK` (vertical
/// stroke). Never contains Autodesk material (ADR-0008).
fn make_demo_shx() -> Vec<u8> {
    let diamond_prog: &[u8] = &[
        8, 10, 10, // (0,0) -> (10,10)
        8, 246, 10, // (-10, +10) -> (0,20)
        8, 246, 246, // (-10, -10) -> (-10,10)
        8, 10, 246, // (+10, -10) -> (0,0)
        0,
    ];
    let tick_prog: &[u8] = &[8, 0, 10, 0];
    let mut def1 = b"DIAMOND\0".to_vec();
    def1.extend_from_slice(diamond_prog);
    let mut def2 = b"TICK\0".to_vec();
    def2.extend_from_slice(tick_prog);

    let defs: &[(u16, Vec<u8>)] = &[(1, def1), (2, def2)];
    let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend((defs.len() as u16).to_le_bytes());
    for (num, def) in defs {
        bytes.extend(num.to_le_bytes());
        bytes.extend((def.len() as u16).to_le_bytes());
    }
    for (_, def) in defs {
        bytes.extend(def);
    }
    bytes.extend(b"EOF");
    bytes
}

fn settings_with_fonts(dir: &std::path::Path) -> Settings {
    Settings {
        font_folders: vec![dir.to_path_buf()],
        system_font_folders: Vec::new(),
        ..Settings::default()
    }
}

fn full_extra_sheet_yaml() -> &'static str {
    r#"units: mm
paper: ISO A4 landscape
pdmode: 35
pdsize: 3.0
linetypes:
  CENTER: {pattern: [12, -2, 2, -2]}
layers:
  GEOM: {color: 7}
  WALLS: {color: 3}
  ANNO: {color: 4}
  CL: {color: 1, linetype: CENTER}
mline_styles:
  WALL:
    description: Double wall with centre axis
    elements:
      - {offset: 5, color: 3}
      - {offset: 0, color: 1, linetype: CENTER}
      - {offset: -5, color: 3}
    fill_color: 8
    display_joints: true
    start_square_cap: true
    end_round_cap: true
objects:
  - line:
      id: border
      from: [10, 10, 0]
      to: [280, 10, 0]
      layer: GEOM
  - point:
      id: pt1
      position: [40, 60, 0]
      angle: 15
      layer: ANNO
  - xline:
      id: xl1
      through: [100, 100, 0]
      direction: [1, 0, 0]
      layer: GEOM
  - ray:
      id: ray1
      from: [50, 50, 0]
      direction: [1, 1, 0]
      layer: GEOM
  - mline:
      id: ml1
      points: [[30, 130, 0], [110, 130, 0], [110, 175, 0]]
      style: WALL
      scale: 1.0
      justification: zero
      layer: WALLS
  - tolerance:
      id: tol1
      position: [150, 140, 0]
      text: "{\\Fgdt;j}%%v{\\Fgdt;n}0.05{\\Fgdt;m}%%vA%%vB^J{\\Fgdt;b}%%v0.02%%vA"
      direction: 0
      text_height: 3.5
      layer: ANNO
  - circle:
      id: hole1
      center: [210, 80, 0]
      radius: 15
      layer: GEOM
  - center_mark:
      id: cm1
      center: [210, 80, 0]
      radius: 15
      cross_size: 4
      cross_gap: 1.5
      extension: 3
      show_extensions: true
      layer: CL
  - center_line:
      id: cl1
      from: [140, 60, 0]
      to: [185, 60, 0]
      start_extension: 4
      end_extension: 4
      layer: CL
  - shape:
      id: sh1
      file: demo_shapes.shx
      shape: DIAMOND
      position: [240, 140, 0]
      size: 2.0
      rotation: 0
      layer: ANNO
"#
}

#[test]
fn all_eight_objects_draw_across_display_list_png_and_pdf() {
    let dir = temp_dir("draw");
    fs::write(dir.join("demo_shapes.shx"), make_demo_shx()).unwrap();
    let yaml_path = dir.join("extra.yaml");
    fs::write(&yaml_path, full_extra_sheet_yaml()).unwrap();
    let settings = settings_with_fonts(&dir);

    // 1. Display list contains drawn geometry for every object, with xline/ray finite & clipped.
    let dl = display_list_with(&DisplayListRequest::new(&yaml_path), &settings).unwrap();
    let list = &dl.display_list;
    assert!(
        list.items.len() >= 10,
        "expected drawn items for all 8 extra objects"
    );
    for idx in 0..10 {
        assert!(
            list.items.iter().any(|it| it.object == idx),
            "object index {idx} produced no display list items"
        );
    }
    // Check xline (index 2) and ray (index 3) have finite clipped coordinates inside paper bounds.
    for idx in [2usize, 3] {
        let mut stroke_count = 0;
        for item in list.items.iter().filter(|it| it.object == idx) {
            if let Shape::Stroke { runs, .. } = &item.shape {
                stroke_count += runs.len();
                for run in runs {
                    for &[x, y] in run {
                        assert!(
                            x.is_finite() && y.is_finite(),
                            "non-finite coord on obj {idx}"
                        );
                        assert!(
                            (-50.0..=350.0).contains(&x) && (-50.0..=260.0).contains(&y),
                            "xline/ray coord [{x}, {y}] escaped sheet bounds"
                        );
                    }
                }
            }
        }
        assert!(
            stroke_count > 0,
            "expected clipped strokes for object {idx}"
        );
    }
    // Check MLine (index 4) emitted both Fill (fill_color: 8) and Stroke shapes.
    assert!(
        list.items
            .iter()
            .any(|it| it.object == 4 && matches!(it.shape, Shape::Fill { .. })),
        "mline with fill_color should emit a Fill shape"
    );
    // Check Tolerance (index 5) emitted frame/symbol strokes.
    assert!(
        list.items
            .iter()
            .any(|it| it.object == 5 && matches!(it.shape, Shape::Stroke { .. })),
        "tolerance should emit frame/symbol strokes"
    );
    // Check SHX shape (index 9) emitted vector strokes from demo_shapes.shx.
    assert!(
        list.items
            .iter()
            .any(|it| it.object == 9 && matches!(it.shape, Shape::Stroke { .. })),
        "SHX shape DIAMOND should emit vector strokes from demo_shapes.shx"
    );

    // 2. Render PNG.
    let png_path = dir.join("extra.png");
    let mut render_req = RenderRequest::new(&yaml_path);
    render_req.output = Some(png_path.clone());
    render_with(&render_req, &settings).unwrap();
    let png_bytes = fs::read(&png_path).unwrap();
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    // 3. Plot PDF.
    let pdf_path = dir.join("extra.pdf");
    let mut plot_req = PlotRequest::new(&yaml_path);
    plot_req.output = Some(pdf_path.clone());
    plot_with(&plot_req, &settings).unwrap();
    let pdf_bytes = fs::read(&pdf_path).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dwg_and_dxf_roundtrip_preserves_all_eight_objects_and_styles() {
    let dir = temp_dir("roundtrip");
    fs::write(dir.join("demo_shapes.shx"), make_demo_shx()).unwrap();
    let yaml_path = dir.join("extra.yaml");
    fs::write(&yaml_path, full_extra_sheet_yaml()).unwrap();
    let settings = settings_with_fonts(&dir);

    for (fname, format) in [
        ("extra.dwg", DrawingFormat::Dwg),
        ("extra.dxf", DrawingFormat::Dxf),
        ("extra.bin.dxf", DrawingFormat::DxfBinary),
    ] {
        let out_path = dir.join(fname);
        build_with_format(
            &BuildRequest {
                description: yaml_path.clone(),
                output: Some(out_path.clone()),
                overwrite: true,
            },
            Some(format),
            &settings,
        )
        .unwrap();

        let read_rep = read_original(&ReadRequest {
            source: out_path.clone(),
            mirror: None,
            no_mirror: true,
        })
        .unwrap();
        assert!(
            read_rep.decoded,
            "{fname}: expected full decode, not_decoded: {:?}",
            read_rep.not_decoded
        );
        assert!(
            read_rep.not_decoded.is_empty(),
            "{fname}: unexpected not_decoded: {:?}",
            read_rep.not_decoded
        );
        assert!(
            read_rep.not_read.is_empty(),
            "{fname}: unexpected not_read: {:?}",
            read_rep.not_read
        );

        // Query every object back from the built DWG/DXF!
        let q = query_with(
            &QueryRequest {
                source: out_path.clone(),
                sheet: None,
                query: Query::Find(Selection::default()),
            },
            &settings,
        )
        .unwrap();
        let Answer::Find { objects, .. } = q.answer else {
            panic!("expected Find answer");
        };

        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::Point(p)
                    if (p.position.x - 40.0).abs() < 1e-4 && (p.angle - 15.0).abs() < 1e-3
            )),
            "{fname}: missing Point"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::XLine(x) if (x.through.x - 100.0).abs() < 1e-4
            )),
            "{fname}: missing XLine"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::Ray(r) if (r.from.x - 50.0).abs() < 1e-4
            )),
            "{fname}: missing Ray"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::MLine(m) if m.points.len() == 3 && m.style.eq_ignore_ascii_case("WALL")
            )),
            "{fname}: missing MLine"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::Tolerance(t) if t.text.contains("0.05") && t.text.contains("0.02")
            )),
            "{fname}: missing Tolerance"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::CenterMark(cm)
                    if (cm.center.x - 210.0).abs() < 1e-4 && (cm.radius - 15.0).abs() < 1e-4
            )),
            "{fname}: missing CenterMark"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::CenterLine(cl)
                    if (cl.from.x - 140.0).abs() < 1e-4 && (cl.start_extension - 4.0).abs() < 1e-4
            )),
            "{fname}: missing CenterLine"
        );
        assert!(
            objects.iter().any(|o| matches!(
                &o.object,
                Object::Shape(s)
                    if s.file.eq_ignore_ascii_case("demo_shapes.shx")
                        && (s.position.x - 240.0).abs() < 1e-4
            )),
            "{fname}: missing Shape"
        );

        // Verify the round-tripped DWG/DXF also renders cleanly through the single pipeline!
        let dl = display_list_with(&DisplayListRequest::new(&out_path), &settings).unwrap();
        assert!(
            dl.display_list.items.len() >= 10,
            "{fname}: display list should draw all round-tripped objects"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn change_set_ops_snaps_queries_transforms_stretch_explode_and_undo() {
    let dir = temp_dir("changeset");
    fs::write(dir.join("demo_shapes.shx"), make_demo_shx()).unwrap();
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        "units: mm\npaper: ISO A4 landscape\nobjects:\n  - line: {id: base, from: [10, 10, 0], to: [100, 10, 0]}\n",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:\n  - sheet: S1\n    template: template.yaml\n",
    )
    .unwrap();
    let settings = settings_with_fonts(&dir);

    let orig_template = fs::read(dir.join("template.yaml")).unwrap();
    let orig_rows = fs::read(dir.join("rows.yaml")).unwrap();

    // Change set 1: add styles + all 8 objects via AutoCAD aliases.
    let cs1 = json!({
        "operations": [
            {"op": "DDPTYPE", "sheet": "S1", "pdmode": 35, "pdsize": 2.5},
            {"op": "MLSTYLE", "sheet": "S1", "name": "PIPE", "elements": [{"offset": 3}, {"offset": -3}], "start_square_cap": true, "end_square_cap": true},
            {"op": "PO", "sheet": "S1", "id": "p1", "position": [20, 30, 0]},
            {"op": "XL", "sheet": "S1", "id": "x1", "through": [50, 50, 0], "direction": [1, 0, 0]},
            {"op": "RAY", "sheet": "S1", "id": "r1", "from": [30, 40, 0], "direction": [0, 1, 0]},
            {"op": "ML", "sheet": "S1", "id": "m1", "points": [[20, 80, 0], [80, 80, 0], [80, 120, 0]], "style": "PIPE"},
            {"op": "TOL", "sheet": "S1", "id": "t1", "position": [120, 100, 0], "text": "{\\Fgdt;j}%%v0.1%%vA"},
            {"op": "CENTERMARK", "sheet": "S1", "id": "cm1", "center": [150, 60, 0], "radius": 12},
            {"op": "CENTERLINE", "sheet": "S1", "id": "cl1", "from": [120, 30, 0], "to": [170, 30, 0], "start_extension": 3, "end_extension": 3},
            {"op": "SHAPE", "sheet": "S1", "id": "sh1", "file": "demo_shapes.shx", "shape": "DIAMOND", "position": [200, 80, 0], "size": 2.0}
        ]
    });
    let reply1 = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs1,
            dry_run: false,
        },
        &settings,
    );
    assert!(reply1.ok, "cs1 failed: {reply1:?}");
    let entry1 = reply1
        .result
        .as_ref()
        .and_then(|r| r.journal_entry)
        .expect("journal entry 1");

    // Verify snaps on the new objects via query_with.
    let snap_pt = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: Some("S1".into()),
            query: serde_json::from_value(json!({
                "kind": "snap",
                "id": "S1/p1",
                "snap": "node"
            }))
            .unwrap(),
        },
        &settings,
    )
    .unwrap();
    match snap_pt.answer {
        Answer::Snap { ref points, .. } => {
            assert_eq!(points.len(), 1);
            assert!((points[0].at.x - 20.0).abs() < 1e-6);
            assert!((points[0].at.y - 30.0).abs() < 1e-6);
        }
        other => panic!("expected Snap, got {other:?}"),
    }

    let snap_cm = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: Some("S1".into()),
            query: serde_json::from_value(json!({
                "kind": "snap",
                "id": "S1/cm1",
                "snap": "center"
            }))
            .unwrap(),
        },
        &settings,
    )
    .unwrap();
    match snap_cm.answer {
        Answer::Snap { ref points, .. } => {
            assert_eq!(points.len(), 1);
            assert!((points[0].at.x - 150.0).abs() < 1e-6);
        }
        other => panic!("expected Snap, got {other:?}"),
    }

    // Change set 2: move, rotate, scale, mirror, stretch, and explode!
    let cs2 = json!({
        "operations": [
            {"op": "move", "target": {"id": "S1/p1"}, "by": [10, 5, 0]},
            {"op": "rotate", "target": {"id": "S1/x1"}, "base_point": [50, 50, 0], "angle": 45},
            {"op": "scale", "target": {"id": "S1/sh1"}, "base_point": [200, 80, 0], "factor": 2.0},
            {"op": "mirror", "target": {"id": "S1/r1"}, "first_point": [30, 0, 0], "second_point": [30, 100, 0], "erase_source": true},
            {"op": "stretch", "sheet": "S1", "box": [[160, 20], [180, 40]], "by": [15, 0, 0]},
            {"op": "explode", "target": {"id": "S1/m1"}, "id_prefix": "mlpart"},
            {"op": "explode", "target": {"id": "S1/cm1"}, "id_prefix": "cmpart"}
        ]
    });
    let reply2 = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs2,
            dry_run: false,
        },
        &settings,
    );
    assert!(reply2.ok, "cs2 failed: {reply2:?}");
    let res2 = reply2.result.as_ref().unwrap();
    assert!(
        !res2.operations[5].created.is_empty(),
        "mline explode should create lines"
    );
    assert!(
        !res2.operations[6].created.is_empty(),
        "center_mark explode should create lines"
    );

    // Verify query find sees the exploded lines and stretched center_line.
    let found = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: Some("S1".into()),
            query: Query::Find(Selection::default()),
        },
        &settings,
    )
    .unwrap();
    let Answer::Find { objects, .. } = found.answer else {
        panic!("expected Find");
    };
    assert!(objects.iter().any(|o| o.id.starts_with("S1/mlpart")));
    assert!(objects.iter().any(|o| o.id.starts_with("S1/cmpart")));
    assert!(objects.iter().any(|o| matches!(
        &o.object,
        Object::CenterLine(cl) if (cl.to.x - 185.0).abs() < 1e-4 && (cl.from.x - 120.0).abs() < 1e-4
    )));

    // Undo back to entry1 and verify byte-for-byte restoration!
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry1,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    assert_eq!(fs::read(dir.join("template.yaml")).unwrap(), orig_template);
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), orig_rows);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dwg_revision_adds_transforms_and_preserves_untouched() {
    let dir = temp_dir("orig-revise");
    fs::write(dir.join("demo_shapes.shx"), make_demo_shx()).unwrap();
    let base_yaml = dir.join("base.yaml");
    fs::write(
        &base_yaml,
        "units: mm\npaper: ISO A4 landscape\nobjects:\n  - line: {id: untouched_line, from: [10, 10, 0], to: [120, 10, 0]}\n",
    )
    .unwrap();
    let orig_dwg = dir.join("base.dwg");
    let settings = settings_with_fonts(&dir);
    build_with_format(
        &BuildRequest {
            description: base_yaml,
            output: Some(orig_dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap();

    // Revise original DWG: set PDMODE/PDSIZE, add WALL style, add all 8 objects, and transform!
    let rev_out = dir.join("base.rev1.dwg");
    let cs = json!({
        "operations": [
            {"op": "DDPTYPE", "pdmode": 66, "pdsize": 2.0},
            {"op": "MLSTYLE", "name": "WALL", "elements": [{"offset": 4}, {"offset": -4}], "start_square_cap": true, "end_square_cap": true},
            {"op": "POINT", "id": "p1", "position": [30, 40, 0]},
            {"op": "XLINE", "id": "x1", "through": [50, 50, 0], "direction": [1, 0, 0]},
            {"op": "RAY", "id": "r1", "from": [40, 60, 0], "direction": [1, 1, 0]},
            {"op": "MLINE", "id": "m1", "points": [[20, 100, 0], [90, 100, 0], [90, 140, 0]], "style": "WALL"},
            {"op": "TOLERANCE", "id": "t1", "position": [130, 120, 0], "text": "{\\Fgdt;j}%%v0.05%%vA"},
            {"op": "CENTERMARK", "id": "cm1", "center": [170, 70, 0], "radius": 10},
            {"op": "CENTERLINE", "id": "cl1", "from": [130, 40, 0], "to": [180, 40, 0], "start_extension": 3, "end_extension": 3},
            {"op": "SHAPE", "id": "sh1", "file": "demo_shapes.shx", "shape": "DIAMOND", "position": [220, 90, 0], "size": 2.5},
            {"op": "move", "target": {"id": "base/p1"}, "by": [5, 5, 0]},
            {"op": "rotate", "target": {"id": "base/x1"}, "base_point": [50, 50, 0], "angle": 30},
            {"op": "scale", "target": {"id": "base/m1"}, "base_point": [20, 100, 0], "factor": 1.5}
        ]
    });
    let rev_reply = revise_with(
        &ReviseRequest {
            set: orig_dwg,
            change_set: cs,
            dry_run: false,
            output: Some(rev_out.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev_reply.ok, "revision failed: {rev_reply:?}");
    let rev = rev_reply.result.unwrap();
    assert!(
        rev.unexpected_changes.is_empty(),
        "unexpected changes on original revision: {:?}",
        rev.unexpected_changes
    );
    assert!(
        rev.requested_failures.is_empty(),
        "requested failures on original revision: {:?}",
        rev.requested_failures
    );

    // Re-read the written revision DWG and verify every object and the untouched line survived.
    let re = read_original(&ReadRequest {
        source: rev_out.clone(),
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(re.decoded, "not_decoded: {:?}", re.not_decoded);
    assert!(re.not_read.is_empty(), "not_read: {:?}", re.not_read);

    let q = query_with(
        &QueryRequest {
            source: rev_out,
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &settings,
    )
    .unwrap();
    let Answer::Find { objects, .. } = q.answer else {
        panic!("expected Find");
    };
    assert!(objects.iter().any(|o| matches!(
        &o.object,
        Object::Point(p) if (p.position.x - 35.0).abs() < 1e-4 && (p.position.y - 45.0).abs() < 1e-4
    )));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::XLine(_))));
    assert!(objects.iter().any(|o| matches!(&o.object, Object::Ray(_))));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::MLine(_))));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::Tolerance(_))));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::CenterMark(_))));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::CenterLine(_))));
    assert!(objects
        .iter()
        .any(|o| matches!(&o.object, Object::Shape(_))));

    let _ = fs::remove_dir_all(&dir);
}
