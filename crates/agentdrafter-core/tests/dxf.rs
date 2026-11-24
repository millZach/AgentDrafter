//! DXF read and write (#89): building sheets and drawing sets as ASCII or
//! binary DXF with the same build mark and object IDs, reading DXF everywhere
//! a DWG is read (`read`, `render`, `plot`, `display-list`, `query`, `check`,
//! `import-blocks`, `stale`), and revising original DXF drawings with journal,
//! undo and accept.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    accept_with, build, build_set_with, build_set_with_format, build_with_format, change_with,
    check_stale, check_with, display_list_with, import_blocks, journal, plot_with, query_with,
    read_dwg, read_dxf, read_original, render_with, revise_with, undo, AcceptRequest, BuildRequest,
    ChangeRequest, CheckRequest, DisplayListRequest, DrawingFormat, ErrorCode, ImportBlocksRequest,
    Interface, JournalRequest, LeftoverKind, PlotRequest, Query, QueryRequest, ReadRequest,
    RenderRequest, ReviseRequest, Selection, SetBuildRequest, Settings, SourceKind, StaleRequest,
    UndoRequest, Unit,
};
use serde_json::json;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dxf")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default().without_system_fonts()
}

fn write_rich_sheet(dir: &Path) -> PathBuf {
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm
blocks:
  TAG_BOX:
    objects:
      - polyline:
          points: [[0, 0, 0], [30, 0, 0], [30, 12, 0], [0, 12, 0]]
          closed: true
      - attdef:
          tag: TAG
          default: 'FT-100'
          position: [3, 4, 0]
          height: 3
",
    )
    .unwrap();
    let sheet = dir.join("panel.yaml");
    fs::write(
        &sheet,
        "units: mm
paper: ISO A4 landscape
block_library: blocks.yaml
custom_properties:
  JOB: 'DEMO-42'
linetypes:
  DASHED: {pattern: [6, -3]}
layers:
  wire: {color: 1, lineweight: 0.35}
  dashed: {color: 3, linetype: DASHED}
  notes: {color: 7}
objects:
  - polyline:
      id: border
      name: sheet-border
      points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]]
      closed: true
  - line:
      id: wire-1
      name: main-bus
      layer: wire
      from: [20, 40, 0]
      to: [140, 40, 0]
  - circle:
      id: dot-1
      layer: dashed
      center: [80, 100, 0]
      radius: 15
  - arc:
      id: arc-1
      center: [160, 100, 0]
      radius: 20
      start_angle: 0
      end_angle: 180
  - text:
      id: title
      name: heading
      layer: notes
      contents: 'PANEL LAYOUT'
      position: [20, 185, 0]
      height: 6
  - insert:
      id: dev-1
      name: transmitter
      block: TAG_BOX
      position: [40, 60, 0]
      attributes:
        TAG: 'PT-201'
",
    )
    .unwrap();
    sheet
}

