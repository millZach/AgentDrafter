//! Integration tests for AutoCAD groups (`GROUP`, `UNGROUP`, `GROUPEDIT`, #92).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_with_format, change_with, display_list_with, plot_with, query_with, read_dwg, read_dxf,
    render_with, revise_with, undo, Answer, BuildRequest, ChangeRequest, DisplayListRequest,
    DrawingFormat, Matching, Object, PlotRequest, Query, QueryRequest, RenderRequest,
    ReviseRequest, Selection, Settings, UndoRequest,
};
use serde_json::json;

static NEXT: AtomicU64 = AtomicU64::new(1);

fn temp_dir(tag: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-group-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_fonts() -> Settings {
    Settings {
        font_folders: Vec::new(),
        system_font_folders: Vec::new(),
        plot_style_folders: Vec::new(),
    }
}

fn write_demo_set(dir: &Path) -> PathBuf {
    let set = dir.join("set");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/sheet.yaml"),
        "\
units: mm
paper: [200, 100]
layers:
  equip: {color: blue}
  wire: {color: green}
objects:
  - line: {from: [10, 10, 0], to: [190, 10, 0], id: bottom-rail, layer: wire}
  - line: {from: [10, 90, 0], to: [190, 90, 0], id: top-rail, layer: wire}
groups:
  RAILS:
    objects: [bottom-rail, top-rail]
    description: Power rails
",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "\
rows:
  - sheet: SH-01
    template: templates/sheet.yaml
    values: {}
    objects:
      - circle: {center: [50, 50, 0], radius: 10, id: ft-body, name: ft-symbol, layer: equip}
      - text: {contents: \"FT-101\", position: [50, 50, 0], height: 3, id: ft-tag, layer: equip}
      - line: {from: [60, 50, 0], to: [90, 50, 0], id: ft-lead, layer: wire}
      - polyline: {points: [[120, 40, 0], [140, 40, 0], [140, 60, 0]], closed: true, id: box-1, layer: equip}
    groups:
      FT-101:
        objects: [ft-body, ft-tag, ft-lead]
        description: Flow transmitter 101
        selectable: true
",
    )
    .unwrap();
    set
}

#[test]
fn sheet_groups_roundtrip_dwg_and_dxf_and_add_no_ink() {
    let dir = temp_dir("roundtrip");
    let with_groups = dir.join("with-groups.yaml");
    let without_groups = dir.join("without-groups.yaml");
    let body = "\
units: mm
paper: [120, 80]
layers:
  equip: {color: blue}
objects:
  - circle: {center: [30, 40, 0], radius: 8, id: c1, name: valve-body, layer: equip}
  - line: {from: [38, 40, 0], to: [70, 40, 0], id: l1, layer: equip}
  - text: {contents: \"FV-101\", position: [30, 52, 0], height: 3, id: t1, layer: equip}
";
    fs::write(&without_groups, body).unwrap();
    fs::write(
        &with_groups,
        format!(
            "{body}groups:\n  FV-101:\n    objects: [valve-body, l1, t1]\n    description: Control valve 101\n    selectable: true\n  SHARED:\n    objects: [c1, t1]\n    selectable: false\n"
        ),
    )
    .unwrap();

    // 1. DWG 2018, ASCII DXF, and binary DXF roundtrip.
    let dwg = dir.join("out.dwg");
    let dxf = dir.join("out.dxf");
    let dxf_bin = dir.join("out-bin.dxf");
    build_with_format(
        &BuildRequest {
            description: with_groups.clone(),
            output: Some(dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &no_fonts(),
    )
    .unwrap();
    build_with_format(
        &BuildRequest {
            description: with_groups.clone(),
            output: Some(dxf.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dxf),
        &no_fonts(),
    )
    .unwrap();
    build_with_format(
        &BuildRequest {
            description: with_groups.clone(),
            output: Some(dxf_bin.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::DxfBinary),
        &no_fonts(),
    )
    .unwrap();

    for (label, contents) in [
        ("dwg", read_dwg(&dwg).unwrap()),
        ("dxf", read_dxf(&dxf).unwrap()),
        ("dxf_binary", read_dxf(&dxf_bin).unwrap()),
    ] {
        assert_eq!(contents.groups.len(), 2, "{label}: expected 2 groups");
        let fv = contents
            .groups
            .iter()
            .find(|g| g.name == "FV-101")
            .expect("FV-101 present");
        assert_eq!(fv.description, "Control valve 101", "{label}");
        assert!(fv.selectable, "{label}");
        assert!(!fv.unnamed, "{label}");
        assert_eq!(fv.objects, vec![0, 1, 2], "{label}");

        let shared = contents
            .groups
            .iter()
            .find(|g| g.name == "SHARED")
            .expect("SHARED present");
        assert!(!shared.selectable, "{label}");
        assert_eq!(shared.objects, vec![0, 2], "{label}");
    }

    // 2. Display list exposes group metadata without adding any drawn items.
    let dl_with = display_list_with(&DisplayListRequest::new(&with_groups), &no_fonts()).unwrap();
    let dl_without =
        display_list_with(&DisplayListRequest::new(&without_groups), &no_fonts()).unwrap();
    assert_eq!(dl_with.display_list.items, dl_without.display_list.items);
    assert_eq!(dl_with.groups.len(), 2);
    assert_eq!(dl_with.groups[0].name, "FV-101");
    assert_eq!(dl_with.groups[0].objects.len(), 3);
    assert_eq!(dl_with.objects[0].groups, vec!["FV-101", "SHARED"]);
    assert_eq!(dl_with.objects[1].groups, vec!["FV-101"]);

    // 3. Rendered PNG and plotted PDF match ungrouped geometry.
    let png_a = dir.join("a.png");
    let png_b = dir.join("b.png");
    let mut r_a = RenderRequest::new(&with_groups);
    r_a.output = Some(png_a.clone());
    r_a.overwrite = true;
    r_a.width = Some(400);
    render_with(&r_a, &no_fonts()).unwrap();
    let mut r_b = RenderRequest::new(&without_groups);
    r_b.output = Some(png_b.clone());
    r_b.overwrite = true;
    r_b.width = Some(400);
    render_with(&r_b, &no_fonts()).unwrap();
    // Both decode to identical pixel buffers (PNG stamps differ by source filename).
    let img_a = image::open(&png_a).unwrap().to_rgba8();
    let img_b = image::open(&png_b).unwrap().to_rgba8();
    assert_eq!(img_a.as_raw(), img_b.as_raw());

    let pdf_a = dir.join("a.pdf");
    plot_with(
        &PlotRequest {
            source: with_groups,
            output: Some(pdf_a.clone()),
            overwrite: true,
            plot_style: None,
        },
        &no_fonts(),
    )
    .unwrap();
    assert!(pdf_a.exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn query_groups_and_group_filter_work_on_set_and_dwg() {
    let dir = temp_dir("query");
    let set = write_demo_set(&dir);

    let rep = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Groups,
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Groups { count, groups } = rep.answer else {
        panic!("expected Groups answer");
    };
    assert_eq!(count, 2);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].groups[0].name, "RAILS");
    assert_eq!(groups[0].groups[0].count, 2);
    assert_eq!(groups[0].groups[1].name, "FT-101");
    assert_eq!(groups[0].groups[1].count, 3);

    // Filter `query find` by group (case-insensitive).
    let find_rep = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Find(Selection {
                group: Some("ft-101".to_owned()),
                ..Selection::default()
            }),
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Find { count, objects } = find_rep.answer else {
        panic!("expected Find answer");
    };
    assert_eq!(count, 3);
    let ids: Vec<&str> = objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, vec!["SH-01/ft-body", "SH-01/ft-tag", "SH-01/ft-lead"]);

    // Substring match (`match: contains`).
    let contains_rep = query_with(
        &QueryRequest {
            source: set,
            sheet: None,
            query: Query::Find(Selection {
                group: Some("rail".to_owned()),
                matching: Matching::Contains,
                ..Selection::default()
            }),
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Find { count, .. } = contains_rep.answer else {
        panic!("expected Find answer");
    };
    assert_eq!(count, 2);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn change_set_group_operations_targeting_and_undo() {
    let dir = temp_dir("change-set");
    let set = write_demo_set(&dir);
    let original_rows = fs::read_to_string(set.join("rows.yaml")).unwrap();

    // 1. Move, change layer, copy, rotate, and add revision cloud around group `FT-101`.
    let cs1 = json!({
        "operations": [
            {"op": "MOVE", "target": {"group": "FT-101"}, "by": [10, 5, 0]},
            {"op": "change", "target": {"group": "FT-101"}, "set": {"layer": "wire"}},
            {"op": "ROTATE", "target": {"group": "FT-101"}, "base_point": [60, 55, 0], "angle": 90},
            {"op": "COPY", "target": {"group": "FT-101"}, "by": [40, 0, 0]},
            {"op": "REVCLOUD", "around": [{"group": "FT-101"}], "margin": 2, "id": "cloud-ft"}
        ]
    });
    let r1 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: cs1,
            dry_run: false,
        },
        &no_fonts(),
    );
    assert!(r1.ok, "{r1:?}");
    let entry1 = r1.result.unwrap().journal_entry.unwrap();

    // Verify FT-101's circle moved to [60, 55, 0] (and rotated about [60, 55, 0] stays there)
    // and its layer changed to `wire`.
    let found = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Find(Selection {
                id: Some("SH-01/ft-body".to_owned()),
                ..Selection::default()
            }),
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = found.answer else {
        panic!("expected Find");
    };
    let Object::Circle(c) = &objects[0].object else {
        panic!("expected circle");
    };
    assert!((c.center.x - 60.0).abs() < 1e-6);
    assert!((c.center.y - 55.0).abs() < 1e-6);
    assert_eq!(c.layer, "wire");

    // 2. Create a new group via `G` alias, edit it with `GROUPEDIT`, delete one member
    //    (verifying automatic pruning), then `UNGROUP` it.
    let cs2 = json!({
        "operations": [
            {
                "op": "G",
                "sheet": "SH-01",
                "name": "BOX-GRP",
                "targets": [{"id": "SH-01/box-1"}, {"id": "SH-01/ft-lead"}],
                "description": "Box and lead",
                "selectable": false
            },
            {
                "op": "GROUPEDIT",
                "sheet": "SH-01",
                "name": "BOX-GRP",
                "add": [{"id": "SH-01/ft-tag"}],
                "remove": [{"id": "SH-01/ft-lead"}],
                "rename": "BOX-RENAMED",
                "selectable": true
            }
        ]
    });
    let r2 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: cs2,
            dry_run: false,
        },
        &no_fonts(),
    );
    assert!(r2.ok, "{r2:?}");

    let g_rep = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Groups,
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Groups { groups, .. } = g_rep.answer else {
        panic!("expected Groups");
    };
    let renamed = groups[0]
        .groups
        .iter()
        .find(|g| g.name == "BOX-RENAMED")
        .expect("BOX-RENAMED exists");
    assert!(renamed.selectable);
    assert_eq!(renamed.count, 2);

    // 3. Delete one member of FT-101 -> pruned cleanly, set still valid; then delete whole group
    //    BOX-RENAMED -> its members are deleted cleanly.
    let cs3 = json!({
        "operations": [
            {"op": "ERASE", "target": {"id": "SH-01/ft-lead"}},
            {"op": "UNGROUP", "sheet": "SH-01", "name": "BOX-RENAMED"},
            {"op": "EXPLODE", "target": {"id": "SH-01/box-1"}}
        ]
    });
    let r3 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: cs3,
            dry_run: false,
        },
        &no_fonts(),
    );
    assert!(r3.ok, "{r3:?}");

    // 4. Undo back to entry1 restores `rows.yaml` byte-for-byte.
    let u = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry1,
        dry_run: false,
    });
    assert!(u.ok, "{u:?}");
    assert_eq!(
        fs::read_to_string(set.join("rows.yaml")).unwrap(),
        original_rows
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dwg_revisions_preserve_and_manage_groups() {
    let dir = temp_dir("revise-orig");
    let yaml = dir.join("orig.yaml");
    let dwg = dir.join("orig.dwg");
    fs::write(
        &yaml,
        "\
units: mm
paper: [200, 100]
layers:
  equip: {color: blue}
  wire: {color: green}
objects:
  - line: {from: [0, 0, 0], to: [50, 0, 0], id: untouched, layer: wire}
  - circle: {center: [50, 50, 0], radius: 8, id: c1, layer: equip}
  - line: {from: [58, 50, 0], to: [90, 50, 0], id: l1, layer: equip}
  - circle: {center: [120, 50, 0], radius: 6, id: c2, layer: equip}
groups:
  KEEP_GRP:
    objects: [untouched]
    description: Must stay untouched
  EDIT_GRP:
    objects: [c1, l1]
    description: Editable group
",
    )
    .unwrap();
    build_with_format(
        &BuildRequest {
            description: yaml,
            output: Some(dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &no_fonts(),
    )
    .unwrap();

    // 1. Move an unrelated object: KEEP_GRP and EDIT_GRP must stay intact with 0 unexpected changes.
    let rev1 = dir.join("rev1.dwg");
    let r1 = revise_with(
        &ReviseRequest {
            set: dwg.clone(),
            change_set: json!({
                "operations": [
                    {"op": "MOVE", "target": {"id": "orig/c2"}, "by": [5, 0, 0]}
                ]
            }),
            dry_run: false,
            output: Some(rev1.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &no_fonts(),
    );
    assert!(r1.ok, "{r1:?}");
    let rep1 = r1.result.unwrap();
    assert!(
        rep1.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        rep1.unexpected_changes
    );
    let c1 = read_dwg(&rev1).unwrap();
    assert_eq!(c1.groups.len(), 2);

    // 2. Revise using GROUP, MOVE by group, GROUPEDIT, ERASE of one member, and UNGROUP.
    let rev2 = dir.join("rev2.dwg");
    let r2 = revise_with(
        &ReviseRequest {
            set: dwg,
            change_set: json!({
                "operations": [
                    {"op": "MOVE", "target": {"group": "EDIT_GRP"}, "by": [10, 10, 0]},
                    {"op": "GROUP", "name": "NEW_GRP", "targets": [{"id": "orig/c2"}, {"id": "orig/l1"}], "description": "Created in revision", "selectable": false},
                    {"op": "GROUPEDIT", "name": "NEW_GRP", "add": [{"id": "orig/c1"}], "rename": "RENAMED_GRP", "selectable": true},
                    {"op": "ERASE", "target": {"id": "orig/l1"}},
                    {"op": "UNGROUP", "name": "EDIT_GRP"}
                ]
            }),
            dry_run: false,
            output: Some(rev2.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &no_fonts(),
    );
    assert!(r2.ok, "{r2:?}");
    let rep2 = r2.result.unwrap();
    assert!(
        rep2.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        rep2.unexpected_changes
    );

    let q2 = query_with(
        &QueryRequest {
            source: rev2,
            sheet: None,
            query: Query::Groups,
        },
        &no_fonts(),
    )
    .unwrap();
    let Answer::Groups { count, groups } = q2.answer else {
        panic!("expected Groups");
    };
    assert_eq!(count, 2);
    let names: Vec<&str> = groups[0].groups.iter().map(|g| g.name.as_str()).collect();
    assert!(names.contains(&"KEEP_GRP"), "{names:?}");
    assert!(names.contains(&"RENAMED_GRP"), "{names:?}");
    let renamed = groups[0]
        .groups
        .iter()
        .find(|g| g.name == "RENAMED_GRP")
        .unwrap();
    assert!(renamed.selectable);
    // l1 was deleted and pruned, leaving c2 and c1 (2 members).
    assert_eq!(renamed.count, 2);

    let _ = fs::remove_dir_all(&dir);
}
