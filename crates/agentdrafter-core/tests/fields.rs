//! Integration tests for AutoCAD fields (`FIELD`) and custom drawing
//! properties (`DWGPROPS`) across sheet descriptions, drawing sets, DWG
//! round-trip, rendering/plotting/display-list, `set_field` (`FIELD`), undo,
//! and original-DWG revisions (#87).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, build_with, change_with, display_list_with, plot_with, read_dwg, render_with,
    revise_with, undo, BuildRequest, ChangeRequest, DisplayListRequest, FieldKind, Object,
    PlotRequest, RenderRequest, ReviseRequest, SetBuildRequest, Settings, UndoRequest,
};
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-fields")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sheet_with_fields_in_text_mtext_and_attributes_builds_roundtrips_and_draws() {
    let dir = scratch("sheet-roundtrip");
    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  TITLE_BLK:
    base_point: [0, 0, 0]
    objects:
      - polyline:
          points: [[0, 0, 0], [80, 0, 0], [80, 20, 0], [0, 20, 0]]
          closed: true
      - attdef:
          tag: FILE
          position: [4, 12, 0]
          height: 3
          default: "?"
      - attdef:
          tag: PROJECT
          position: [4, 4, 0]
          height: 3
          default: "?"
"#,
    )
    .unwrap();
    let sheet_path = dir.join("LOOP-101.yaml");
    fs::write(
        &sheet_path,
        r#"units: mm
block_library: blocks.yaml
paper: ISO A4 landscape
layout:
  name: SH-01
custom_properties:
  Project: Demo Mill
  Area: Mixing
objects:
  - line:
      id: pipe-1
      from: [10, 10, 0]
      to: [40, 50, 0]
  - circle:
      id: tank-1
      center: [100, 50, 0]
      radius: 10
  - text:
      id: title-file
      field:
        kind: filename
        format: "%fn6"
      position: [10, 80, 0]
      height: 4
      space: paper
  - text:
      id: sheet-no
      field:
        kind: sheet_number
      position: [10, 70, 0]
      height: 4
      space: paper
  - text:
      id: pipe-len
      field:
        template: "PIPE LEN: {0} mm"
        kind: object_property
        object: pipe-1
        property: Length
        format: "%lu2%pr1"
      position: [10, 60, 0]
      height: 3.5
  - mtext:
      id: summary
      field:
        template: "PROJECT: {0}\nSAVED: {1}\nPLOTTED: {2}"
        items:
          - kind: custom_property
            property: Project
            format: "%tc1"
          - kind: save_date
            format: "yyyy-MM-dd"
            value: "2026-11-24"
          - kind: plot_date
            format: "yyyy-MM-dd"
      position: [10, 140, 0]
      height: 3
      width: 120
      width_factor: 0.9
      space: paper
  - insert:
      id: tb-1
      block: TITLE_BLK
      position: [180, 10, 0]
      space: paper
      attribute_fields:
        FILE:
          kind: filename
          format: "%fn2"
        PROJECT:
          kind: custom_property
          property: Area
"#,
    )
    .unwrap();

    let dwg_path = dir.join("LOOP-101.dwg");
    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &Settings::default(),
    )
    .unwrap();

    // Read the DWG back and verify custom_properties + every field round-trips.
    let contents = read_dwg(&dwg_path).unwrap();
    assert!(contents.decode.decoded, "{:#?}", contents.decode);
    assert_eq!(contents.objects_not_read, 0, "{:#?}", contents.not_read);
    assert_eq!(
        contents
            .custom_properties
            .get("Project")
            .map(String::as_str),
        Some("Demo Mill")
    );
    assert_eq!(
        contents.custom_properties.get("Area").map(String::as_str),
        Some("Mixing")
    );

    let find_obj = |local_id: &str| -> &Object {
        let full = format!("LOOP-101/{local_id}");
        let idx = contents
            .ids_and_names
            .iter()
            .position(|idn| idn.id.as_deref() == Some(full.as_str()))
            .unwrap_or_else(|| panic!("missing {full}"));
        &contents.objects[idx]
    };

    let Object::Text(t_file) = find_obj("title-file") else {
        panic!()
    };
    assert_eq!(t_file.contents, "LOOP-101.dwg");
    let f_file = t_file.field.as_ref().expect("filename field read back");
    assert_eq!(f_file.items.len(), 1);
    assert_eq!(f_file.items[0].kind, FieldKind::Filename);
    assert_eq!(f_file.items[0].format.as_deref(), Some("%fn6"));

    let Object::Text(t_sheet) = find_obj("sheet-no") else {
        panic!()
    };
    assert_eq!(t_sheet.contents, "SH-01");
    assert_eq!(
        t_sheet.field.as_ref().unwrap().items[0].kind,
        FieldKind::SheetNumber
    );

    let Object::Text(t_len) = find_obj("pipe-len") else {
        panic!()
    };
    assert_eq!(t_len.contents, "PIPE LEN: 50.0 mm");
    let f_len = t_len.field.as_ref().unwrap();
    assert_eq!(f_len.template, "PIPE LEN: {0} mm");
    assert_eq!(f_len.items[0].kind, FieldKind::ObjectProperty);
    assert_eq!(f_len.items[0].object.as_deref(), Some("pipe-1"));
    assert_eq!(f_len.items[0].property.as_deref(), Some("Length"));

    let Object::MText(mt) = find_obj("summary") else {
        panic!()
    };
    assert_eq!(
        mt.contents,
        "PROJECT: DEMO MILL\nSAVED: 2026-11-24\nPLOTTED: ----"
    );
    assert_eq!(mt.width_factor, Some(0.9));
    let f_mt = mt.field.as_ref().expect("mtext field read back");
    assert_eq!(f_mt.template, "PROJECT: {0}\nSAVED: {1}\nPLOTTED: {2}");
    assert_eq!(f_mt.items.len(), 3);
    assert_eq!(f_mt.items[0].kind, FieldKind::CustomProperty);
    assert_eq!(f_mt.items[0].property.as_deref(), Some("Project"));
    assert_eq!(f_mt.items[1].kind, FieldKind::SaveDate);
    assert_eq!(f_mt.items[2].kind, FieldKind::PlotDate);

    let Object::Insert(ins) = find_obj("tb-1") else {
        panic!()
    };
    assert_eq!(ins.attributes.len(), 2);
    assert_eq!(ins.attributes[0].tag, "FILE");
    assert_eq!(ins.attributes[0].value, "LOOP-101");
    assert_eq!(
        ins.attributes[0].field.as_ref().unwrap().items[0].kind,
        FieldKind::Filename
    );
    assert_eq!(ins.attributes[1].tag, "PROJECT");
    assert_eq!(ins.attributes[1].value, "Mixing");
    assert_eq!(
        ins.attributes[1].field.as_ref().unwrap().items[0].kind,
        FieldKind::CustomProperty
    );

    // Display list, PNG render, and PDF plot all draw the evaluated values cleanly.
    let dl_req: DisplayListRequest = serde_json::from_value(json!({ "source": dwg_path })).unwrap();
    let dl_report = display_list_with(&dl_req, &Settings::default()).unwrap();
    let drawn_text: Vec<String> = dl_report
        .display_list
        .text
        .iter()
        .map(|r| r.characters.iter().map(|(c, _)| *c).collect())
        .collect();
    assert!(
        drawn_text.iter().any(|s| s.contains("LOOP-101.dwg")),
        "drawn runs: {drawn_text:?}"
    );
    assert!(
        drawn_text.iter().any(|s| s.contains("DEMO MILL")),
        "drawn runs: {drawn_text:?}"
    );
    assert!(
        drawn_text.iter().any(|s| s.contains("Mixing")),
        "drawn runs: {drawn_text:?}"
    );

    let png_path = dir.join("LOOP-101.png");
    let render_req: RenderRequest = serde_json::from_value(json!({
        "source": dwg_path,
        "output": png_path,
        "width": 800,
        "overwrite": true,
    }))
    .unwrap();
    render_with(&render_req, &Settings::default()).unwrap();
    assert!(png_path.exists());

    let pdf_path = dir.join("LOOP-101.pdf");
    let plot_req: PlotRequest = serde_json::from_value(json!({
        "source": dwg_path,
        "output": pdf_path,
        "overwrite": true,
    }))
    .unwrap();
    plot_with(&plot_req, &Settings::default()).unwrap();
    assert!(pdf_path.exists());
}