#[test]
fn single_sheet_builds_to_ascii_and_binary_dxf_and_reads_back_with_ids_and_build_mark() {
    let dir = scratch("single-sheet");
    let sheet = write_rich_sheet(&dir);
    let settings = no_system_fonts();

    // 1. Build DWG baseline.
    let dwg_report = build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: None,
            overwrite: false,
        },
        None,
        &settings,
    )
    .unwrap();
    assert_eq!(dwg_report.format, DrawingFormat::Dwg);
    assert_eq!(dwg_report.dxf, None);
    let dwg_path = PathBuf::from(&dwg_report.dwg);
    let dwg_read = read_dwg(&dwg_path).unwrap();

    // 2. Auto-detect ASCII DXF from `.dxf` output extension.
    let auto_dxf = dir.join("auto.dxf");
    let auto_report = build(&BuildRequest {
        description: sheet.clone(),
        output: Some(auto_dxf.clone()),
        overwrite: false,
    })
    .unwrap();
    assert_eq!(auto_report.format, DrawingFormat::Dxf);
    assert_eq!(auto_report.dxf.as_deref(), Some(auto_dxf.to_str().unwrap()));
    assert_eq!(auto_report.dwg_version, "AC1032");
    let ascii_text = fs::read_to_string(&auto_dxf).expect("ASCII DXF is valid UTF-8 text");
    assert!(ascii_text.contains("$ACADVER"));
    assert!(ascii_text.contains("AC1032"));
    assert!(ascii_text.contains("$CUSTOMPROPERTYTAG"));
    assert!(ascii_text.contains("AgentDrafter"));

    // 3. Explicit `--format dxf` writes beside the sheet with `.dxf` extension.
    let explicit_dxf_report = build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: None,
            overwrite: false,
        },
        Some(DrawingFormat::Dxf),
        &settings,
    )
    .unwrap();
    let explicit_dxf = dir.join("panel.dxf");
    assert_eq!(
        explicit_dxf_report.dxf.as_deref(),
        Some(explicit_dxf.to_str().unwrap())
    );
    assert!(explicit_dxf.is_file());

    // 4. Explicit `--format dxf_binary` writes binary DXF starting with sentinel.
    let bin_dxf = dir.join("panel-bin.dxf");
    let bin_report = build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: Some(bin_dxf.clone()),
            overwrite: false,
        },
        Some(DrawingFormat::DxfBinary),
        &settings,
    )
    .unwrap();
    assert_eq!(bin_report.format, DrawingFormat::DxfBinary);
    let bin_bytes = fs::read(&bin_dxf).unwrap();
    assert!(bin_bytes.starts_with(b"AutoCAD Binary DXF"));

    // 5. Both ASCII and binary DXF read back with identical units, objects, custom properties, and carried IDs.
    for path in [&auto_dxf, &explicit_dxf, &bin_dxf] {
        let dxf_read = read_dxf(path).unwrap();
        assert_eq!(dxf_read.dwg_version, "AC1032", "{path:?}");
        assert_eq!(dxf_read.units, Some(Unit::Mm), "{path:?}");
        assert_eq!(dxf_read.objects_not_read, 0, "{path:?}");
        assert_eq!(dxf_read.objects.len(), dwg_read.objects.len(), "{path:?}");
        assert_eq!(dxf_read.ids_and_names, dwg_read.ids_and_names, "{path:?}");
        assert_eq!(
            dxf_read.custom_properties, dwg_read.custom_properties,
            "{path:?}"
        );
        assert_eq!(
            dxf_read.custom_properties.get("JOB").map(String::as_str),
            Some("DEMO-42")
        );
    }

    // 6. Overwrite replaces AgentDrafter-built DXF, but refuses an unmarked original DXF.
    build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: Some(auto_dxf.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dxf),
        &settings,
    )
    .unwrap();

    let foreign_dxf = dir.join("foreign.dxf");
    let stripped = ascii_text.replace("AgentDrafter", "OtherApp");
    fs::write(&foreign_dxf, &stripped).unwrap();
    let err = build(&BuildRequest {
        description: sheet.clone(),
        output: Some(foreign_dxf.clone()),
        overwrite: true,
    })
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotABuild);
    assert!(err.message.contains("DXF"), "{err:?}");
    assert_eq!(fs::read_to_string(&foreign_dxf).unwrap(), stripped);

    // 7. Mismatched format and output extension are refused.
    let mismatch_1 = build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: Some(dir.join("bad.dxf")),
            overwrite: false,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap_err();
    assert_eq!(mismatch_1.code, ErrorCode::Usage);

    let mismatch_2 = build_with_format(
        &BuildRequest {
            description: sheet,
            output: Some(dir.join("bad.dwg")),
            overwrite: false,
        },
        Some(DrawingFormat::Dxf),
        &settings,
    )
    .unwrap_err();
    assert_eq!(mismatch_2.code, ErrorCode::Usage);
}

