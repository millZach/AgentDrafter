//! End-to-end tests for 24-bit object true colour (#106):
//! sheet description, schema, DWG round-trip, ByBlock inheritance, MTEXT `\c`,
//! display list, PNG render, PDF plot through CTB & STB, and `change` on
//! originals.
//!
//! All drawings and plot styles in these tests are made up (ADR-0008).

#[path = "support/ctb.rs"]
mod ctb;

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, display_list_with, plot_with, read_dwg, read_original, render_with, revise_with,
    sheet_description_schema, BuildRequest, DisplayListRequest, ObjectColor, PlotRequest,
    ReadRequest, RenderRequest, ReviseRequest, Settings,
};
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-true-color")
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
/// rendered PDF commands (colors, lineweights).
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

fn has_stroke_rgb(pdf_text: &str, rgb: [u8; 3]) -> bool {
    let target = rgb.map(|c| f64::from(c) / 255.0);
    pdf_text.lines().any(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 4 && parts[3] == "RG" {
            let parsed: Option<[f64; 3]> = (|| {
                Some([
                    parts[0].parse().ok()?,
                    parts[1].parse().ok()?,
                    parts[2].parse().ok()?,
                ])
            })();
            if let Some([r, g, b]) = parsed {
                return (r - target[0]).abs() < 0.002
                    && (g - target[1]).abs() < 0.002
                    && (b - target[2]).abs() < 0.002;
            }
        }
        false
    })
}

#[test]
fn true_colour_round_trips_dwg_renders_png_and_plots_ctb_and_stb() {
    let dir = scratch("e2e");
    let lib_path = dir.join("blocks.yaml");
    fs::write(
        &lib_path,
        "\
units: mm
blocks:
  MARKER:
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0], color: ByBlock}
",
    )
    .unwrap();

    let sheet_path = dir.join("sheet.yaml");
    let yaml = "\
units: mm
block_library: blocks.yaml
layers:
  WIRE: {color: red}
objects:
  - line: {id: orange-line, from: [0, 0, 0], to: [60, 0, 0], layer: WIRE, color: [255, 128, 0], lineweight: 0.7}
  - line: {id: red-line, from: [0, 20, 0], to: [60, 20, 0], layer: WIRE, lineweight: 0.35}
  - insert: {id: teal-marker, block: MARKER, position: [0, 40, 0], color: [10, 200, 150]}
  - mtext:
      id: rich-orange
      position: [0, 55, 0]
      height: 5
      runs:
        - {text: 'ORANGE', color: [255, 128, 0]}
