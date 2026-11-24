//! Integration tests for Issue #88: Plot-style control — named plot styles
//! (STB), CTB/STB screening, layer & object transparency, `set.yaml`
//! `plot_style`, `page_setup` (`PAGESETUP`), plot areas & plot scale, and
//! `set_page_setup` on drawing sets and original DWGs.
//!
//! All drawings and plot styles in these tests are made up (ADR-0008).

#[path = "support/ctb.rs"]
mod ctb;

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, change_with, parse_sheet_description, plot_set_with, plot_with, query_with,
    read_dwg, render_with, revise_with, undo, BuildRequest, ChangeRequest, ObjectPlotStyle,
    ObjectTransparency, PlotArea, PlotRequest, PlotScale, PlotSetRequest, PlotStyleMode, Query,
    QueryRequest, RenderRequest, ReviseRequest, Settings, UndoRequest,
};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-plot-style-control")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn no_sys_settings(plot_styles_dir: Option<&Path>) -> Settings {
    let mut s = Settings::default();
    if let Some(dir) = plot_styles_dir {
        s.plot_style_folders.push(dir.to_path_buf());
    }
    s
}

/// Decompresses all FlateDecode streams inside a PDF so tests can inspect
/// rendered PDF commands (colors, lineweights, ExtGState `/GS` operators).
fn pdf_streams_text(pdf_bytes: &[u8]) -> String {
    let mut out = String::from_utf8_lossy(pdf_bytes).into_owned();
    let mut pos = 0;
    while let Some(rel) = pdf_bytes[pos..].windows(6).position(|w| w == b"stream") {
        let mut start = pos + rel + 6;
        if pdf_bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if pdf_bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        if let Some(end_rel) = pdf_bytes[start..]
            .windows(9)
            .position(|w| w == b"endstream")
        {
            let slice = &pdf_bytes[start..start + end_rel];
            let trimmed = slice.strip_suffix(b"\n").unwrap_or(slice);
            let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
            if let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec_zlib(trimmed) {
                out.push('\n');
                out.push_str(&String::from_utf8_lossy(&decompressed));
            }
            pos = start + end_rel + 9;
        } else {
            break;
        }
    }
    out
}

