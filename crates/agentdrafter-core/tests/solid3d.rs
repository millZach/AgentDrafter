//! End-to-end tests for Rank 42 (#96): native AutoCAD ACIS 3D solids
//! (`solid3d` / `3DSOLID`, ADR-0012), viewport `view_direction`, 2D wireframe
//! projection through the single drawing pipeline, geometric queries (`volume`,
//! `surface_area`, 3D `extents`, snaps), drawing-set operations (`add_solid3d`,
//! `add_box`, `add_cylinder`, `extrude`, `revolve`, `union`, `subtract`,
//! `intersect`, transforms, `explode`, `undo`), and original-DWG revisions.

use std::f64::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_with_format, change_with, display_list_with, operations, plot_with, query_with,
    read_original, render_with, revise_with, undo, Answer, BuildRequest, ChangeRequest,
    DisplayListRequest, DrawingFormat, Object, PlotRequest, Query, QueryRequest, ReadRequest,
    RenderRequest, ReviseRequest, Selection, Settings, Shape, SnapKind, UndoRequest,
};
use serde_json::json;

static NEXT: AtomicU64 = AtomicU64::new(1);

fn temp_dir(tag: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-solid3d-{tag}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_sys_fonts() -> Settings {
    Settings {
        font_folders: Vec::new(),
        system_font_folders: Vec::new(),
        ..Settings::default()
    }
}

fn full_solids_sheet_yaml() -> &'static str {
    r#"units: mm
paper: ISO A3 landscape
layers:
  SOLIDS: {color: 7}
  CUTS: {color: 3}
  BORDER: {color: 7}
layout:
  name: Views
objects:
  - solid3d:
      id: s_box
      kind: box
      origin: [0, 0, 0]
      size: [60, 40, 30]
      layer: SOLIDS
  - solid3d:
      id: s_cyl
      kind: cylinder
      base: [100, 20, 0]
      radius: 15
      height: 50
      layer: SOLIDS
  - solid3d:
      id: s_cone
      kind: cone
      base: [160, 20, 0]
      radius: 18
      height: 45
      layer: SOLIDS
  - solid3d:
      id: s_sphere
      kind: sphere
      center: [220, 20, 20]
      radius: 20
      layer: SOLIDS
  - solid3d:
      id: s_torus
      kind: torus
      center: [290, 20, 12]
      major_radius: 22
      minor_radius: 7
      layer: SOLIDS
  - solid3d:
      id: s_wedge
      kind: wedge
      origin: [0, 80, 0]
      size: [50, 30, 35]
      layer: SOLIDS
  - solid3d:
      id: s_pyramid
      kind: pyramid
      base: [100, 95, 0]
      base_size: 36
      height: 40
      layer: SOLIDS
  - solid3d:
      id: s_extrude_l
      kind: extrude
      profile:
        - [150, 70, 0]
        - [200, 70, 0]
        - [200, 85, 0]
        - [168, 85, 0]
        - [168, 120, 0]
        - [150, 120, 0]
      height: 25
      layer: SOLIDS
  - solid3d:
      id: s_extrude_taper
      kind: extrude
      profile:
        - [220, 70, 0]
        - [260, 70, 0]
        - [260, 110, 0]
        - [220, 110, 0]
      height: 30
      taper_angle: 12
      layer: SOLIDS
  - solid3d:
      id: s_revolve_full
      kind: revolve
      profile:
        - [12, 170, 0]
        - [26, 170, 0]
        - [26, 170, 18]
        - [12, 170, 18]
      axis_origin: [0, 170, 0]
      axis_direction: [0, 0, 1]
      angle: 360
      layer: SOLIDS
  - solid3d:
      id: s_revolve_half
      kind: revolve
      profile:
        - [80, 170, 0]
        - [95, 170, 0]
        - [95, 170, 20]
        - [80, 170, 20]
      axis_origin: [70, 170, 0]
      axis_direction: [0, 0, 1]
      angle: 180
      layer: SOLIDS
  - solid3d:
      id: s_union
      kind: union
      operands:
        - kind: box
          origin: [130, 150, 0]
          size: [40, 25, 25]
        - kind: box
          origin: [155, 160, 0]
          size: [30, 30, 25]
      layer: CUTS
  - solid3d:
      id: s_subtract
      kind: subtract
      operands:
        - kind: box
          origin: [210, 150, 0]
          size: [60, 50, 30]
        - kind: cylinder
          base: [240, 175, -5]
          radius: 12
          height: 40
      layer: CUTS
  - solid3d:
      id: s_intersect
      kind: intersect
      operands:
        - kind: box
          origin: [290, 150, 0]
          size: [40, 40, 40]
        - kind: box
          origin: [310, 170, 10]
          size: [40, 40, 40]
      layer: CUTS
  - solid3d:
      id: s_mesh
      kind: mesh
      vertices:
        - [0, 220, 0]
        - [30, 220, 0]
        - [15, 246, 0]
        - [15, 229, 28]
      faces:
        - [0, 2, 1]
        - [0, 1, 3]
        - [1, 2, 3]
        - [2, 0, 3]
      layer: SOLIDS
  - viewport:
      id: vp_top
      space: Views
      center: [105, 220, 0]
      width: 180
      height: 120
      view_center: [160, 120, 0]
      view_direction: top
      scale: 0.45
      layer: BORDER
  - viewport:
      id: vp_front
      space: Views
      center: [310, 220, 0]
      width: 180
      height: 120
      view_center: [160, 120, 20]
      view_direction: front
      scale: 0.45
      layer: BORDER
  - viewport:
      id: vp_iso
      space: Views
      center: [210, 80, 0]
      width: 360
      height: 120
      view_center: [160, 120, 20]
      view_direction: se_isometric
      scale: 0.45
      layer: BORDER
"#
}