";
    fs::write(&sheet_path, yaml).unwrap();

    // Schema accepts `[r, g, b]` on object `color`.
    let schema = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let parsed_val = json!({
        "units": "mm",
        "objects": [
            { "line": { "from": [0, 0, 0], "to": [10, 0, 0], "color": [255, 128, 0] } }
        ]
    });
    assert!(
        validator.is_valid(&parsed_val),
        "schema should accept `[255, 128, 0]` on object color"
    );

    let settings = no_sys_settings(None);

    // 1. Build to DWG and read back as an original: zero unmodelled entities,
    //    true colours preserved verbatim.
    let dwg_path = dir.join("sheet.dwg");
    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();

    let contents = read_dwg(&dwg_path).unwrap();
    assert_eq!(
        contents.objects[0].properties().color,
        ObjectColor::Rgb([255, 128, 0])
    );
    assert_eq!(
        contents.objects[2].properties().color,
        ObjectColor::Rgb([10, 200, 150])
    );

    let mirror_path = dir.join("sheet.mirror.yaml");
    let orig = read_original(&ReadRequest {
        source: dwg_path.clone(),
        mirror: Some(mirror_path.clone()),
        no_mirror: false,
    })
    .unwrap();
    assert!(
        orig.not_read.is_empty(),
        "expected all true-colour objects to be modelled, got {:?}",
        orig.not_read
    );
    let mirror = fs::read_to_string(&mirror_path).unwrap();
    assert!(
        mirror.contains("[255, 128, 0]"),
        "mirror should record [255, 128, 0]: {mirror}"
    );
    assert!(
        mirror.contains("[10, 200, 150]"),
        "mirror should record [10, 200, 150]: {mirror}"
    );

    // 2. Display list carries `true_color` on the true-colour line, the ByBlock
    //    insert child, and the MTEXT inline run, and leaves `None` on the ACI line.
    let dl = display_list_with(&DisplayListRequest::new(&sheet_path), &settings).unwrap();
    let item0 = dl
        .display_list
        .items
        .iter()
        .find(|i| i.object == 0)
        .unwrap();
    assert_eq!(item0.true_color, Some([255, 128, 0]));
    let item1 = dl
        .display_list
        .items
        .iter()
        .find(|i| i.object == 1)
        .unwrap();
    assert_eq!(item1.true_color, None);
    let item2 = dl
        .display_list
        .items
        .iter()
        .find(|i| i.object == 2)
        .unwrap();
    assert_eq!(item2.true_color, Some([10, 200, 150]));
    let item3 = dl
        .display_list
        .items
        .iter()
        .find(|i| i.object == 3)
        .unwrap();
    assert_eq!(item3.true_color, Some([255, 128, 0]));

    // 3. Render PNG: contains exact orange pixels `[255, 128, 0]`.
    let png_path = dir.join("sheet.png");
    let mut render_req = RenderRequest::new(&sheet_path);
    render_req.output = Some(png_path.clone());
    render_req.overwrite = true;
    render_with(&render_req, &settings).unwrap();
    let (_w, _h, rgba) = decode_png_rgba(&png_path);
    let has_orange = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] >= 245 && (118..=138).contains(&p[1]) && p[2] <= 10 && p[3] == 255);
    assert!(
        has_orange,
        "rendered PNG should contain orange [255, 128, 0]"
    );

    // 4. Plot PDF through built-in monochrome CTB:
    //    - True-colour line bypasses CTB (plots orange RGB and keeps 0.7 mm width).
    //    - Indexed red line follows monochrome CTB (plots black RGB).
    let pdf_path = dir.join("sheet_mono.pdf");
    let mut plot_req = PlotRequest::new(&sheet_path);
    plot_req.output = Some(pdf_path.clone());
    plot_req.overwrite = true;
    plot_with(&plot_req, &settings).unwrap();
    let pdf_text = pdf_streams_text(&fs::read(&pdf_path).unwrap());
    assert!(
        has_stroke_rgb(&pdf_text, [255, 128, 0]),
        "monochrome CTB PDF should preserve true colour orange [255, 128, 0]"
    );
    assert!(
        has_stroke_rgb(&pdf_text, [10, 200, 150]),
        "monochrome CTB PDF should preserve ByBlock true colour teal [10, 200, 150]"
    );
    assert!(
        has_stroke_rgb(&pdf_text, [0, 0, 0]),
        "monochrome CTB PDF should still plot indexed red line as black"
    );

    // 5. Plot PDF through an STB where `Heavy` forces black:
    //    an object with true colour + `plot_style: Heavy` plots black under STB,
    //    while `Normal` preserves its true colour `[10, 200, 150]`.
    let stb_dir = dir.join("styles");
    fs::create_dir_all(&stb_dir).unwrap();
    // -1073741824 is 0xC000_0000 (RGB(0, 0, 0)) as AutoCAD STB encodes black.
    let stb_bytes = ctb::made_up_stb(&[("Heavy", -1073741824, 21, 100)]);
    fs::write(stb_dir.join("test.stb"), &stb_bytes).unwrap();
    let stb_sheet = dir.join("stb_sheet.yaml");
    fs::write(
        &stb_sheet,
        "\
units: mm
objects:
  - line: {from: [0, 0, 0], to: [40, 0, 0], color: [255, 128, 0], plot_style: Heavy}
  - line: {from: [0, 10, 0], to: [40, 10, 0], color: [10, 200, 150], plot_style: Normal}
",
    )
    .unwrap();
    let stb_pdf = dir.join("stb_sheet.pdf");
    let mut stb_plot = PlotRequest::new(&stb_sheet);
    stb_plot.output = Some(stb_pdf.clone());
    stb_plot.overwrite = true;
    stb_plot.plot_style = Some("test.stb".to_owned());
    let stb_settings = no_sys_settings(Some(&stb_dir));
    plot_with(&stb_plot, &stb_settings).unwrap();
    let stb_text = pdf_streams_text(&fs::read(&stb_pdf).unwrap());
    assert!(
        !has_stroke_rgb(&stb_text, [255, 128, 0]),
        "STB Heavy style should override true-colour orange to black"
    );
    assert!(
        has_stroke_rgb(&stb_text, [0, 0, 0]),
        "STB Heavy style should emit black stroke"
    );
    assert!(
        has_stroke_rgb(&stb_text, [10, 200, 150]),
        "STB Normal style should preserve true-colour teal [10, 200, 150]"
    );

    // 6. `change` on an original DWG sets a true colour and reads it back.
    let rev_dwg = dir.join("sheet.rev1.dwg");
    let reply = revise_with(
        &ReviseRequest {
            set: dwg_path.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "change",
                        "target": { "id": "sheet/orange-line" },
                        "set": { "color": [40, 180, 90] }
                    }
                ]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(reply.ok, "{:?}", reply.error);
    let rev_contents = read_dwg(&rev_dwg).unwrap();
    assert_eq!(
        rev_contents.objects[0].properties().color,
        ObjectColor::Rgb([40, 180, 90])
    );
}