#[test]
fn read_render_plot_display_list_query_check_and_import_work_on_ascii_and_binary_dxf() {
    let dir = scratch("all-readers");
    let sheet = write_rich_sheet(&dir);
    let settings = no_system_fonts();

    let dwg_path = dir.join("panel.dwg");
    let ascii_dxf = dir.join("panel-ascii.dxf");
    let bin_dxf = dir.join("panel-bin.dxf");

    build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: Some(dwg_path.clone()),
            overwrite: false,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap();
    build_with_format(
        &BuildRequest {
            description: sheet.clone(),
            output: Some(ascii_dxf.clone()),
            overwrite: false,
        },
        Some(DrawingFormat::Dxf),
        &settings,
    )
    .unwrap();
    build_with_format(
        &BuildRequest {
            description: sheet,
            output: Some(bin_dxf.clone()),
            overwrite: false,
        },
        Some(DrawingFormat::DxfBinary),
        &settings,
    )
    .unwrap();

    // Baseline DWG render, plot, display-list, query, check.
    let dwg_dl = display_list_with(&DisplayListRequest::new(&dwg_path), &settings).unwrap();
    let dwg_png = render_with(
        &RenderRequest {
            width: Some(400),
            height: Some(300),
            ..RenderRequest::new(&dwg_path)
        },
        &settings,
    )
    .unwrap();
    let dwg_pdf = plot_with(&PlotRequest::new(&dwg_path), &settings).unwrap();
    let dwg_query = query_with(
        &QueryRequest {
            source: dwg_path.clone(),
            sheet: None,
            query: Query::Find(Selection {
                id: Some("panel/wire-1".into()),
                ..Selection::default()
            }),
        },
        &settings,
    )
    .unwrap();

    for (dxf_path, expected_format) in [
        (&ascii_dxf, DrawingFormat::Dxf),
        (&bin_dxf, DrawingFormat::DxfBinary),
    ] {
        // `read`
        let read_rep = read_original(&ReadRequest {
            source: dxf_path.clone(),
            mirror: None,
            no_mirror: false,
        })
        .unwrap();
        assert!(read_rep.decoded, "{read_rep:?}");
        assert_eq!(read_rep.format, Some(expected_format));
        assert_eq!(read_rep.dwg_version, "AC1032");
        assert_eq!(read_rep.units, Some(Unit::Mm));
        assert!(read_rep.not_read.is_empty());
        let mirror_path = PathBuf::from(read_rep.mirror.as_ref().unwrap());
        assert!(mirror_path.is_file());

        // Refusing `.dxf` as mirror output path.
        let mirror_err = read_original(&ReadRequest {
            source: dxf_path.clone(),
            mirror: Some(dir.join("bad-mirror.dxf")),
            no_mirror: false,
        })
        .unwrap_err();
        assert_eq!(mirror_err.code, ErrorCode::Usage);

        // `display-list`
        let dxf_dl = display_list_with(&DisplayListRequest::new(dxf_path), &settings).unwrap();
        assert_eq!(
            dxf_dl.display_list.items.len(),
            dwg_dl.display_list.items.len()
        );
        assert_eq!(dxf_dl.objects.len(), dwg_dl.objects.len());
        assert_eq!(dxf_dl.objects_not_drawn, 0);

        // `render`
        let dxf_png = render_with(
            &RenderRequest {
                width: Some(400),
                height: Some(300),
                ..RenderRequest::new(dxf_path)
            },
            &settings,
        )
        .unwrap();
        assert_eq!(dxf_png.width, dwg_png.width);
        assert_eq!(dxf_png.height, dwg_png.height);
        assert_eq!(dxf_png.stamp.sources[0].kind, SourceKind::Dxf);
        let stale_png = check_stale(&StaleRequest {
            output: PathBuf::from(&dxf_png.png),
            source: None,
        })
        .unwrap();
        assert!(!stale_png.stale);
        assert_eq!(stale_png.sources[0].kind, SourceKind::Dxf);

        // `plot`
        let dxf_pdf = plot_with(&PlotRequest::new(dxf_path), &settings).unwrap();
        assert_eq!(dxf_pdf.sheet_size, dwg_pdf.sheet_size);
        assert_eq!(dxf_pdf.stamp.sources[0].kind, SourceKind::Dxf);
        let stale_pdf = check_stale(&StaleRequest {
            output: PathBuf::from(&dxf_pdf.pdf),
            source: None,
        })
        .unwrap();
        assert!(!stale_pdf.stale);

        // `query`
        let dxf_query = query_with(
            &QueryRequest {
                source: dxf_path.clone(),
                sheet: None,
                query: Query::Find(Selection {
                    id: Some("panel/wire-1".into()),
                    ..Selection::default()
                }),
            },
            &settings,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&dxf_query.answer).unwrap()["objects"]
                .as_array()
                .unwrap()
                .len(),
            serde_json::to_value(&dwg_query.answer).unwrap()["objects"]
                .as_array()
                .unwrap()
                .len()
        );

        // `check`
        let dxf_check = check_with(
            &CheckRequest {
                source: dxf_path.clone(),
                sheets: Vec::new(),
            },
            &settings,
        )
        .unwrap();
        assert_eq!(dxf_check.objects_not_read, 0);

        // `import-blocks`
        let lib = dir.join(format!("imported-{}.yaml", expected_format.name()));
        let imported = import_blocks(&ImportBlocksRequest {
            drawing: dxf_path.clone(),
            library: lib,
            blocks: Vec::new(),
            replace: false,
            units: None,
            dry_run: false,
        })
        .unwrap();
        assert_eq!(imported.imported.blocks_added.len(), 1);
        assert_eq!(imported.imported.blocks_added[0].name, "TAG_BOX");
    }
}

#[test]
fn drawing_set_with_format_dxf_builds_changes_accepts_stamps_and_cleans_leftovers() {
    let dir = scratch("dxf-set");
    let settings = no_system_fonts();

    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\nformat: dxf\n",
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        "units: mm
paper: ISO A4 landscape
objects:
  - polyline:
      id: border
      points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]]
      closed: true
  - text:
      id: title
      contents: {row: TITLE}
      position: [20, 190, 0]
      height: 5