#[test]
fn all_primitives_sweeps_booleans_and_mesh_roundtrip_dwg_and_dxf() {
    let dir = temp_dir("roundtrip");
    let yaml_path = dir.join("solids.yaml");
    fs::write(&yaml_path, full_solids_sheet_yaml()).unwrap();
    let settings = no_sys_fonts();

    let dwg_path = dir.join("solids.dwg");
    let dxf_path = dir.join("solids.dxf");
    let dxf_bin_path = dir.join("solids-bin.dxf");

    for (out, fmt) in [
        (&dwg_path, DrawingFormat::Dwg),
        (&dxf_path, DrawingFormat::Dxf),
        (&dxf_bin_path, DrawingFormat::DxfBinary),
    ] {
        build_with_format(
            &BuildRequest {
                description: yaml_path.clone(),
                output: Some(out.clone()),
                overwrite: true,
            },
            Some(fmt),
            &settings,
        )
        .unwrap_or_else(|e| panic!("build {fmt:?} failed: {e}"));

        let read = read_original(&ReadRequest {
            source: out.clone(),
            mirror: None,
            no_mirror: true,
        })
        .unwrap_or_else(|e| panic!("read_original {fmt:?} failed: {e}"));

        assert!(
            read.decoded,
            "{fmt:?} decode incomplete: {:?}",
            read.not_decoded
        );
        assert!(
            read.not_decoded.is_empty(),
            "{fmt:?} not_decoded: {:?}",
            read.not_decoded
        );
        assert!(
            read.not_read.is_empty(),
            "{fmt:?} not_read: {:?}",
            read.not_read
        );

        let q = query_with(
            &QueryRequest {
                source: out.clone(),
                sheet: None,
                query: Query::Find(Selection::default()),
            },
            &settings,
        )
        .unwrap();
        let Answer::Find { objects, .. } = q.answer else {
            panic!("expected Find answer");
        };
        let solid_count = objects
            .iter()
            .filter(|o| matches!(&o.object, Object::Solid3D(_)))
            .count();
        assert_eq!(
            solid_count, 15,
            "{fmt:?}: expected 15 solid3d objects round-tripped, got {solid_count}"
        );
    }

    // Single drawing pipeline: display-list, PNG render, and PDF plot on the built DWG.
    let mut dl_req = DisplayListRequest::new(&dwg_path);
    dl_req.layout = Some("Views".to_owned());
    let dl = display_list_with(&dl_req, &settings).expect("display_list on solids.dwg");
    let stroke_items = dl
        .display_list
        .items
        .iter()
        .filter(|it| matches!(it.shape, Shape::Stroke { .. }))
        .count();
    assert!(
        stroke_items > 40,
        "expected rich 2D wireframe strokes across 3 viewports, got {stroke_items}"
    );

    let png_path = dir.join("solids.png");
    let mut render_req = RenderRequest::new(&dwg_path);
    render_req.output = Some(png_path.clone());
    render_with(&render_req, &settings).expect("render solids.dwg");
    let png_bytes = fs::read(&png_path).unwrap();
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let pdf_path = dir.join("solids.pdf");
    let mut plot_req = PlotRequest::new(&dwg_path);
    plot_req.output = Some(pdf_path.clone());
    plot_with(&plot_req, &settings).expect("plot solids.dwg");
    let pdf_bytes = fs::read(&pdf_path).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn query_geometry_volume_surface_area_extents_and_snaps() {
    let dir = temp_dir("query");
    let yaml_path = dir.join("metrics.yaml");
    fs::write(
        &yaml_path,
        r#"units: mm
paper: ISO A3 landscape
objects:
  - solid3d:
      id: box1
      kind: box
      origin: [10, 20, 5]
      size: [100, 80, 40]
  - solid3d:
      id: cyl1
      kind: cylinder
      base: [200, 60, 0]
      radius: 15
      height: 40
  - solid3d:
      id: sphere1
      kind: sphere
      center: [300, 60, 25]
      radius: 25
  - solid3d:
      id: enclosure
      kind: subtract
      operands:
        - kind: box
          origin: [0, 150, 0]
          size: [100, 80, 40]
        - kind: cylinder
          base: [50, 190, -10]
          radius: 15
          height: 60
"#,
    )
    .unwrap();
    let settings = no_sys_fonts();

    let dwg_path = dir.join("metrics.dwg");
    build_with_format(
        &BuildRequest {
            description: yaml_path,
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap();

    let res = query_with(
        &QueryRequest {
            source: dwg_path.clone(),
            sheet: None,
            query: serde_json::from_value(json!({ "kind": "geometry" })).unwrap(),
        },
        &settings,
    )
    .unwrap();

    let Answer::Geometry { objects, .. } = res.answer else {
        panic!("expected Geometry answer");
    };
    let by_id = |suffix: &str| {
        objects
            .iter()
            .find(|g| g.object.id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing geometry for {suffix}"))
    };

    // Box 100 x 80 x 40 => V = 320_000, A = 2*(8000+4000+3200) = 30_400
    let b = by_id("/box1");
    assert!((b.measures.volume.unwrap() - 320_000.0).abs() < 1e-3);
    assert!((b.measures.surface_area.unwrap() - 30_400.0).abs() < 1e-3);
    let be = b.measures.extents.as_ref().unwrap();
    assert!((be.min[0] - 10.0).abs() < 1e-6 && (be.max[2] - 45.0).abs() < 1e-6);

    // Cylinder r=15, h=40 => V = pi*225*40
    let c = by_id("/cyl1");
    let cyl_v = PI * 15.0 * 15.0 * 40.0;
    assert!((c.measures.volume.unwrap() - cyl_v).abs() < 1e-2);

    // Sphere r=25 => V = 4/3 * pi * 25^3
    let s = by_id("/sphere1");
    let sph_v = (4.0 / 3.0) * PI * 25.0_f64.powi(3);
    assert!((s.measures.volume.unwrap() - sph_v).abs() < 1e-2);

    // CSG subtract: box 100x80x40 minus cylinder r=15 through h=40
    let enc = by_id("/enclosure");
    let expected_enc = 320_000.0 - PI * 15.0 * 15.0 * 40.0;
    let rel_err = (enc.measures.volume.unwrap() - expected_enc).abs() / expected_enc;
    assert!(
        rel_err < 0.015,
        "subtracted enclosure volume {} vs expected {} (rel err {:.4})",
        enc.measures.volume.unwrap(),
        expected_enc,
        rel_err
    );

    // Object snaps on box1: endpoint (8 corners) and center (1 centroid)
    let snap_res = query_with(
        &QueryRequest {
            source: dwg_path,
            sheet: None,
            query: serde_json::from_value(json!({
                "kind": "snap",
                "id": "metrics/box1"
            }))
            .unwrap(),
        },
        &settings,
    )
    .unwrap();
    let Answer::Snap { points, .. } = snap_res.answer else {
        panic!("expected Snap answer");
    };
    let endpoint_count = points
        .iter()
        .filter(|s| s.snap == SnapKind::Endpoint)
        .count();
    let center = points
        .iter()
        .find(|s| s.snap == SnapKind::Center)
        .expect("center snap on box");
    assert_eq!(endpoint_count, 8);
    assert!((center.at.x - 60.0).abs() < 1e-6);
    assert!((center.at.y - 60.0).abs() < 1e-6);
    assert!((center.at.z - 25.0).abs() < 1e-6);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn drawing_set_solid3d_operations_and_byte_exact_undo() {
    let ops = operations();
    for expected in [
        "add_solid3d",
        "add_box",
        "add_cylinder",
        "extrude",
        "revolve",
        "union",
        "subtract",
        "intersect",
    ] {
        assert!(
            ops.operations.iter().any(|o| o.name == expected),
            "missing operation `{expected}` from registry"
        );
    }

    let dir = temp_dir("set");
    let settings = no_sys_fonts();
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        r#"units: mm
paper: ISO A3 landscape
layers:
  SOLIDS: {color: 7}
objects:
  - line:
      id: ref_line
      from: [0, 0, 0]
      to: [400, 0, 0]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        r#"rows:
  - sheet: S-01
    template: template.yaml
    objects:
      - polyline:
          id: prof_rect
          points: [[0, 0, 0], [50, 0, 0], [50, 40, 0], [0, 40, 0]]
          closed: true
          layer: SOLIDS
      - circle:
          id: prof_circ
          center: [120, 20, 0]
          radius: 15
          layer: SOLIDS
      - polyline:
          id: prof_rev
          points: [[200, 0, 0], [215, 0, 0], [215, 0, 25], [200, 0, 25]]
          closed: true
          layer: SOLIDS
"#,
    )
    .unwrap();

    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    let change_set = json!({
        "operations": [
            {
                "op": "BOX",
                "sheet": "S-01",
                "id": "b1",
                "origin": [0, 80, 0],
                "length": 60,
                "width": 50,
                "height": 30,
                "layer": "SOLIDS"
            },
            {
                "op": "CYLINDER",
                "sheet": "S-01",
                "id": "c_hole",
                "base": [30, 105, -5],
                "radius": 10,
                "height": 40,
                "layer": "SOLIDS"
            },
            {
                "op": "SUBTRACT",
                "target": {"id": "S-01/b1"},
                "subtract": {"id": "S-01/c_hole"}
            },
            {
                "op": "EXTRUDE",
                "target": {"id": "S-01/prof_rect"},
                "height": 20,
                "id": "ext_box"
            },
            {
                "op": "EXT",
                "target": {"id": "S-01/prof_circ"},
                "height": 35,
                "id": "ext_cyl"
            },
            {
                "op": "REVOLVE",
                "target": {"id": "S-01/prof_rev"},
                "axis_origin": [190, 0, 0],
                "axis_direction": [0, 0, 1],
                "angle": 180,
                "id": "rev_part"
            },
            {
                "op": "add_solid3d",
                "sheet": "S-01",
                "id": "u1",
                "kind": "box",
                "origin": [260, 80, 0],
                "size": [30, 30, 30]
            },
            {
                "op": "BOX",
                "sheet": "S-01",
                "id": "u2",
                "origin": [275, 95, 0],
                "size": [30, 30, 30]
            },
            {
                "op": "UNION",
                "targets": [{"id": "S-01/u1"}, {"id": "S-01/u2"}]
            },
            {
                "op": "BOX",
                "sheet": "S-01",
                "id": "i1",
                "origin": [330, 80, 0],
                "size": [30, 30, 30]
            },
            {
                "op": "BOX",
                "sheet": "S-01",
                "id": "i2",
                "origin": [345, 95, 10],
                "size": [30, 30, 30]
            },
            {
                "op": "INTERSECT",
                "targets": [{"id": "S-01/i1"}, {"id": "S-01/i2"}]
            },
            {
                "op": "MOVE",
                "target": {"id": "S-01/ext_box"},
                "by": [5, 0, 0]
            },
            {
                "op": "ROTATE",
                "target": {"id": "S-01/ext_box"},
                "base_point": [5, 0, 0],
                "angle": 90
            },
            {
                "op": "SCALE",
                "target": {"id": "S-01/ext_cyl"},
                "base_point": [120, 20, 0],
                "factor": 1.2
            },
            {
                "op": "EXPLODE",
                "target": {"id": "S-01/i1"}
            }
        ]
    });

    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set,
            dry_run: false,
        },
        &settings,
    );
    assert!(reply.ok, "change set failed: {reply:?}");
    assert!(dir.join("out/S-01.dwg").exists());

    // Confirm built DWG decodes cleanly with zero unread or not_read.
    let read = read_original(&ReadRequest {
        source: dir.join("out/S-01.dwg"),
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(read.decoded);
    assert!(read.not_read.is_empty());

    // Undo restores rows.yaml byte-for-byte.
    let cs_id = reply
        .result
        .as_ref()
        .unwrap()
        .journal_entry
        .expect("journal change set id");
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: cs_id,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "undo failed: {undone:?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dwg_revisions_with_3d_solids() {
    let dir = temp_dir("revise");
    let settings = no_sys_fonts();
    let yaml_path = dir.join("orig.yaml");
    fs::write(
        &yaml_path,
        r#"units: mm
paper: ISO A3 landscape
layers:
  SOLIDS: {color: 7}
  KEEP: {color: 2}
objects:
  - line:
      id: untouched_line
      from: [0, 0, 0]
      to: [100, 0, 0]
      layer: KEEP
  - polyline:
      id: plate_2d
      points: [[0, 20, 0], [80, 20, 0], [80, 70, 0], [0, 70, 0]]
      closed: true
      layer: SOLIDS
  - circle:
      id: boss_2d
      center: [150, 45, 0]
      radius: 12
      layer: SOLIDS
"#,
    )
    .unwrap();
    let orig_dwg = dir.join("orig.dwg");
    build_with_format(
        &BuildRequest {
            description: yaml_path,
            output: Some(orig_dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap();
    let orig_bytes = fs::read(&orig_dwg).unwrap();

    let rev_out = dir.join("orig.rev1.dwg");
    let cs = json!({
        "operations": [
            {
                "op": "EXTRUDE",
                "target": {"id": "orig/plate_2d"},
                "height": 25,
                "id": "plate_3d"
            },
            {
                "op": "CYLINDER",
                "id": "hole_tool",
                "base": [40, 45, -5],
                "radius": 10,
                "height": 35,
                "layer": "SOLIDS"
            },
            {
                "op": "SUBTRACT",
                "target": {"id": "orig/plate_3d"},
                "subtract": {"id": "orig/hole_tool"}
            },
            {
                "op": "EXTRUDE",
                "target": {"id": "orig/boss_2d"},
                "height": 30,
                "id": "boss_3d"
            },
            {
                "op": "BOX",
                "id": "added_box",
                "origin": [200, 20, 0],
                "size": [40, 30, 20],
                "layer": "SOLIDS"
            },
            {
                "op": "MOVE",
                "target": {"id": "orig/added_box"},
                "by": [10, 5, 0]
            }
        ]
    });

    let rev_reply = revise_with(
        &ReviseRequest {
            set: orig_dwg.clone(),
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
    assert_eq!(
        rev.revision.as_ref().unwrap().dwg_version,
        rev.original.dwg_version
    );
    assert!(
        rev.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        rev.unexpected_changes
    );
    assert!(
        rev.requested_failures.is_empty(),
        "requested failures: {:?}",
        rev.requested_failures
    );
    assert_eq!(fs::read(&orig_dwg).unwrap(), orig_bytes);

    let read = read_original(&ReadRequest {
        source: rev_out,
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(read.decoded);
    assert!(read.not_read.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_solids_and_empty_boolean_rejected() {
    let dir = temp_dir("invalid");
    let settings = no_sys_fonts();

    let disjoint = dir.join("disjoint.yaml");
    fs::write(
        &disjoint,
        r#"units: mm
paper: ISO A4 landscape
objects:
  - solid3d:
      id: bad_int
      kind: intersect
      operands:
        - kind: box
          origin: [0, 0, 0]
          size: [10, 10, 10]
        - kind: box
          origin: [100, 100, 100]
          size: [10, 10, 10]
"#,
    )
    .unwrap();
    let err = build_with_format(
        &BuildRequest {
            description: disjoint,
            output: Some(dir.join("disjoint.dwg")),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .expect_err("disjoint intersect must fail");
    assert!(
        err.message.to_lowercase().contains("empty"),
        "unexpected error message: {}",
        err.message
    );

    let _ = fs::remove_dir_all(&dir);
}
