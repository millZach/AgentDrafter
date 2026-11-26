//! External references (`XREF`, `XATTACH`, `XBIND`, #90):
//! building, reading, drawing, stamping, revising and binding attached and
//! overlaid DWG/DXF external references on sheet descriptions, drawing sets
//! and originals.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with_format, change_with, check_stale, check_with, display_list_with, find_command,
    operations, plot_with, query_with, read_dwg, read_original, render_with, revise_with,
    sheet_description_schema, undo, Answer, BuildRequest, ChangeRequest, CheckRequest,
    DisplayListRequest, DrawingFormat, Interface, Object, PlotRequest, Query, QueryRequest,
    ReadRequest, RenderRequest, ReviseRequest, Selection, Settings, StaleRequest, UndoRequest,
    WarningCode, XrefKind,
};
use serde_json::{json, Value};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-xref-{}", std::process::id()))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_sys_fonts() -> Settings {
    Settings::default().without_system_fonts()
}

fn build_file(yaml_path: &Path, out_path: &Path, format: DrawingFormat) {
    let req = BuildRequest {
        description: yaml_path.to_path_buf(),
        output: Some(out_path.to_path_buf()),
        overwrite: true,
    };
    build_with_format(&req, Some(format), &no_sys_fonts())
        .unwrap_or_else(|e| panic!("build {} failed: {}", yaml_path.display(), e.message));
}

fn revise_req(dwg: &Path, ops: Value) -> ReviseRequest {
    ReviseRequest {
        set: dwg.to_path_buf(),
        change_set: json!({ "operations": ops }),
        dry_run: false,
        allow_loss: false,
        output: None,
        no_pdf: true,
        plot_style: None,
        expected_original_sha256: None,
    }
}

fn change_req(set: &Path, ops: Value) -> ChangeRequest {
    ChangeRequest {
        set: set.to_path_buf(),
        change_set: json!({ "operations": ops }),
        dry_run: false,
    }
}

fn make_grid_dwg(dir: &Path, file_name: &str, format: DrawingFormat, text: &str) -> PathBuf {
    let yaml = dir.join(format!("{file_name}.yaml"));
    let out = dir.join(file_name);
    let body = format!(
        r#"units: mm
layers:
  AXES: {{color: 8}}
objects:
  - line: {{id: axis-h, from: [0, 0, 0], to: [100, 0, 0], layer: AXES}}
  - line: {{id: axis-v, from: [0, 0, 0], to: [0, 80, 0], layer: AXES}}
  - text: {{id: label, contents: "{text}", position: [10, 10, 0], height: 5, layer: AXES}}
"#
    );
    fs::write(&yaml, body).unwrap();
    build_file(&yaml, &out, format);
    out
}