fn decode_png_rgba(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(
        fs::File::open(path).expect("open png"),
    ));
    let mut reader = decoder.read_info().expect("read png info");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("decode png frame");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn stb_named_plot_styles_round_trip_dwg_and_plot_pdf_and_png() {
    let dir = scratch("stb-roundtrip");
    let ps_dir = dir.join("plot-styles");
    fs::create_dir_all(&ps_dir).unwrap();

    // Made-up STB with three named styles:
    // - Heavy: red (255, 0, 0), 0.70 mm (table index 13 -> 14), screen 100
    // - Light: blue (0, 0, 255), 0.18 mm (table index 4 -> 5), screen 50 -> (128, 128, 255)
    // - Accent: green (0, 255, 0), 0.50 mm (table index 10 -> 11), screen 100
    let stb_bytes = ctb::made_up_stb(&[
        ("Heavy", -1073741824 + (255 << 16), 14, 100),
        ("Light", -1073741824 + 255, 5, 50),
        ("Accent", -1073741824 + (255 << 8), 11, 100),
    ]);
    fs::write(ps_dir.join("plant.stb"), &stb_bytes).unwrap();

    fs::write(
        dir.join("blocks.yaml"),
        "\
units: mm
blocks:
  MARK:
    objects:
      - solid: {corners: [[0, 0, 0], [20, 0, 0], [20, 20, 0], [0, 20, 0]], layer: '0', plot_style: ByBlock}
",
    )
    .unwrap();

    let yaml = "\
units: mm
paper: [100, 60]
block_library: blocks.yaml
layers:
  heavy: {color: 7, plot_style: Heavy}
  light: {color: 7, plot_style: Light}
objects:
  - solid: {corners: [[5, 5, 0], [25, 5, 0], [25, 25, 0], [5, 25, 0]], layer: heavy, id: s-heavy}
  - solid: {corners: [[35, 5, 0], [55, 5, 0], [55, 25, 0], [35, 25, 0]], layer: light, id: s-light}
  - insert: {block: MARK, position: [65, 5, 0], layer: heavy, plot_style: Accent, id: ins-accent}
";
    let sheet_path = dir.join("stb.yaml");
    let dwg_path = dir.join("stb.dwg");
    fs::write(&sheet_path, yaml).unwrap();

    let settings = no_sys_settings(Some(&ps_dir));
    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build STB DWG");

    // Read back the DWG and verify PSTYLEMODE=0 + layer/entity/block plot styles.
    let contents = read_dwg(&dwg_path).expect("read back STB DWG");
    assert_eq!(contents.plot_style_mode, PlotStyleMode::Named);
    let heavy_layer = contents.layers.iter().find(|l| l.name == "heavy").unwrap();
    let light_layer = contents.layers.iter().find(|l| l.name == "light").unwrap();
    assert_eq!(heavy_layer.plot_style.as_deref(), Some("Heavy"));
    assert_eq!(light_layer.plot_style.as_deref(), Some("Light"));
    let ins_obj = contents
        .objects
        .iter()
        .find(|o| matches!(o, agentdrafter_core::Object::Insert(_)))
        .unwrap();
    assert_eq!(
        ins_obj.properties().plot_style,
        ObjectPlotStyle::Named("Accent".to_owned())
    );
    let mark_block = contents.blocks.iter().find(|b| b.name == "MARK").unwrap();
    assert_eq!(
        mark_block.objects[0].properties().plot_style,
        ObjectPlotStyle::ByBlock
    );

    // Plot the built DWG through `plant.stb`.
    let pdf_path = dir.join("stb.pdf");
    let mut plot_req = PlotRequest::new(&dwg_path);
    plot_req.output = Some(pdf_path.clone());
    plot_req.overwrite = true;
    plot_req.plot_style = Some("plant.stb".to_owned());
    let plot_report = plot_with(&plot_req, &settings).expect("plot STB DWG");
    assert_eq!(plot_report.plot_style.name, "plant.stb");
    assert!(
        plot_report.plot_style.not_followed.is_empty(),
        "screening is followed so not_followed must be empty: {:?}",
        plot_report.plot_style.not_followed
    );
    let pdf_text = pdf_streams_text(&fs::read(&pdf_path).unwrap());
    // Heavy = pure red (1 0 0 rg), Accent = pure green (0 1 0 rg), Light = 50%-screened blue (128/255 ≈ 0.50196).
    assert!(pdf_text.contains("1 0 0 rg"), "missing Heavy red in PDF");
    assert!(pdf_text.contains("0 1 0 rg"), "missing Accent green in PDF");
    assert!(
        pdf_text.contains("0.50196") && pdf_text.contains("1 rg"),
        "missing 50%-screened Light blue in PDF"
    );

    // Render to PNG through `plant.stb` and verify pixel colors match the STB!
    let png_path = dir.join("stb.png");
    let mut render_req = RenderRequest::new(&dwg_path);
    render_req.output = Some(png_path.clone());
    render_req.overwrite = true;
    render_req.width = Some(400);
    render_req.plot_style = Some("plant.stb".to_owned());
    render_with(&render_req, &settings).expect("render STB PNG");
    let (w, h, rgba) = decode_png_rgba(&png_path);
    assert!(w > 0 && h > 0);
    let has_pixel = |r: u8, g: u8, b: u8| {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0].abs_diff(r) <= 2 && px[1].abs_diff(g) <= 2 && px[2].abs_diff(b) <= 2)
    };
    assert!(has_pixel(255, 0, 0), "PNG should have Heavy red pixels");
    assert!(
        has_pixel(128, 128, 255),
        "PNG should have 50%-screened Light blue pixels"
    );
    assert!(has_pixel(0, 255, 0), "PNG should have Accent green pixels");
}