",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:
  - sheet: SH-01
    template: template.yaml
    values: {TITLE: 'FIRST SHEET'}
  - sheet: SH-02
    template: template.yaml
    values: {TITLE: 'SECOND SHEET'}
",
    )
    .unwrap();

    // 1. Build the drawing set -> writes `.dxf` files carrying the set build mark.
    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: false,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{built:?}");
    let report = built.result.unwrap();
    assert_eq!(report.built, 2);
    let sh1 = dir.join("out").join("SH-01.dxf");
    let sh2 = dir.join("out").join("SH-02.dxf");
    assert!(sh1.is_file());
    assert!(sh2.is_file());
    assert_eq!(
        report.sheets[0].result.as_ref().unwrap().format,
        DrawingFormat::Dxf
    );

    // 2. A change set updates the sheet and rebuilds its `.dxf`.
    let changed = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {"op": "change", "target": {"id": "SH-01/title"}, "set": {"contents": "UPDATED SHEET"}}
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(changed.ok, "{changed:?}");
    let sh1_text = fs::read_to_string(&sh1).unwrap();
    assert!(sh1_text.contains("UPDATED SHEET"));

    // 3. Accept issues the `.dxf` files; plotting SH-01.dxf alone finds the set via its build mark,
    //    uses the template's declared paper (297 x 210 mm), and stamps `issued: true`.
    let accepted = accept_with(
        &AcceptRequest {
            set: dir.clone(),
            despite_failed_checks: false,
            newest_seen: None,
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(accepted.ok, "{accepted:?}");

    let plotted = plot_with(&PlotRequest::new(&sh1), &settings).unwrap();
    assert!((plotted.sheet_size.width_mm - 297.0).abs() < 0.01);
    assert!((plotted.sheet_size.height_mm - 210.0).abs() < 0.01);
    assert_eq!(plotted.stamp.sources[0].kind, SourceKind::Dxf);
    assert!(
        plotted.stamp.revision.as_ref().is_some_and(|r| r.issued),
        "expected issued stamp on DXF plot: {:?}",
        plotted.stamp.revision
    );

    // 4. Removing SH-02 from rows makes SH-02.dxf a leftover (`kind: dxf`) that `clean: true`
    //    moves to `superseded/`, while an unmarked foreign DXF is never touched.
    let foreign = dir.join("out").join("VENDOR-99.dxf");
    fs::write(&foreign, sh1_text.replace("AgentDrafter", "VendorApp")).unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:
  - sheet: SH-01
    template: template.yaml
    values: {TITLE: 'UPDATED SHEET'}
",
    )
    .unwrap();

    let rebuilt = build_set_with_format(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: Some(true),
        },
        None,
        &settings,
    );
    assert!(rebuilt.ok, "{rebuilt:?}");
    let leftovers = rebuilt.result.unwrap().leftovers.unwrap();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    assert_eq!(leftovers[0].kind, LeftoverKind::Dxf);
    assert_eq!(leftovers[0].sheet, "SH-02");
    assert!(leftovers[0].moved_to.is_some());
    assert!(!sh2.exists());
    assert!(dir
        .join("out")
        .join("superseded")
        .join("SH-02.dxf")
        .is_file());
    assert!(foreign.is_file(), "foreign DXF must never be moved");
}