#[test]
fn set_field_operation_on_drawing_set_and_byte_for_byte_undo() {
    let dir = scratch("set-change-undo");
    let set_dir = dir.join("set");
    fs::create_dir_all(&set_dir).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  TAG_BLK:
    base_point: [0, 0, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 5}
      - attdef: {tag: DWG, position: [0, 0, 0], height: 2.5, default: "NONE"}
"#,
    )
    .unwrap();
    fs::write(
        set_dir.join("template.yaml"),
        r#"units: mm
block_library: blocks.yaml
custom_properties:
  Plant: Demo Unit
objects:
  - line:
      id: rail
      from: [0, 0, 0]
      to: [100, 0, 0]
  - text:
      id: lbl
      contents: "STATIC"
      position: [10, 20, 0]
      height: 3
  - insert:
      id: badge
      block: TAG_BLK
      position: [50, 20, 0]
      attributes:
        DWG: "STATIC-DWG"
"#,
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        r#"rows:
  - sheet: SH-001
    template: template.yaml
"#,
    )
    .unwrap();

    let built = build_set_with(
        &SetBuildRequest {
            set: set_dir.clone(),
            overwrite: true,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    let template_before = fs::read(set_dir.join("template.yaml")).unwrap();

    // Attach fields via `FIELD` alias on both text and insert attribute.
    let changed = change_with(
        &ChangeRequest {
            set: set_dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "FIELD",
                        "target": { "id": "SH-001/lbl" },
                        "field": {
                            "template": "RAIL={0}",
                            "kind": "object_property",
                            "object": "rail",
                            "property": "Length",
                            "format": "%lu2%pr0"
                        }
                    },
                    {
                        "op": "set_field",
                        "target": { "id": "SH-001/badge" },
                        "tag": "DWG",
                        "field": {
                            "kind": "filename",
                            "format": "%fn6"
                        }
                    }
                ]
            }),
            dry_run: false,
        },
        &Settings::default(),
    );
    assert!(changed.ok, "{:#?}", changed.error);
    let entry_id = changed.result.as_ref().unwrap().journal_entry.unwrap();

    let dwg_after = read_dwg(&set_dir.join("dwg").join("SH-001.dwg")).unwrap();
    let lbl_idx = dwg_after
        .ids_and_names
        .iter()
        .position(|i| i.id.as_deref() == Some("SH-001/lbl"))
        .unwrap();
    let Object::Text(t) = &dwg_after.objects[lbl_idx] else {
        panic!()
    };
    assert_eq!(t.contents, "RAIL=100");
    assert_eq!(
        t.field.as_ref().unwrap().items[0].kind,
        FieldKind::ObjectProperty
    );

    let badge_idx = dwg_after
        .ids_and_names
        .iter()
        .position(|i| i.id.as_deref() == Some("SH-001/badge"))
        .unwrap();
    let Object::Insert(ins) = &dwg_after.objects[badge_idx] else {
        panic!()
    };
    assert_eq!(ins.attributes[0].value, "SH-001.dwg");
    assert_eq!(
        ins.attributes[0].field.as_ref().unwrap().items[0].kind,
        FieldKind::Filename
    );

    // Clear a field back to plain text.
    let cleared = change_with(
        &ChangeRequest {
            set: set_dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "set_field",
                        "target": { "id": "SH-001/lbl" },
                        "field": null,
                        "value": "PLAIN AGAIN"
                    }
                ]
            }),
            dry_run: false,
        },
        &Settings::default(),
    );
    assert!(cleared.ok, "{:#?}", cleared.error);
    let dwg_cleared = read_dwg(&set_dir.join("dwg").join("SH-001.dwg")).unwrap();
    let Object::Text(t2) = &dwg_cleared.objects[lbl_idx] else {
        panic!()
    };
    assert_eq!(t2.contents, "PLAIN AGAIN");
    assert!(t2.field.is_none());

    // Undo restores template byte for byte.
    let undone = undo(&UndoRequest {
        set: set_dir.clone(),
        change_set: entry_id,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(
        fs::read(set_dir.join("template.yaml")).unwrap(),
        template_before
    );
}

#[test]
fn original_dwg_keeps_untouched_fields_and_supports_set_field_revisions() {
    let dir = scratch("original-revise");
    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  HDR:
    base_point: [0, 0, 0]
    objects:
      - line: {from: [0, 0, 0], to: [30, 0, 0]}
      - attdef: {tag: REV, position: [5, 5, 0], height: 2.5, default: "0"}
"#,
    )
    .unwrap();
    let src_yaml = dir.join("PLANT-01.yaml");
    fs::write(
        &src_yaml,
        r#"units: mm
block_library: blocks.yaml
custom_properties:
  Site: North Plant
objects:
  - line:
      id: beam
      from: [0, 0, 0]
      to: [60, 80, 0]
  - text:
      id: untouched-field
      field:
        kind: custom_property
        property: Site
      position: [0, 100, 0]
      height: 3
  - text:
      id: plain-note
      contents: "OLD NOTE"
      position: [0, 110, 0]
      height: 3
  - insert:
      id: hdr-1
      block: HDR
      position: [50, 100, 0]
      attributes:
        REV: "A"
"#,
    )
    .unwrap();

    let original_dwg = dir.join("PLANT-01.dwg");
    build_with(
        &BuildRequest {
            description: src_yaml,
            output: Some(original_dwg.clone()),
            overwrite: true,
        },
        &Settings::default(),
    )
    .unwrap();

    // 1. Revise an unrelated object: untouched-field's FIELD records must come
    //    through 100% unchanged with zero unexpected changes.
    let rev1 = dir.join("PLANT-01.rev1.dwg");
    let r1 = revise_with(
        &ReviseRequest {
            set: original_dwg.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "move",
                        "target": { "id": "PLANT-01/plain-note" },
                        "by": [10, 0, 0]
                    }
                ]
            }),
            output: Some(rev1.clone()),
            dry_run: false,
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &Settings::default(),
    );
    assert!(r1.ok, "{:#?}", r1.error);
    let rep1 = r1.result.unwrap();
    assert!(
        rep1.unexpected_changes.is_empty(),
        "unexpected changes: {:#?}",
        rep1.unexpected_changes
    );
    let read_r1 = read_dwg(&rev1).unwrap();
    let uf_idx = read_r1
        .ids_and_names
        .iter()
        .position(|i| i.id.as_deref() == Some("PLANT-01/untouched-field"))
        .unwrap();
    let Object::Text(uf) = &read_r1.objects[uf_idx] else {
        panic!()
    };
    assert_eq!(uf.contents, "North Plant");
    assert_eq!(
        uf.field.as_ref().unwrap().items[0].kind,
        FieldKind::CustomProperty
    );

    // 2. Attach fields via `set_field` (`FIELD`) onto plain-note and hdr-1.REV,
    //    leaving untouched-field intact, with zero unexpected changes.
    let rev2 = dir.join("PLANT-01.rev2.dwg");
    let r2 = revise_with(
        &ReviseRequest {
            set: original_dwg,
            change_set: json!({
                "operations": [
                    {
                        "op": "FIELD",
                        "target": { "id": "PLANT-01/plain-note" },
                        "field": {
                            "template": "BEAM={0}",
                            "kind": "object_property",
                            "object": "beam",
                            "property": "Length",
                            "format": "%lu2%pr0"
                        }
                    },
                    {
                        "op": "set_field",
                        "target": { "id": "PLANT-01/hdr-1" },
                        "tag": "REV",
                        "field": {
                            "kind": "filename",
                            "format": "%fn2"
                        }
                    }
                ]
            }),
            output: Some(rev2.clone()),
            dry_run: false,
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &Settings::default(),
    );
    assert!(r2.ok, "{:#?}", r2.error);
    let rep2 = r2.result.unwrap();
    assert!(
        rep2.unexpected_changes.is_empty(),
        "unexpected changes: {:#?}",
        rep2.unexpected_changes
    );
    let read_r2 = read_dwg(&rev2).unwrap();
    let pn_idx = read_r2
        .ids_and_names
        .iter()
        .position(|i| i.id.as_deref() == Some("PLANT-01/plain-note"))
        .unwrap();
    let Object::Text(pn) = &read_r2.objects[pn_idx] else {
        panic!()
    };
    assert_eq!(pn.contents, "BEAM=100");
    assert_eq!(
        pn.field.as_ref().unwrap().items[0].kind,
        FieldKind::ObjectProperty
    );
    let hdr_idx = read_r2
        .ids_and_names
        .iter()
        .position(|i| i.id.as_deref() == Some("PLANT-01/hdr-1"))
        .unwrap();
    let Object::Insert(hdr) = &read_r2.objects[hdr_idx] else {
        panic!()
    };
    assert_eq!(hdr.attributes[0].value, "PLANT-01");
    assert_eq!(
        hdr.attributes[0].field.as_ref().unwrap().items[0].kind,
        FieldKind::Filename
    );
}