#[test]
fn ctb_screening_plots_lightened_colors_and_reports_no_not_followed() {
    let dir = scratch("ctb-screening");
    let ps_dir = dir.join("plot-styles");
    fs::create_dir_all(&ps_dir).unwrap();

    // Pen 1 (index 0): red with screen=50 -> (255, 128, 128).
    // Pen 2 (index 1): black with screen=50 -> (128, 128, 128).
    let mut text = String::from(
        "description=\"made-up screening ctb\"\n\
         aci_table_available=TRUE\n\
         scale_factor=1.0\n\
         apply_factor=FALSE\n\
         custom_lineweight_display_units=0\n\
         aci_table{\n",
    );
    for i in 0..255 {
        text.push_str(&format!(" {i}=\"Color_{}\"\n", i + 1));
    }
    text.push_str("}\nplot_style{\n");
    for i in 0..255 {
        let (color, screen) = match i {
            0 => (-1073741824 + (255 << 16), 50), // red @ 50%
            1 => (-1073741824, 50),               // black @ 50%
            _ => (-1, 100),
        };
        text.push_str(&format!(
            " {i}{{\n  name=\"Color_{}\"\n  color={color}\n  color_policy=1\n  screen={screen}\n  linetype=31\n  lineweight=0\n  fill_style=73\n  end_style=4\n  join_style=5\n }}\n",
            i + 1
        ));
    }
    text.push_str("}\ncustom_lineweight_table{\n 0=0.0\n 1=0.25\n}\n");
    fs::write(ps_dir.join("screened.ctb"), ctb::ctb_file(&text)).unwrap();

    let yaml = "\
units: mm
paper: [80, 40]
layers:
  l1: {color: 1}
  l2: {color: 2}
objects:
  - solid: {corners: [[5, 5, 0], [30, 5, 0], [30, 30, 0], [5, 30, 0]], layer: l1}
  - solid: {corners: [[45, 5, 0], [70, 5, 0], [70, 30, 0], [45, 30, 0]], layer: l2}
";
    let sheet_path = dir.join("screen.yaml");
    let pdf_path = dir.join("screen.pdf");
    fs::write(&sheet_path, yaml).unwrap();

    let settings = no_sys_settings(Some(&ps_dir));
    let mut plot_req = PlotRequest::new(&sheet_path);
    plot_req.output = Some(pdf_path.clone());
    plot_req.overwrite = true;
    plot_req.plot_style = Some("screened.ctb".to_owned());
    let report = plot_with(&plot_req, &settings).expect("plot screened CTB");
    assert!(
        report.plot_style.not_followed.is_empty(),
        "expected empty not_followed, got {:?}",
        report.plot_style.not_followed
    );
    let pdf_text = pdf_streams_text(&fs::read(&pdf_path).unwrap());
    // 50%-screened red = (255, 128, 128) -> `1 0.50196... 0.50196... rg`
    // 50%-screened black = (128, 128, 128) -> `0.50196... 0.50196... 0.50196... rg`
    assert!(
        pdf_text.contains("1 0.50196"),
        "missing 50% red in PDF: {pdf_text}"
    );
    assert!(
        pdf_text.contains("0.5019608 0.5019608 0.5019608 rg")
            || pdf_text.contains("0.50196 0.50196 0.50196 rg"),
        "missing 50% black in PDF"
    );
}

#[test]
fn transparency_round_trips_dwg_and_draws_in_png_and_pdf_with_plot_transparency_toggle() {
    let dir = scratch("transparency");
    fs::write(
        dir.join("blocks.yaml"),
        "\
units: mm
blocks:
  B:
    objects:
      - solid: {corners: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]], layer: '0', transparency: ByBlock}
",
    )
    .unwrap();
    let yaml = "\
units: mm
paper: [100, 60]
block_library: blocks.yaml
layers:
  bg: {color: 7}
  translucent: {color: 1, transparency: 50}
objects:
  - solid: {corners: [[0, 0, 0], [100, 0, 0], [100, 60, 0], [0, 60, 0]], layer: bg}
  - solid: {corners: [[10, 10, 0], [45, 10, 0], [45, 50, 0], [10, 50, 0]], layer: translucent}
  - solid: {corners: [[55, 10, 0], [90, 10, 0], [90, 50, 0], [55, 50, 0]], layer: bg, color: 3, transparency: 75}
  - insert: {block: B, position: [5, 5, 0], layer: bg, transparency: 40}