#[test]
fn build_and_roundtrip_attach_and_overlay_xrefs_in_dwg_and_dxf() {
    let dir = scratch("roundtrip");
    let _grid = make_grid_dwg(&dir, "grid.dwg", DrawingFormat::Dwg, "GRID-A1");
    let _stamp = make_grid_dwg(&dir, "overlay.dxf", DrawingFormat::Dxf, "OVL-NOTE");

    let host_yaml = dir.join("host.yaml");
    fs::write(
        &host_yaml,
        r#"units: mm
xrefs:
  GRID: {path: grid.dwg}
  NOTE: {path: overlay.dxf, kind: overlay}
  OLD_REF: {path: grid.dwg, unloaded: true}
objects:
  - line: {id: own, from: [0, 0, 0], to: [10, 0, 0]}
  - insert: {id: grid-ins, block: GRID, position: [20, 10, 0]}
  - insert: {id: note-ins, block: NOTE, position: [0, 0, 0]}
  - insert: {id: old-ins, block: OLD_REF, position: [50, 50, 0]}
"#,
    )
    .unwrap();

    for (ext, format) in [
        ("dwg", DrawingFormat::Dwg),
        ("dxf", DrawingFormat::Dxf),
        ("bin.dxf", DrawingFormat::DxfBinary),
    ] {
        let out = dir.join(format!("host.{ext}"));
        build_file(&host_yaml, &out, format);
        let contents = read_dwg(&out).unwrap();
        assert_eq!(contents.blocks_not_read, 0, "{ext}: blocks_not_read");
        assert_eq!(contents.objects_not_read, 0, "{ext}: objects_not_read");
        assert!(
            contents.blocks.is_empty(),
            "{ext}: xrefs must not embed into local blocks"
        );
        assert_eq!(contents.xrefs.len(), 3, "{ext}: 3 xrefs");
        let grid = contents.xrefs.iter().find(|x| x.name == "GRID").unwrap();
        assert_eq!(grid.path, "grid.dwg");
        assert_eq!(grid.kind, XrefKind::Attach);
        assert!(!grid.unloaded);
        let note = contents.xrefs.iter().find(|x| x.name == "NOTE").unwrap();
        assert_eq!(note.path, "overlay.dxf");
        assert_eq!(note.kind, XrefKind::Overlay);
        assert!(!note.unloaded);
        let old = contents.xrefs.iter().find(|x| x.name == "OLD_REF").unwrap();
        assert_eq!(old.path, "grid.dwg");
        // DWG preserves the unloaded flag directly; DXF has no unloaded flag on BLOCK.
        if format == DrawingFormat::Dwg {
            assert!(old.unloaded, "DWG preserves unloaded flag");
        }
        assert_eq!(contents.objects.len(), 4, "{ext}: line + 3 xref inserts");
        assert!(matches!(&contents.objects[1], Object::Insert(ins) if ins.block == "GRID"));
    }

    // `read` reports xrefs, zero unresolved xrefs, and writes `xrefs:` into the text mirror.
    let host_dwg = dir.join("host.dwg");
    let read_rep = read_original(&ReadRequest {
        source: host_dwg.clone(),
        mirror: None,
        no_mirror: false,
    })
    .unwrap();
    assert_eq!(read_rep.xrefs.len(), 3);
    assert!(read_rep.unresolved_xrefs.is_empty());
    let mirror = fs::read_to_string(read_rep.mirror.unwrap()).unwrap();
    assert!(
        mirror.contains("xrefs:\n"),
        "mirror should list xrefs:\n{mirror}"
    );
    assert!(mirror.contains("GRID: {path: grid.dwg}"));
    assert!(mirror.contains("kind: overlay"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolved_xrefs_draw_in_render_plot_display_list_query_check_and_stamp_stale() {
    let dir = scratch("draw_and_stale");
    let grid_dwg = make_grid_dwg(&dir, "grid.dwg", DrawingFormat::Dwg, "PLANT-GRID");

    let host_yaml = dir.join("sheet.yaml");
    fs::write(
        &host_yaml,
        r#"units: mm
paper: [200, 150]
xrefs:
  GRID: {path: grid.dwg}
objects:
  - polyline: {id: border, points: [[0, 0, 0], [200, 0, 0], [200, 150, 0], [0, 150, 0]], closed: true}
  - insert: {id: grid-ref, block: GRID, position: [25, 15, 0]}
"#,
    )
    .unwrap();
    let host_dwg = dir.join("sheet.dwg");
    build_file(&host_yaml, &host_dwg, DrawingFormat::Dwg);

    // 1. Display list draws xref geometry (border + 2 lines + 1 text inside xref).
    let dl = display_list_with(&DisplayListRequest::new(&host_dwg), &no_sys_fonts()).unwrap();
    assert!(dl.unresolved_xrefs.is_empty());
    assert!(
        dl.display_list.items.iter().any(|item| item.object == 1),
        "display list must include shapes drawn for xref insert at index 1"
    );

    // 2. Render PNG and stamp includes `grid.dwg` as `xref`.
    let png_path = dir.join("sheet.png");
    let mut rreq = RenderRequest::new(&host_dwg);
    rreq.output = Some(png_path.clone());
    rreq.width = Some(200);
    rreq.height = Some(150);
    rreq.overwrite = true;
    let rrep = render_with(&rreq, &no_sys_fonts()).unwrap();
    assert!(rrep.unresolved_xrefs.is_empty());
    assert!(rrep
        .stamp
        .sources
        .iter()
        .any(|s| s.path.ends_with("grid.dwg")));

    // 3. Plot PDF includes searchable text from xref and stamps `grid.dwg`.
    let pdf_path = dir.join("sheet.pdf");
    let mut preq = PlotRequest::new(&host_dwg);
    preq.output = Some(pdf_path.clone());
    preq.overwrite = true;
    let prep = plot_with(&preq, &no_sys_fonts()).unwrap();
    assert!(prep.unresolved_xrefs.is_empty());
    assert!(
        prep.text_lines >= 1,
        "PDF should carry searchable text from xref"
    );
    assert!(prep
        .stamp
        .sources
        .iter()
        .any(|s| s.path.ends_with("grid.dwg")));

    // 4. Query finds text inside xref and reports extents including xref.
    let q_find = query_with(
        &QueryRequest {
            source: host_dwg.clone(),
            sheet: None,
            query: Query::Find(Selection {
                text: Some("PLANT-GRID".to_owned()),
                ..Default::default()
            }),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    assert!(q_find.unresolved_xrefs.is_empty());
    let Answer::Find { count, .. } = q_find.answer else {
        panic!("expected Find answer");
    };
    assert_eq!(
        count, 1,
        "query find should locate text inside resolved xref"
    );

    // 5. Check passes cleanly with resolved xref.
    let check_rep = check_with(
        &CheckRequest {
            source: host_yaml.clone(),
            sheets: Vec::new(),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    assert!(
        check_rep.passed,
        "check should pass: {:?}",
        check_rep.findings
    );
    assert!(check_rep.unresolved_xrefs.is_empty());

    // 6. Stale check: fresh now, stale after editing the referenced `grid.dwg`!
    let fresh = check_stale(&StaleRequest {
        output: pdf_path.clone(),
        source: None,
    })
    .unwrap();
    assert!(!fresh.stale);

    let _updated = make_grid_dwg(&dir, "grid.dwg", DrawingFormat::Dwg, "PLANT-GRID-REV2");
    assert!(grid_dwg.is_file());
    let stale = check_stale(&StaleRequest {
        output: pdf_path,
        source: None,
    })
    .unwrap();
    assert!(
        stale.stale,
        "modifying referenced xref must mark output stale"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nested_attach_propagates_while_nested_overlay_is_skipped_and_units_scale() {
    let dir = scratch("nested_and_units");

    // Child attached DWG (in inches: 2 in line -> 50.8 mm in mm host!).
    let attached_yaml = dir.join("child_in.yaml");
    let attached_dwg = dir.join("child_in.dwg");
    fs::write(
        &attached_yaml,
        r#"units: in
objects:
  - line: {id: l, from: [0, 0, 0], to: [2, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&attached_yaml, &attached_dwg, DrawingFormat::Dwg);

    // Child overlaid DWG (in mm: far away at x=500 so it would change extents if mistakenly loaded!).
    let overlaid_yaml = dir.join("child_ovl.yaml");
    let overlaid_dwg = dir.join("child_ovl.dwg");
    fs::write(
        &overlaid_yaml,
        r#"units: mm
objects:
  - line: {id: l, from: [500, 0, 0], to: [600, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&overlaid_yaml, &overlaid_dwg, DrawingFormat::Dwg);

    // Middle DWG (in mm) attaches `child_in.dwg` and overlays `child_ovl.dwg`.
    let mid_yaml = dir.join("mid.yaml");
    let mid_dwg = dir.join("mid.dwg");
    fs::write(
        &mid_yaml,
        r#"units: mm
xrefs:
  CHILD_ATT: {path: child_in.dwg, kind: attach}
  CHILD_OVL: {path: child_ovl.dwg, kind: overlay}
objects:
  - line: {id: m, from: [0, 0, 0], to: [10, 0, 0]}
  - insert: {id: att, block: CHILD_ATT, position: [0, 0, 0]}
  - insert: {id: ovl, block: CHILD_OVL, position: [0, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&mid_yaml, &mid_dwg, DrawingFormat::Dwg);

    // Host DWG (in mm) attaches `mid.dwg`.
    let host_yaml = dir.join("host.yaml");
    let host_dwg = dir.join("host.dwg");
    fs::write(
        &host_yaml,
        r#"units: mm
xrefs:
  MID: {path: mid.dwg}
objects:
  - insert: {id: mid-ins, block: MID, position: [0, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&host_yaml, &host_dwg, DrawingFormat::Dwg);

    // In `mid.dwg`, both CHILD_ATT and CHILD_OVL draw (max_x = 600).
    let mid_dl = display_list_with(&DisplayListRequest::new(&mid_dwg), &no_sys_fonts()).unwrap();
    let mid_ext = mid_dl.extents.unwrap();
    assert!(
        (mid_ext.max[0] - 600.0).abs() < 1e-3,
        "mid extents: {mid_ext:?}"
    );

    // In `host.dwg`, nested CHILD_ATT draws (scaled 2 in -> 50.8 mm) and nested CHILD_OVL is skipped!
    let host_dl = display_list_with(&DisplayListRequest::new(&host_dwg), &no_sys_fonts()).unwrap();
    assert!(host_dl.unresolved_xrefs.is_empty());
    let host_ext = host_dl.extents.unwrap();
    assert!(
        (host_ext.max[0] - 50.8).abs() < 1e-3,
        "expected nested inch attach (50.8 mm) and skipped overlay, got {host_ext:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_and_circular_xrefs_report_unresolved_and_top_level_warnings() {
    let dir = scratch("missing_and_cycle");

    // 1. Missing xref
    let missing_yaml = dir.join("missing_host.yaml");
    let missing_dwg = dir.join("missing_host.dwg");
    fs::write(
        &missing_yaml,
        r#"units: mm
xrefs:
  GONE: {path: does_not_exist.dwg}
objects:
  - line: {id: l, from: [0, 0, 0], to: [10, 0, 0]}
  - insert: {id: g, block: GONE, position: [0, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&missing_yaml, &missing_dwg, DrawingFormat::Dwg);

    let read_reply = find_command("read").unwrap().run(
        json!({"source": missing_dwg.display().to_string(), "no_mirror": true}),
        &no_sys_fonts(),
        Interface::Cli,
    );
    assert!(read_reply.ok);
    assert_eq!(read_reply.warnings.len(), 1);
    assert_eq!(read_reply.warnings[0].code, WarningCode::UnresolvedXref);
    assert_eq!(read_reply.warnings[0].xref.as_deref(), Some("GONE"));

    let dl_reply = find_command("display-list").unwrap().run(
        json!({"source": missing_dwg.display().to_string()}),
        &no_sys_fonts(),
        Interface::Cli,
    );
    assert!(dl_reply.ok);
    assert!(dl_reply
        .warnings
        .iter()
        .any(|w| w.code == WarningCode::UnresolvedXref && w.xref.as_deref() == Some("GONE")));

    // 2. Circular xref: A -> B -> A
    let a_yaml = dir.join("a.yaml");
    let b_yaml = dir.join("b.yaml");
    let a_dwg = dir.join("a.dwg");
    let b_dwg = dir.join("b.dwg");
    fs::write(
        &a_yaml,
        r#"units: mm
xrefs:
  B: {path: b.dwg}
objects:
  - line: {id: la, from: [0, 0, 0], to: [10, 0, 0]}
  - insert: {id: ib, block: B, position: [0, 0, 0]}
"#,
    )
    .unwrap();
    fs::write(
        &b_yaml,
        r#"units: mm
xrefs:
  A: {path: a.dwg}
objects:
  - line: {id: lb, from: [0, 10, 0], to: [20, 10, 0]}
  - insert: {id: ia, block: A, position: [0, 0, 0]}
"#,
    )
    .unwrap();
    build_file(&a_yaml, &a_dwg, DrawingFormat::Dwg);
    build_file(&b_yaml, &b_dwg, DrawingFormat::Dwg);

    let dl = display_list_with(&DisplayListRequest::new(&a_dwg), &no_sys_fonts()).unwrap();
    assert_eq!(dl.unresolved_xrefs.len(), 1);
    assert_eq!(dl.unresolved_xrefs[0].name, "A");
    assert!(
        dl.unresolved_xrefs[0].reason.contains("circular"),
        "expected circular reason, got {:?}",
        dl.unresolved_xrefs[0]
    );
    // A's line and B's line still both drew cleanly without infinite recursion!
    assert_eq!(dl.extents.unwrap().max[0], 20.0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dwg_revision_preserves_xrefs_and_supports_attach_detach_and_xbind() {
    let dir = scratch("revise_xref");
    let _grid = make_grid_dwg(&dir, "grid.dwg", DrawingFormat::Dwg, "REV-GRID");

    // 1. Original DWG with an existing attached xref; revise an unrelated line -> xref stays untouched!
    let orig_yaml = dir.join("orig.yaml");
    let orig_dwg = dir.join("orig.dwg");
    fs::write(
        &orig_yaml,
        r#"units: mm
xrefs:
  GRID: {path: grid.dwg}
objects:
  - line: {id: l1, from: [0, 0, 0], to: [20, 0, 0]}
  - insert: {id: g1, block: GRID, position: [10, 5, 0]}
"#,
    )
    .unwrap();
    build_file(&orig_yaml, &orig_dwg, DrawingFormat::Dwg);

    let move_rep = revise_with(
        &revise_req(
            &orig_dwg,
            json!([{
                "op": "move",
                "target": {"id": "orig/l1"},
                "by": [0, 5, 0],
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(move_rep.ok, "move revision failed: {move_rep:?}");
    let rev1 = PathBuf::from(
        move_rep
            .result
            .as_ref()
            .unwrap()
            .revision_dwg
            .as_ref()
            .unwrap(),
    );
    assert!(move_rep
        .result
        .as_ref()
        .unwrap()
        .unexpected_changes
        .is_empty());
    let rev1_contents = read_dwg(&rev1).unwrap();
    assert_eq!(rev1_contents.xrefs.len(), 1);
    assert_eq!(rev1_contents.xrefs[0].name, "GRID");
    assert_eq!(rev1_contents.xrefs[0].path, "grid.dwg");

    // 2. XBIND (`bind_xref` via AutoCAD alias `XBIND`) converts GRID into a local block!
    let bind_rep = revise_with(
        &revise_req(
            &orig_dwg,
            json!([{
                "op": "XBIND",
                "name": "GRID",
                "mode": "bind",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(bind_rep.ok, "XBIND failed: {bind_rep:?}");
    let rev2 = PathBuf::from(
        bind_rep
            .result
            .as_ref()
            .unwrap()
            .revision_dwg
            .as_ref()
            .unwrap(),
    );
    assert!(bind_rep
        .result
        .as_ref()
        .unwrap()
        .unexpected_changes
        .is_empty());
    let rev2_contents = read_dwg(&rev2).unwrap();
    assert!(
        rev2_contents.xrefs.is_empty(),
        "bound xref should no longer be an xref"
    );
    assert!(
        rev2_contents
            .blocks
            .iter()
            .any(|b| b.name == "GRID" && b.objects.len() == 3),
        "GRID must now be a local block holding the 3 grid objects: {:?}",
        rev2_contents.blocks
    );
    assert!(
        rev2_contents.layers.iter().any(|l| l.name == "GRID$0$AXES"),
        "bind mode prefixes dependent layers with GRID$0$: {:?}",
        rev2_contents.layers
    );

    // 3. Attach a fresh overlay xref via `XATTACH` and then detach it via `detach_xref`.
    let attach_rep = revise_with(
        &revise_req(
            &orig_dwg,
            json!([{
                "op": "XATTACH",
                "name": "OVL",
                "path": "grid.dwg",
                "kind": "overlay",
                "position": [5, 5, 0],
                "id": "ovl-1",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(attach_rep.ok, "XATTACH failed: {attach_rep:?}");
    let rev3 = PathBuf::from(
        attach_rep
            .result
            .as_ref()
            .unwrap()
            .revision_dwg
            .as_ref()
            .unwrap(),
    );
    let rev3_contents = read_dwg(&rev3).unwrap();
    assert!(rev3_contents
        .xrefs
        .iter()
        .any(|x| x.name == "OVL" && x.kind == XrefKind::Overlay));

    let detach_rep = revise_with(
        &revise_req(
            &orig_dwg,
            json!([{
                "op": "detach_xref",
                "name": "OVL",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(detach_rep.ok, "detach_xref failed: {detach_rep:?}");
    let rev4 = PathBuf::from(
        detach_rep
            .result
            .as_ref()
            .unwrap()
            .revision_dwg
            .as_ref()
            .unwrap(),
    );
    let rev4_contents = read_dwg(&rev4).unwrap();
    assert!(rev4_contents.xrefs.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn drawing_set_attach_xbind_detach_and_undo_work_atomically() {
    let dir = scratch("set_xref");
    let _grid = make_grid_dwg(&dir, "grid.dwg", DrawingFormat::Dwg, "SET-GRID");

    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  DOT:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        r#"units: mm
block_library: blocks.yaml
paper: [200, 150]
objects:
  - polyline: {id: border, points: [[0, 0, 0], [200, 0, 0], [200, 150, 0], [0, 150, 0]], closed: true}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        r#"rows:
  - sheet: S-001
    template: template.yaml
"#,
    )
    .unwrap();

    // 1. Attach xref via `XATTACH`
    let attach = change_with(
        &change_req(
            &dir,
            json!([{
                "op": "XATTACH",
                "template": "template.yaml",
                "name": "GRID",
                "path": "grid.dwg",
                "position": [10, 10, 0],
                "id": "grid-ins",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(attach.ok, "set XATTACH failed: {attach:?}");
    let built = read_dwg(&dir.join("dwg").join("S-001.dwg")).unwrap();
    assert_eq!(built.xrefs.len(), 1);
    assert_eq!(built.xrefs[0].name, "GRID");

    // 2. Bind xref via `XBIND` into `blocks.yaml`
    let bind = change_with(
        &change_req(
            &dir,
            json!([{
                "op": "XBIND",
                "template": "template.yaml",
                "name": "GRID",
                "mode": "bind",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(bind.ok, "set XBIND failed: {bind:?}");
    let bind_entry = bind.result.as_ref().unwrap().journal_entry.unwrap();
    let bound_dwg = read_dwg(&dir.join("dwg").join("S-001.dwg")).unwrap();
    assert!(bound_dwg.xrefs.is_empty());
    assert!(bound_dwg.blocks.iter().any(|b| b.name == "GRID"));
    assert!(bound_dwg.layers.iter().any(|l| l.name == "GRID$0$AXES"));

    // 3. Undo the `XBIND` restores the xref cleanly!
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: bind_entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "undo failed: {undone:?}");
    assert!(!undone.result.unwrap().files.is_empty());
    let restored = read_dwg(&dir.join("dwg").join("S-001.dwg")).unwrap();
    assert_eq!(restored.xrefs.len(), 1);
    assert_eq!(restored.xrefs[0].name, "GRID");

    // 4. Detach the xref
    let detach = change_with(
        &change_req(
            &dir,
            json!([{
                "op": "detach_xref",
                "template": "template.yaml",
                "name": "GRID",
            }]),
        ),
        &no_sys_fonts(),
    );
    assert!(detach.ok, "set detach_xref failed: {detach:?}");
    let detached = read_dwg(&dir.join("dwg").join("S-001.dwg")).unwrap();
    assert!(detached.xrefs.is_empty());
    assert_eq!(detached.objects.len(), 1, "only border remains");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn operations_and_schema_advertise_xrefs() {
    let ops = operations();
    let has = |name: &str, alias: &str| {
        ops.operations.iter().any(|o| {
            o.name == name
                && (o.alias.eq_ignore_ascii_case(alias)
                    || o.other_aliases
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(alias)))
        })
    };
    assert!(has("attach_xref", "XATTACH"));
    assert!(has("attach_xref", "XREF"));
    assert!(has("detach_xref", "XDETACH"));
    assert!(has("bind_xref", "XBIND"));

    let schema: Value = sheet_description_schema();
    assert!(schema["properties"].get("xrefs").is_some());
    assert!(schema["$defs"].get("xref").is_some());
}