#[test]
fn revising_original_ascii_and_binary_dxf_preserves_format_and_supports_journal_undo_accept() {
    let dir = std::env::temp_dir().join(format!("agentdrafter-dxf-rev-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let sheet = write_rich_sheet(&dir);
    let settings = no_system_fonts();

    for (name, format) in [
        ("a.dxf", DrawingFormat::Dxf),
        ("b.dxf", DrawingFormat::DxfBinary),
    ] {
        let orig = dir.join(name);
        build_with_format(
            &BuildRequest {
                description: sheet.clone(),
                output: Some(orig.clone()),
                overwrite: false,
            },
            Some(format),
            &settings,
        )
        .unwrap();
        let orig_bytes_before = fs::read(&orig).unwrap();

        // 1. Apply revision change set -> creates `<stem>.rev1.dxf` in same format.
        let rev1 = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: json!({
                    "operations": [
                        {"op": "add_line", "from": [10, 10, 0], "to": [50, 10, 0]}
                    ]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                expected_original_sha256: None,
                plot_style: None,
                no_pdf: false,
            },
            &settings,
        );
        assert!(rev1.ok, "{name}: {rev1:?}");
        let rev1_report = rev1.result.unwrap();
        let rev1_path = dir.join(&rev1_report.revision.as_ref().unwrap().file);
        assert!(rev1_path.extension().is_some_and(|e| e == "dxf"));
        assert!(rev1_path.is_file(), "expected {:?}", rev1_path);
        assert_eq!(
            fs::read(&orig).unwrap(),
            orig_bytes_before,
            "original DXF untouched"
        );

        // Verify format preserved (ASCII vs binary).
        let rev1_bytes = fs::read(&rev1_path).unwrap();
        if format.is_binary_dxf() {
            assert!(rev1_bytes.starts_with(b"AutoCAD Binary DXF"));
        } else {
            assert!(!rev1_bytes.starts_with(b"AutoCAD Binary DXF"));
            assert!(std::str::from_utf8(&rev1_bytes).is_ok());
        }

        // 2. Journal lists the revision.
        let j = journal(&JournalRequest { set: orig.clone() });
        assert!(j.ok, "{name}: {j:?}");
        assert_eq!(j.result.unwrap().entries.len(), 1);

        // 3. Undo removes rev1.dxf.
        let u = undo(&UndoRequest {
            set: orig.clone(),
            change_set: 1,
            dry_run: false,
        });
        assert!(u.ok, "{name}: {u:?}");
        assert_eq!(u.result.unwrap().undid, [1]);
        assert!(!rev1_path.exists());

        // 4. Revise again and accept; plot of accepted revision DXF carries issued stamp.
        let rev2 = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: json!({
                    "operations": [
                        {"op": "add_line", "from": [10, 20, 0], "to": [60, 20, 0]}
                    ]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                expected_original_sha256: None,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(rev2.ok, "{name}: {rev2:?}");
        let rev2_path = dir.join(&rev2.result.unwrap().revision.unwrap().file);
        let acc = accept_with(
            &AcceptRequest {
                set: orig.clone(),
                despite_failed_checks: false,
                newest_seen: None,
                dry_run: false,
            },
            &settings,
            Interface::Cli,
        );
        assert!(acc.ok, "{name}: {acc:?}");
        let plotted = plot_with(&PlotRequest::new(&rev2_path), &settings).unwrap();
        assert!(
            plotted.stamp.revision.as_ref().is_some_and(|r| r.issued),
            "{name}: expected issued stamp on accepted revision DXF: {:?}",
            plotted.stamp.revision
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn layouts_and_viewports_round_trip_through_dxf() {
    let dir = scratch("dxf-layouts");
    let sheet = dir.join("layouts.yaml");
    fs::write(
        &sheet,
        "units: mm
layers:
  equip: {color: 1}
  notes: {color: 3}
layouts:
  - {name: Front, paper: ISO A4 landscape}
  - {name: Back, paper: ISO A4 landscape}
objects:
  - line: {id: m-1, layer: equip, from: [0, 0, 0], to: [100, 50, 0]}
  - text: {id: m-2, layer: notes, contents: 'HIDDEN IN VP', position: [10, 10, 0], height: 4}
  - polyline: {space: Front, points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]], closed: true}
  - viewport:
      id: vp-1
      space: Front
      center: [148.5, 105, 0]
      width: 240
      height: 160
      view_center: [50, 25, 0]
      scale: 1:1
      frozen_layers: [notes]
  - polyline: {space: Back, points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]], closed: true}
  - viewport:
      id: vp-2
      space: Back
      center: [148.5, 105, 0]
      width: 240
      height: 160
      view_center: [50, 25, 0]
      scale: 1:2
",
    )
    .unwrap();
    let settings = no_system_fonts();
    let dxf = dir.join("layouts.dxf");
    build_with_format(
        &BuildRequest {
            description: sheet,
            output: Some(dxf.clone()),
            overwrite: false,
        },
        Some(DrawingFormat::Dxf),
        &settings,
    )
    .unwrap();

    let read = read_dxf(&dxf).unwrap();
    let layout = read
        .layout
        .as_ref()
        .expect("Front layout read back from DXF");
    assert_eq!(layout.name, "Front");
    assert_eq!(read.other_layouts.len(), 1);
    assert_eq!(read.other_layouts[0].name, "Back");
    assert_eq!(read.objects_not_read, 0);

    let plotted = plot_with(&PlotRequest::new(&dxf), &settings).unwrap();
    assert!((plotted.sheet_size.width_mm - 297.0).abs() < 0.01);
    assert!((plotted.sheet_size.height_mm - 210.0).abs() < 0.01);
}