";
    let sheet_path = dir.join("trans.yaml");
    let dwg_path = dir.join("trans.dwg");
    fs::write(&sheet_path, yaml).unwrap();

    let settings = no_sys_settings(None);
    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build transparency DWG");

    // Verify DWG round-trip of layer, entity, and ByBlock transparency.
    let read = read_dwg(&dwg_path).expect("read back transparency DWG");
    let tl = read
        .layers
        .iter()
        .find(|l| l.name == "translucent")
        .unwrap();
    assert_eq!(tl.transparency, 50);
    assert_eq!(
        read.objects[2].properties().transparency,
        ObjectTransparency::Percent(75)
    );
    assert_eq!(
        read.objects[3].properties().transparency,
        ObjectTransparency::Percent(40)
    );
    let blk = read.blocks.iter().find(|b| b.name == "B").unwrap();
    assert_eq!(
        blk.objects[0].properties().transparency,
        ObjectTransparency::ByBlock
    );

    // Render PNG: bg solid (color 7 -> black on paper) + 50% red solid -> ~(128, 0, 0).
    let png_path = dir.join("trans.png");
    let mut render_req = RenderRequest::new(&dwg_path);
    render_req.output = Some(png_path.clone());
    render_req.overwrite = true;
    render_req.width = Some(300);
    render_with(&render_req, &settings).expect("render transparency PNG");
    let (_, _, rgba) = decode_png_rgba(&png_path);
    let has_blended_red = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|px| px[0].abs_diff(128) <= 4 && px[1] <= 4 && px[2] <= 4);
    assert!(
        has_blended_red,
        "PNG should blend 50% red over black to ~(128, 0, 0)"
    );

    // Plot PDF (plot_transparency defaults to true) -> emits ExtGState /GS50 and /GS75.
    let pdf_on = dir.join("trans-on.pdf");
    let mut plot_on = PlotRequest::new(&dwg_path);
    plot_on.output = Some(pdf_on.clone());
    plot_on.overwrite = true;
    plot_with(&plot_on, &settings).expect("plot transparent PDF");
    let pdf_on_text = pdf_streams_text(&fs::read(&pdf_on).unwrap());
    assert!(pdf_on_text.contains("/GS50"), "PDF should emit /GS50");
    assert!(pdf_on_text.contains("/GS75"), "PDF should emit /GS75");
    assert!(pdf_on_text.contains("/ca 0.5"), "PDF should emit /ca 0.5");

    // Plot with `plot_transparency: false` -> NO ExtGState emitted.
    let yaml_off = format!("{yaml}page_setup:\n  plot_transparency: false\n");
    let sheet_off = dir.join("trans-off.yaml");
    let pdf_off = dir.join("trans-off.pdf");
    fs::write(&sheet_off, yaml_off).unwrap();
    let mut plot_off = PlotRequest::new(&sheet_off);
    plot_off.output = Some(pdf_off.clone());
    plot_off.overwrite = true;
    plot_with(&plot_off, &settings).expect("plot with plot_transparency: false");
    let pdf_off_text = pdf_streams_text(&fs::read(&pdf_off).unwrap());
    assert!(
        !pdf_off_text.contains("/GS50"),
        "PDF with plot_transparency: false must not emit /GS50"
    );
}

#[test]
fn page_setup_window_scale_center_and_dwg_round_trip() {
    let dir = scratch("page-setup");
    let ps_dir = dir.join("plot-styles");
    fs::create_dir_all(&ps_dir).unwrap();
    let stb_bytes = ctb::made_up_stb(&[("Heavy", -1073741824 + (255 << 16), 10, 100)]);
    fs::write(ps_dir.join("sheet.stb"), &stb_bytes).unwrap();

    let yaml = "\
units: mm
page_setup:
  paper: ISO A4 landscape
  area: window
  window: [[10, 20], [210, 120]]
  scale: '1:2'
  center: true
  plot_style: sheet.stb
  plot_transparency: false
layers:
  walls: {color: 7, plot_style: Heavy}
objects:
  - line: {from: [10, 20, 0], to: [210, 120, 0], layer: walls}
";
    let parsed = parse_sheet_description(yaml).expect("parse sheet with page_setup");
    let ps = parsed.page_setup.as_ref().expect("page_setup parsed");
    assert_eq!(
        ps.area,
        Some(PlotArea::Window {
            min: [10.0, 20.0],
            max: [210.0, 120.0],
        })
    );
    assert_eq!(
        ps.scale,
        PlotScale::Ratio {
            numerator: 1.0,
            denominator: 2.0,
        }
    );
    assert!(ps.center);
    assert_eq!(ps.plot_style.as_deref(), Some("sheet.stb"));
    assert!(!ps.plot_transparency);

    let sheet_path = dir.join("ps.yaml");
    let dwg_path = dir.join("ps.dwg");
    fs::write(&sheet_path, yaml).unwrap();
    let settings = no_sys_settings(Some(&ps_dir));

    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build page_setup DWG");

    // Verify DWG round-trip of model-space PlotSettings!
    let read = read_dwg(&dwg_path).expect("read back page_setup DWG");
    let rps = read
        .page_setup
        .as_ref()
        .expect("model page_setup round-tripped");
    assert_eq!(rps.plot_style.as_deref(), Some("sheet.stb"));
    assert!(rps.center);
    assert!(!rps.plot_transparency);
    assert_eq!(
        rps.area,
        Some(PlotArea::Window {
            min: [10.0, 20.0],
            max: [210.0, 120.0],
        })
    );
    assert!((rps.scale.factor().unwrap() - 0.5).abs() < 1e-6);

    // Plot the built DWG directly (no --plot-style flag): should pick up `sheet.stb`, ISO A4 landscape, scale 0.5!
    let pdf_path = dir.join("ps.pdf");
    let mut plot_req = PlotRequest::new(&dwg_path);
    plot_req.output = Some(pdf_path.clone());
    plot_req.overwrite = true;
    let report = plot_with(&plot_req, &settings).expect("plot page_setup DWG");
    assert!((report.sheet_size.width_mm - 297.0).abs() < 0.05);
    assert!((report.sheet_size.height_mm - 210.0).abs() < 0.05);
    assert!((report.scale - 0.5).abs() < 1e-6);
    assert_eq!(report.plot_style.name, "sheet.stb");

    // Query layout on the YAML sheet reports the configured page_setup.
    let q = query_with(
        &QueryRequest {
            source: sheet_path,
            sheet: None,
            query: Query::Layout,
        },
        &settings,
    )
    .expect("query layout on YAML");
    let agentdrafter_core::Answer::Layout { layouts, .. } = q.answer else {
        panic!("expected Layout answer");
    };
    let used = layouts[0].page_setup.as_ref().expect("page_setup reported");
    assert_eq!(used.area, "window");
    assert_eq!(used.scale, "1:2");
    assert_eq!(used.plot_style, "sheet.stb");
}

