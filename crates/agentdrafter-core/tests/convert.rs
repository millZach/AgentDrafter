//! Integration tests for DWG/DXF version conversion (`convert`) and per-job
//! `dwg_version` settings in `set.yaml` and `build` (Issue #95, ADR-0004,
//! ADR-0006).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_set_with_options, build_with_options, change_with, convert, convert_reply,
    display_list_with, find_command, query_with, read_dwg, read_original, BuildRequest,
    ChangeRequest, ConversionLossKind, ConvertReport, ConvertRequest, DisplayListRequest,
    DrawingFormat, DwgVersion, ErrorCode, Interface, Query, QueryRequest, ReadRequest, Selection,
    SetBuildRequest, Settings,
};

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-convert-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn header_version(path: &Path) -> String {
    let bytes = fs::read(path).expect("read drawing");
    String::from_utf8_lossy(&bytes[..6]).into_owned()
}

fn build_plain_sheet(dir: &Path, name: &str, version: Option<DwgVersion>) -> PathBuf {
    let yaml = dir.join(format!("{name}.yaml"));
    let dwg = dir.join(format!("{name}.dwg"));
    fs::write(
        &yaml,
        "\
units: mm
layers:
  EQUIP:
    color: 3
objects:
  - line:
      id: rail
      layer: EQUIP
      from: [0, 0, 0]
      to: [100, 0, 0]
  - circle:
      id: nozzle
      layer: EQUIP
      center: [50, 25, 0]
      radius: 10
  - arc:
      id: bend
      center: [20, 20, 0]
      radius: 8
      start_angle: 0
      end_angle: 90
  - text:
      id: label
      contents: PUMP-101
      position: [10, 40, 0]
      height: 5
",
    )
    .expect("write yaml");
    let settings = Settings::default().without_system_fonts();
    let report = build_with_options(
        &BuildRequest {
            description: yaml,
            output: Some(dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        version,
        &settings,
    )
    .expect("build plain sheet");
    let expected = version.unwrap_or_default().code();
    assert_eq!(report.dwg_version, expected);
    assert_eq!(header_version(&dwg), expected);
    dwg
}

fn build_rich_sheet(dir: &Path) -> PathBuf {
    let yaml = dir.join("rich.yaml");
    let dwg = dir.join("rich.dwg");
    fs::write(
        &yaml,
        "\
units: mm
custom_properties:
  PLANT_AREA: AREA-04
annotation_scales:
  '1:1': [1, 1]
  '1:2': [1, 2]
annotation_scale: '1:1'
layers:
  MASK:
    color: 7
    transparency: 50
objects:
  - line:
      id: base
      from: [0, 0, 0]
      to: [120, 0, 0]
  - text:
      id: anno-note
      contents: VALVE-201
      position: [10, 10, 0]
      height: 3.5
      annotative: true
      annotation_scales: ['1:1', '1:2']
      transparency: 30
  - table:
      id: sched
      position: [10, 60, 0]
      columns: [30, 30]
      rows: [8, 8]
      cells:
        - [TAG, SERVICE]
        - [P-101, COOLING]
  - mleader:
      id: callout
      leaders: [[[50, 0, 0], [60, 15, 0]]]
      content: {text: SEE NOTE 1}
  - pdf_underlay:
      id: bg-pdf
      path: missing-underlay.pdf
      page: '1'
      position: [0, 0, 0]
      scale: 1
      rotation: 0
",
    )
    .expect("write rich yaml");
    let settings = Settings::default().without_system_fonts();
    build_with_options(
        &BuildRequest {
            description: yaml,
            output: Some(dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        None,
        &settings,
    )
    .expect("build rich sheet");
    dwg
}

#[test]
fn convert_plain_drawing_down_and_back_up_preserves_geometry_and_original() {
    let dir = temp_dir("down-up");
    let src = build_plain_sheet(&dir, "plain", None);
    let orig_bytes = fs::read(&src).expect("read original");

    // Convert 2018 -> 2013, 2010, 2007, 2004, 2000 and verify clean headers & round-trip.
    for version in [
        DwgVersion::R2013,
        DwgVersion::R2010,
        DwgVersion::R2007,
        DwgVersion::R2004,
        DwgVersion::R2000,
    ] {
        let report = convert(&ConvertRequest {
            source: src.clone(),
            version,
            output: None,
            format: None,
            overwrite: false,
            allow_loss: false,
            dry_run: false,
        })
        .expect("convert down");
        let ConvertReport::Drawing(drawing) = report else {
            panic!("expected single drawing report");
        };
        assert_eq!(drawing.source_version, "AC1032");
        assert_eq!(drawing.target_version, version.code());
        assert!(
            drawing.output_decode.decoded,
            "{version}: {:?}",
            drawing.output_decode
        );
        assert_eq!(
            drawing.unchanged_objects, drawing.total_objects,
            "{version} losses: {:?}",
            drawing.losses
        );
        assert!(drawing.lossless, "{version} losses: {:?}", drawing.losses);
        let out_path = PathBuf::from(&drawing.output);
        assert!(out_path.is_file());
        assert_eq!(header_version(&out_path), version.code());

        // Converted file reads cleanly through read_dwg, query, and display_list.
        let contents = read_dwg(&out_path).expect("read_dwg on converted");
        assert_eq!(contents.dwg_version, version.code());
        assert_eq!(contents.objects.len(), 4);
        let settings = Settings::default().without_system_fonts();
        let dl = display_list_with(&DisplayListRequest::new(out_path.clone()), &settings)
            .expect("display_list on converted");
        assert!(!dl.display_list.items.is_empty());
        let q = query_with(
            &QueryRequest {
                source: out_path,
                sheet: None,
                query: Query::Find(Selection::default()),
            },
            &settings,
        )
        .expect("query on converted");
        assert_eq!(q.sheets.len(), 1);
    }

    // Original must still be untouched byte-for-byte (ADR-0004).
    assert_eq!(fs::read(&src).expect("re-read original"), orig_bytes);

    // Convert 2000 back UP to 2018.
    let v2000 = dir.join("plain.2000.dwg");
    let up_path = dir.join("plain.up-2018.dwg");
    let up = convert(&ConvertRequest {
        source: v2000,
        version: DwgVersion::R2018,
        output: Some(up_path.clone()),
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("convert up to 2018");
    let ConvertReport::Drawing(up_drawing) = up else {
        panic!("expected drawing report");
    };
    assert_eq!(up_drawing.source_version, "AC1015");
    assert_eq!(up_drawing.target_version, "AC1032");
    assert!(up_drawing.lossless, "up losses: {:?}", up_drawing.losses);
    assert_eq!(header_version(&up_path), "AC1032");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_reports_feature_losses_when_saving_rich_drawing_to_older_versions() {
    let dir = temp_dir("rich-losses");
    let src = build_rich_sheet(&dir);

    // Save down to 2007 (AC1021): PDF underlay is newer (2010), so it must be reported.
    let r2007 = convert(&ConvertRequest {
        source: src.clone(),
        version: DwgVersion::R2007,
        output: None,
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("convert rich to 2007");
    let ConvertReport::Drawing(d2007) = r2007 else {
        panic!("expected drawing report");
    };
    assert!(!d2007.lossless);
    assert!(
        d2007
            .losses
            .iter()
            .any(|l| l.kind.contains("UNDERLAY") && l.change == ConversionLossKind::Downgraded),
        "expected underlay loss in 2007: {:?}",
        d2007.losses
    );

    // Save down to 2004 (AC1018): PDF underlay, MLeader, and AnnotationScale must be reported.
    let r2004 = convert(&ConvertRequest {
        source: src.clone(),
        version: DwgVersion::R2004,
        output: None,
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("convert rich to 2004");
    let ConvertReport::Drawing(d2004) = r2004 else {
        panic!("expected drawing report");
    };
    assert!(!d2004.lossless);
    assert!(
        d2004
            .losses
            .iter()
            .any(|l| l.kind.contains("MLEADER") || l.kind.contains("MULTILEADER")),
        "expected mleader loss in 2004: {:?}",
        d2004.losses
    );
    assert!(
        d2004.losses.iter().any(|l| l.kind == "ANNOTATION_SCALE"),
        "expected annotation scale loss in 2004: {:?}",
        d2004.losses
    );

    // Save down to 2000 (AC1015): Table, transparency, and custom drawing properties also reported.
    let r2000 = convert(&ConvertRequest {
        source: src,
        version: DwgVersion::R2000,
        output: None,
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: true,
    })
    .expect("dry-run convert rich to 2000");
    let ConvertReport::Drawing(d2000) = r2000 else {
        panic!("expected drawing report");
    };
    assert!(d2000.dry_run);
    assert!(
        !Path::new(&d2000.output).exists(),
        "dry_run must not write file"
    );
    assert!(
        d2000.losses.iter().any(|l| l.kind.contains("TABLE")),
        "expected table loss in 2000: {:?}",
        d2000.losses
    );
    assert!(
        d2000.losses.iter().any(|l| l.kind == "SUMMARY_INFO"),
        "expected summary_info loss in 2000: {:?}",
        d2000.losses
    );
    assert!(
        d2000
            .losses
            .iter()
            .any(|l| l.kind == "LAYER" && l.detail.contains("transparency")),
        "expected layer transparency loss in 2000: {:?}",
        d2000.losses
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_refuses_to_overwrite_original_and_honors_overwrite_flag() {
    let dir = temp_dir("no-overwrite");
    let src = build_plain_sheet(&dir, "orig", None);

    // Output == source must be refused even when overwrite is true (ADR-0004).
    let err = convert(&ConvertRequest {
        source: src.clone(),
        version: DwgVersion::R2010,
        output: Some(src.clone()),
        format: None,
        overwrite: true,
        allow_loss: false,
        dry_run: false,
    })
    .expect_err("must refuse overwriting source");
    assert_eq!(err.code, ErrorCode::Usage);

    // First write to a distinct output succeeds; second write without overwrite fails.
    let out = dir.join("out.dwg");
    convert(&ConvertRequest {
        source: src.clone(),
        version: DwgVersion::R2010,
        output: Some(out.clone()),
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("first convert");
    let exists_err = convert(&ConvertRequest {
        source: src.clone(),
        version: DwgVersion::R2013,
        output: Some(out.clone()),
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect_err("second convert without overwrite must fail");
    assert_eq!(exists_err.code, ErrorCode::OutputExists);
    assert_eq!(header_version(&out), "AC1024");

    // With overwrite: true on the non-source output, replacing succeeds.
    convert(&ConvertRequest {
        source: src,
        version: DwgVersion::R2013,
        output: Some(out.clone()),
        format: None,
        overwrite: true,
        allow_loss: false,
        dry_run: false,
    })
    .expect("overwrite non-source output");
    assert_eq!(header_version(&out), "AC1027");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_cross_format_dwg_and_dxf_with_version() {
    let dir = temp_dir("dxf-version");
    let src = build_plain_sheet(&dir, "sheet", None);
    let dxf_out = dir.join("sheet.2010.dxf");

    // DWG 2018 -> ASCII DXF 2010 (format inferred from `.dxf` extension).
    let r = convert(&ConvertRequest {
        source: src,
        version: DwgVersion::R2010,
        output: Some(dxf_out.clone()),
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("convert dwg -> dxf 2010");
    let ConvertReport::Drawing(d) = r else {
        panic!("expected drawing report");
    };
    assert_eq!(d.format, DrawingFormat::Dxf);
    assert_eq!(d.target_version, "AC1024");
    let dxf_text = fs::read_to_string(&dxf_out).expect("read ascii dxf");
    assert!(dxf_text.contains("AC1024"), "DXF must carry AC1024 header");

    // DXF 2010 -> DWG 2018.
    let dwg_back = dir.join("sheet.back-2018.dwg");
    let back = convert(&ConvertRequest {
        source: dxf_out,
        version: DwgVersion::R2018,
        output: Some(dwg_back.clone()),
        format: Some(DrawingFormat::Dwg),
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("convert dxf 2010 -> dwg 2018");
    let ConvertReport::Drawing(b) = back else {
        panic!("expected drawing report");
    };
    assert_eq!(b.source_version, "AC1024");
    assert_eq!(b.target_version, "AC1032");
    assert_eq!(header_version(&dwg_back), "AC1032");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_yaml_dwg_version_builds_and_changes_in_that_version_and_batch_convert_works() {
    let dir = temp_dir("set-version");
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\ndwg_version: 2010\n",
    )
    .expect("write set.yaml");
    fs::write(
        dir.join("sheet.yaml"),
        "\
units: mm
objects:
  - line:
      id: border
      from: [0, 0, 0]
      to: [200, 0, 0]
  - text:
      id: title
      contents: {row: TITLE}
      position: [10, 20, 0]
      height: 5
",
    )
    .expect("write template");
    fs::write(
        dir.join("rows.yaml"),
        "\
rows:
  - sheet: S-001
    template: sheet.yaml
    values: {TITLE: SHEET ONE}
  - sheet: S-002
    template: sheet.yaml
    values: {TITLE: SHEET TWO}
",
    )
    .expect("write rows.yaml");

    let settings = Settings::default().without_system_fonts();
    let reply = build_set_with_options(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        None,
        None,
        &settings,
    );
    assert!(reply.ok, "{:?}", reply.error);
    let report = reply.result.expect("set report");
    assert_eq!(report.built, 2);
    for sheet in &report.sheets {
        let res = sheet.result.as_ref().expect("sheet built");
        assert_eq!(res.dwg_version, "AC1024");
        assert_eq!(header_version(Path::new(&res.dwg)), "AC1024");
    }

    // Rebuilding with --overwrite works (including when set.yaml uses 2000 / AC1015).
    let rebuild = build_set_with_options(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        None,
        Some(DwgVersion::R2000),
        &settings,
    );
    assert!(rebuild.ok, "{:?}", rebuild.error);
    assert_eq!(header_version(&dir.join("dwg/S-001.dwg")), "AC1015");

    // Rebuilding again over the AC1015 builds succeeds via the NOD build mark!
    let rebuild2 = build_set_with_options(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        None,
        None,
        &settings,
    );
    assert!(
        rebuild2.ok,
        "overwrite over AC1015 build failed: {:?}",
        rebuild2.error
    );
    assert_eq!(header_version(&dir.join("dwg/S-001.dwg")), "AC1024");

    // `change` on the set keeps rebuilding sheets in `set.yaml`'s `dwg_version` (AC1024).
    let change_reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: serde_json::json!({
                "operations": [
                    { "op": "move", "target": { "id": "S-001/border" }, "by": [5, 0, 0] }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(change_reply.ok, "{:?}", change_reply.error);
    assert_eq!(header_version(&dir.join("dwg/S-001.dwg")), "AC1024");

    // Batch-convert the drawing set folder to 2018 (writes into dwg/converted-2018/).
    let batch_reply = convert_reply(&ConvertRequest {
        source: dir.clone(),
        version: DwgVersion::R2018,
        output: None,
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    });
    assert!(batch_reply.ok, "{:?}", batch_reply.error);
    let ConvertReport::Batch(batch) = batch_reply.result.expect("batch report") else {
        panic!("expected batch report");
    };
    assert_eq!(batch.converted, 2);
    assert_eq!(batch.failed, 0);
    assert_eq!(batch.target_version, "AC1032");
    assert_eq!(
        header_version(&dir.join("dwg/converted-2018/S-001.dwg")),
        "AC1032"
    );
    assert_eq!(
        header_version(&dir.join("dwg/converted-2018/S-002.dwg")),
        "AC1032"
    );
    // Original set DWGs stay AC1024.
    assert_eq!(header_version(&dir.join("dwg/S-001.dwg")), "AC1024");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn revising_an_older_original_never_converts_its_version() {
    let dir = temp_dir("revise-keeps-version");
    let v2018 = build_plain_sheet(&dir, "base", None);
    let orig_2010 = dir.join("plant-2010.dwg");
    convert(&ConvertRequest {
        source: v2018,
        version: DwgVersion::R2010,
        output: Some(orig_2010.clone()),
        format: None,
        overwrite: false,
        allow_loss: false,
        dry_run: false,
    })
    .expect("make 2010 drawing");
    assert_eq!(header_version(&orig_2010), "AC1024");

    let read = read_original(&ReadRequest {
        source: orig_2010.clone(),
        mirror: None,
        no_mirror: true,
    })
    .expect("read 2010 drawing");
    assert_eq!(read.dwg_version, "AC1024");

    // Revising the 2010 drawing writes a new revision still in AC1024 (ADR-0006).
    let rev_out = dir.join("plant-2010.rev1.dwg");
    let settings = Settings::default().without_system_fonts();
    let rev = find_command("change").expect("change command").run(
        serde_json::json!({
            "set": orig_2010.to_string_lossy(),
            "change_set": {
                "operations": [
                    { "op": "add_line", "from": [0, 50, 0], "to": [100, 50, 0] }
                ]
            },
            "output": rev_out.to_string_lossy(),
            "no_pdf": true,
        }),
        &settings,
        Interface::Cli,
    );
    assert!(rev.ok, "{:?}", rev.error);
    assert_eq!(header_version(&rev_out), "AC1024");
    assert_eq!(header_version(&orig_2010), "AC1024");

    let _ = fs::remove_dir_all(&dir);
}