#[test]
fn set_yaml_plot_style_and_set_page_setup_change_with_undo() {
    let dir = scratch("set-plot-style");
    let ps_dir = dir.join("plot-styles");
    fs::create_dir_all(&ps_dir).unwrap();
    let stb_bytes = ctb::made_up_stb(&[("Process", -1073741824 + (255 << 16), 10, 100)]);
    fs::write(ps_dir.join("plant.stb"), &stb_bytes).unwrap();
    let ctb_bytes = ctb::made_up_ctb(&[(1, -1073741824 + (255 << 8), 0)]);
    fs::write(ps_dir.join("override.ctb"), &ctb_bytes).unwrap();

    let set_dir = dir.join("set");
    fs::create_dir_all(set_dir.join("templates")).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\nplot_style: plant.stb\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        "rows:\n  - sheet: S-001\n    template: templates/sheet.yaml\n    values: {}\n",
    )
    .unwrap();
    let tpl_path = set_dir.join("templates/sheet.yaml");
    let tpl_before = "\
units: mm
paper: ISO A3 landscape
layers:
  pipe: {color: 1, plot_style: Process}
objects:
  - line: {from: [10, 10, 0], to: [110, 10, 0], layer: pipe, id: pipe-1}
";
    fs::write(&tpl_path, tpl_before).unwrap();

    let settings = no_sys_settings(Some(&ps_dir));

    // Plotting the set with no CLI plot_style uses `set.yaml`'s `plant.stb`.
    let set_reply = plot_set_with(
        &PlotSetRequest {
            set: set_dir.clone(),
            overwrite: true,
            clean: None,
            plot_style: None,
        },
        &settings,
    );
    assert!(set_reply.ok, "{:?}", set_reply.error);
    let set_plot = set_reply.result.unwrap();
    assert_eq!(
        set_plot.sheets[0].result.as_ref().unwrap().plot_style.name,
        "plant.stb"
    );
    let pdf_text = pdf_streams_text(&fs::read(set_dir.join("dwg/S-001.pdf")).unwrap());
    assert!(
        pdf_text.contains("1 0 0 RG"),
        "Process red stroke from plant.stb"
    );

    // CLI `--plot-style` overrides `set.yaml`'s `plot_style`.
    let cli_reply = plot_set_with(
        &PlotSetRequest {
            set: set_dir.clone(),
            overwrite: true,
            clean: None,
            plot_style: Some("override.ctb".to_owned()),
        },
        &settings,
    );
    assert!(cli_reply.ok, "{:?}", cli_reply.error);
    assert_eq!(
        cli_reply.result.unwrap().sheets[0]
            .result
            .as_ref()
            .unwrap()
            .plot_style
            .name,
        "override.ctb"
    );

    // Apply `set_page_setup` (`PAGESETUP`) via change set on the drawing set.
    let change_reply = change_with(
        &ChangeRequest {
            set: set_dir.clone(),
            change_set: serde_json::json!({
                "operations": [
                    {
                        "op": "PAGESETUP",
                        "sheet": "S-001",
                        "set": {
                            "paper": "ISO A4 landscape",
                            "area": "extents",
                            "scale": "1:2",
                            "center": true
                        }
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(change_reply.ok, "{:?}", change_reply.error);
    let change_report = change_reply.result.unwrap();
    assert_eq!(change_report.landed, 1);

    // Plotting now uses ISO A4 landscape at 1:2!
    let mut after_req = PlotRequest::new(set_dir.join("dwg/S-001.dwg"));
    after_req.output = Some(dir.join("after.pdf"));
    after_req.overwrite = true;
    let after_plot = plot_with(&after_req, &settings).expect("plot after PAGESETUP");
    assert!((after_plot.sheet_size.width_mm - 297.0).abs() < 0.05);
    assert!((after_plot.sheet_size.height_mm - 210.0).abs() < 0.05);
    assert!((after_plot.scale - 0.5).abs() < 1e-6);

    // Undo restores the template byte-for-byte!
    let undo_reply = undo(&UndoRequest {
        set: set_dir.clone(),
        change_set: change_report.journal_entry.expect("journal id"),
        dry_run: false,
    });
    assert!(undo_reply.ok, "{:?}", undo_reply.error);
    assert_eq!(fs::read_to_string(&tpl_path).unwrap(), tpl_before);
}

#[test]
fn revise_original_dwg_with_set_page_setup_stb_and_transparency_has_zero_unexpected_changes() {
    let dir = scratch("revise-orig");
    let yaml = "\
units: mm
layout:
  name: Sheet1
paper: ISO A3 landscape
layers:
  walls: {color: 1}
objects:
  - line: {from: [10, 10, 0], to: [100, 10, 0], layer: walls, id: wall-1}
  - line: {from: [10, 20, 0], to: [100, 20, 0], layer: walls, space: Sheet1, id: border-1}
";
    let sheet_path = dir.join("orig.yaml");
    let built_dwg = dir.join("built.dwg");
    fs::write(&sheet_path, yaml).unwrap();
    let settings = no_sys_settings(None);
    build_with(
        &BuildRequest {
            description: sheet_path,
            output: Some(built_dwg.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build base DWG");

    let rev_dwg = dir.join("built-rev1.dwg");
    let reply = revise_with(
        &ReviseRequest {
            set: built_dwg.clone(),
            change_set: serde_json::json!({
                "operations": [
                    {
                        "op": "change_layer",
                        "name": "walls",
                        "set": {
                            "plot_style": "Heavy",
                            "transparency": 45
                        }
                    },
                    {
                        "op": "change",
                        "target": { "id": "orig/wall-1" },
                        "set": {
                            "plot_style": "Accent",
                            "transparency": 25
                        }
                    },
                    {
                        "op": "set_page_setup",
                        "layout": "Sheet1",
                        "set": {
                            "paper": "ISO A4 landscape",
                            "area": "layout",
                            "scale": "1:1",
                            "plot_style": "plant.stb",
                            "plot_transparency": true
                        }
                    }
                ]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(reply.ok, "{:?}", reply.error);
    let report = reply.result.expect("expected revision report");
    assert!(
        report.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        report.unexpected_changes
    );

    // Re-read the revised DWG and confirm every change persisted cleanly!
    let read = read_dwg(&rev_dwg).expect("read revised DWG");
    assert_eq!(read.plot_style_mode, PlotStyleMode::Named);
    let walls = read.layers.iter().find(|l| l.name == "walls").unwrap();
    assert_eq!(walls.plot_style.as_deref(), Some("Heavy"));
    assert_eq!(walls.transparency, 45);
    assert_eq!(
        read.objects[0].properties().plot_style,
        ObjectPlotStyle::Named("Accent".to_owned())
    );
    assert_eq!(
        read.objects[0].properties().transparency,
        ObjectTransparency::Percent(25)
    );
    let layout = read.layout.as_ref().expect("Sheet1 layout");
    let ps = layout.page_setup.as_ref().expect("Sheet1 page_setup");
    assert_eq!(ps.plot_style.as_deref(), Some("plant.stb"));
    assert!(ps.plot_transparency);
    assert_eq!(ps.area, Some(PlotArea::Layout));
    let p = ps.paper.as_ref().expect("ISO A4 paper");
    assert!((p.width - 297.0).abs() < 0.5);
    assert!((p.height - 210.0).abs() < 0.5);
}
