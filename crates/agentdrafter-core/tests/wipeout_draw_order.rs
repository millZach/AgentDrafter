//! End-to-end tests for wipeouts (`WIPEOUT`, `wipeout_frame`) and draw order
//! (`DRAWORDER` / `DR`, `TEXTTOFRONT`, optional `draw_order` on `add_*`).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, build_with, change_with, check_with, display_list_with,
    parse_sheet_description, plot_with, query_with, read_dwg, render_with, revise_with,
    sheet_description_schema, undo, Answer, BuildRequest, ChangeRequest, CheckRequest,
    DisplayListRequest, Object, PlotRequest, Query, QueryRequest, RenderRequest, ReviseRequest,
    Rule, Selection, SetBuildRequest, Settings, Shape, UndoRequest, WipeoutFrame,
};
use serde_json::json;

#[path = "support/demo_set.rs"]
mod demo_set;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("wipeout-draw-order")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default()
}

#[test]
fn sheet_description_schema_and_reader_agree_on_wipeout_and_frame() {
    let schema = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let sample = json!({
        "units": "in",
        "wipeout_frame": "display",
        "objects": [
            {
                "wipeout": {
                    "id": "mask-1",
                    "points": [[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]]
                }
            }
        ]
    });
    assert!(validator.is_valid(&sample), "schema must accept wipeout");

    let bad_frame = json!({
        "units": "in",
        "wipeout_frame": "sometimes",
        "objects": []
    });
    assert!(!validator.is_valid(&bad_frame));

    let sheet = parse_sheet_description(
        "\
units: in
wipeout_frame: display
objects:
  - wipeout:
      id: mask-1
      points: [[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]]
",
    )
    .expect("reader accepts wipeout");
    assert_eq!(sheet.wipeout_frame, WipeoutFrame::Display);
    assert_eq!(sheet.objects.len(), 1);
    assert!(matches!(sheet.objects[0], Object::Wipeout(_)));
}

#[test]
fn wipeout_hides_line_behind_it_in_png_and_suppresses_text_on_line_check() {
    let dir = scratch("mask-and-check");
    let settings = no_system_fonts();

    // 1. Without a wipeout, a wire behind text fails `text_on_line`.
    let unmasked_yaml = dir.join("unmasked.yaml");
    fs::write(
        &unmasked_yaml,
        "\
units: in
paper: ANSI A landscape
objects:
  - polyline:
      id: border
      closed: true
      points: [[0.5, 0.5, 0], [10.5, 0.5, 0], [10.5, 8.0, 0], [0.5, 8.0, 0]]
  - line:
      id: wire
      from: [1.0, 4.0, 0]
      to: [10.0, 4.0, 0]
  - text:
      id: label
      contents: \"TAG-101\"
      position: [5.5, 4.0, 0]
      height: 0.3
      justify: middle_center
",
    )
    .unwrap();
    let unmasked_check = check_with(
        &CheckRequest {
            source: unmasked_yaml,
            sheets: Vec::new(),
        },
        &settings,
    )
    .unwrap();
    assert!(
        unmasked_check
            .findings
            .iter()
            .any(|f| f.rule == Rule::TextOnLine),
        "expected text_on_line without wipeout: {:?}",
        unmasked_check.findings
    );

    // 2. With a wipeout drawn after the wire and before the text, `text_on_line`
    // passes AND the PNG pixels inside the wipeout (away from text) are pure white
    // while pixels on the wire outside the wipeout are dark!
    let masked_yaml = dir.join("masked.yaml");
    fs::write(
        &masked_yaml,
        "\
units: in
paper: ANSI A landscape
wipeout_frame: off
layers:
  wire: { color: white, lineweight: 1.0 }
objects:
  - polyline:
      id: border
      closed: true
      points: [[0.0, 0.0, 0], [11.0, 0.0, 0], [11.0, 8.5, 0], [0.0, 8.5, 0]]
  - line:
      id: wire
      layer: wire
      from: [1.0, 4.25, 0]
      to: [10.0, 4.25, 0]
  - wipeout:
      id: mask
      points: [[3.5, 3.5, 0], [7.5, 3.5, 0], [7.5, 5.0, 0], [3.5, 5.0, 0]]
  - text:
      id: label
      contents: \"TAG-101\"
      position: [5.5, 4.6, 0]
      height: 0.25
      justify: middle_center
",
    )
    .unwrap();

    let masked_check = check_with(
        &CheckRequest {
            source: masked_yaml.clone(),
            sheets: Vec::new(),
        },
        &settings,
    )
    .unwrap();
    assert!(
        !masked_check
            .findings
            .iter()
            .any(|f| f.rule == Rule::TextOnLine),
        "wipeout should suppress text_on_line: {:?}",
        masked_check.findings
    );

    // Render to PNG and verify actual pixels:
    // At x=2.0, y=4.25 (on wire, outside wipeout) -> dark ink pixel.
    // At x=5.5, y=4.25 (on wire, INSIDE wipeout, below text at y=4.6) -> pure white [255,255,255]!
    let png_path = dir.join("masked.png");
    let mut render_req = RenderRequest::new(&masked_yaml);
    render_req.output = Some(png_path.clone());
    render_req.monochrome = true;
    render_req.overwrite = true;
    let rendered = render_with(&render_req, &settings).unwrap();

    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(&png_path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgb).unwrap();
    let w = info.width as usize;
    let pixel = |x: usize, y: usize| -> [u8; 3] {
        let i = (y * w + x) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };
    let at = |x: f64, y: f64| -> (usize, usize) {
        let [left, top] = rendered.top_left;
        let k = rendered.pixels_per_unit;
        (((x - left) * k) as usize, ((top - y) * k) as usize)
    };
    let (ox, oy) = at(2.0, 4.25);
    let mut saw_dark_outside = false;
    for dy in -3isize..=3 {
        let p = pixel(ox, (oy as isize + dy) as usize);
        if p[0] < 100 && p[1] < 100 && p[2] < 100 {
            saw_dark_outside = true;
        }
    }
    assert!(
        saw_dark_outside,
        "wire should draw dark outside the wipeout"
    );
    let (ix, iy) = at(5.5, 4.25);
    for dy in -2isize..=2 {
        let p = pixel(ix, (iy as isize + dy) as usize);
        assert_eq!(
            p,
            [255, 255, 255],
            "wire must be hidden (pure white) inside the wipeout"
        );
    }

    // Plot to PDF and check display-list contains Shape::Mask.
    let pdf_path = dir.join("masked.pdf");
    plot_with(
        &PlotRequest {
            source: masked_yaml.clone(),
            output: Some(pdf_path.clone()),
            plot_style: None,
            overwrite: true,
        },
        &settings,
    )
    .unwrap();
    assert!(fs::metadata(&pdf_path).unwrap().len() > 200);

    let dl = display_list_with(&DisplayListRequest::new(&masked_yaml), &settings).unwrap();
    assert!(
        dl.display_list
            .items
            .iter()
            .any(|item| matches!(item.shape, Shape::Mask { .. })),
        "display list must emit Shape::Mask for wipeout"
    );
}

#[test]
fn wipeout_inside_block_hides_geometry_and_round_trips_through_dwg() {
    let dir = scratch("block-wipeout");
    let settings = no_system_fonts();
    fs::write(
        dir.join("blocks.yaml"),
        "\
units: in
blocks:
  BADGE:
    base_point: [0, 0, 0]
    objects:
      - wipeout:
          points: [[-0.6, -0.3, 0], [0.6, -0.3, 0], [0.6, 0.3, 0], [-0.6, 0.3, 0]]
      - polyline:
          closed: true
          points: [[-0.6, -0.3, 0], [0.6, -0.3, 0], [0.6, 0.3, 0], [-0.6, 0.3, 0]]
",
    )
    .unwrap();
    let sheet_yaml = dir.join("sheet.yaml");
    fs::write(
        &sheet_yaml,
        "\
units: in
block_library: blocks.yaml
objects:
  - line:
      id: bus
      from: [0, 2, 0]
      to: [8, 2, 0]
  - insert:
      id: badge-1
      block: BADGE
      position: [4, 2, 0]
",
    )
    .unwrap();
    let dwg_path = dir.join("sheet.dwg");
    build_with(
        &BuildRequest {
            description: sheet_yaml,
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();

    let read = read_dwg(&dwg_path).unwrap();
    let block = read
        .blocks
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("BADGE"))
        .expect("BADGE block present");
    assert!(
        block
            .objects
            .iter()
            .any(|o| matches!(o, Object::Wipeout(_))),
        "BADGE block must retain its wipeout after DWG round-trip"
    );
    let dl = display_list_with(&DisplayListRequest::new(&dwg_path), &settings).unwrap();
    assert!(
        dl.display_list
            .items
            .iter()
            .any(|item| matches!(item.shape, Shape::Mask { .. })),
        "block insert containing wipeout must emit Shape::Mask"
    );
}

#[test]
fn drawing_set_wipeout_draw_order_text_to_front_and_undo() {
    let dir = scratch("set-draw-order");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let settings = no_system_fonts();

    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{:?}", built.error);

    let before_rows = fs::read(set.join("rows.yaml")).unwrap();

    // 1. Add a line, a text, then add a wipeout *under* the text via `draw_order`.
    let r1 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: json!({
                "operations": [
                    { "op": "add_line", "sheet": "DEMO-001", "id": "w-line", "from": [2, 2, 0], "to": [6, 2, 0] },
                    { "op": "add_text", "sheet": "DEMO-001", "id": "w-text", "contents": "HI", "position": [4, 2, 0], "height": 0.25 },
                    {
                        "op": "WIPEOUT",
                        "sheet": "DEMO-001",
                        "id": "w-mask",
                        "points": [[3.5, 1.7, 0], [4.5, 1.7, 0], [4.5, 2.3, 0], [3.5, 2.3, 0]],
                        "draw_order": { "under": { "id": "DEMO-001/w-text" } }
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(r1.ok, "{:?}", r1.error);
    let entry1 = r1.result.unwrap().journal_entry.unwrap();

    let order = |s: &Path| -> Vec<String> {
        let q = query_with(
            &QueryRequest {
                source: s.to_owned(),
                sheet: Some("DEMO-001".to_owned()),
                query: Query::Find(Selection::default()),
            },
            &settings,
        )
        .unwrap();
        let Answer::Find { objects, .. } = q.answer else {
            panic!()
        };
        objects
            .into_iter()
            .map(|o| o.id)
            .filter(|id| id.contains("/w-"))
            .collect()
    };
    assert_eq!(
        order(&set),
        vec!["DEMO-001/w-line", "DEMO-001/w-mask", "DEMO-001/w-text"]
    );

    // 2. Move w-mask to front with `DRAWORDER`, then bring text back to front with `TEXTTOFRONT`!
    let r2 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: json!({
                "operations": [
                    { "op": "DR", "target": { "id": "DEMO-001/w-mask" }, "order": "front" }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(r2.ok, "{:?}", r2.error);
    assert_eq!(
        order(&set),
        vec!["DEMO-001/w-line", "DEMO-001/w-text", "DEMO-001/w-mask"]
    );

    let r3 = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: json!({
                "operations": [
                    { "op": "TEXTTOFRONT", "sheet": "DEMO-001", "which": "text" }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(r3.ok, "{:?}", r3.error);
    assert_eq!(
        order(&set),
        vec!["DEMO-001/w-line", "DEMO-001/w-mask", "DEMO-001/w-text"]
    );

    // 3. Undo back to entry1 restores rows.yaml byte for byte!
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry1,
        dry_run: false,
    });
    assert!(undone.ok, "{:?}", undone.error);
    assert_eq!(fs::read(set.join("rows.yaml")).unwrap(), before_rows);
}

#[test]
fn original_dwg_revise_supports_add_wipeout_draw_order_and_text_to_front() {
    let dir = scratch("original-revise");
    let settings = no_system_fonts();

    let base_yaml = dir.join("base.yaml");
    let base_dwg = dir.join("base.dwg");
    fs::write(
        &base_yaml,
        "\
units: in
paper: ANSI A landscape
objects:
  - line:
      id: wire
      from: [1, 4, 0]
      to: [9, 4, 0]
  - text:
      id: tag
      contents: \"X-10\"
      position: [5, 4, 0]
      height: 0.25
      justify: middle_center
",
    )
    .unwrap();
    build_with(
        &BuildRequest {
            description: base_yaml,
            output: Some(base_dwg.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();

    // Revise the original: add a wipeout UNDER the tag, then test DRAWORDER and TEXTTOFRONT.
    let rev = revise_with(
        &ReviseRequest {
            set: base_dwg.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "add_wipeout",
                        "id": "mask",
                        "points": [[4.0, 3.6, 0], [6.0, 3.6, 0], [6.0, 4.4, 0], [4.0, 4.4, 0]],
                        "draw_order": { "under": { "id": "base/tag" } }
                    }
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
    assert!(rev.ok, "{:?}", rev.error);
    let report = rev.result.unwrap();
    assert!(
        report.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        report.unexpected_changes
    );
    let rev1_dwg = PathBuf::from(report.revision_dwg.unwrap());

    let read1 = read_dwg(&rev1_dwg).unwrap();
    let ids1: Vec<String> = read1
        .ids_and_names
        .iter()
        .filter_map(|i| i.id.clone())
        .collect();
    assert_eq!(ids1, vec!["base/wire", "base/mask", "base/tag"]);

    // Chain a second revision: DRAWORDER mask to front, then TEXTTOFRONT brings tag back on top!
    let rev2 = revise_with(
        &ReviseRequest {
            set: base_dwg,
            change_set: json!({
                "operations": [
                    { "op": "DRAWORDER", "target": { "id": "base/mask" }, "order": "front" },
                    { "op": "TEXTTOFRONT", "which": "text" }
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
    assert!(rev2.ok, "{:?}", rev2.error);
    let report2 = rev2.result.unwrap();
    assert!(
        report2.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        report2.unexpected_changes
    );
    let rev2_dwg = PathBuf::from(report2.revision_dwg.unwrap());
    let read2 = read_dwg(&rev2_dwg).unwrap();
    let ids2: Vec<String> = read2
        .ids_and_names
        .iter()
        .filter_map(|i| i.id.clone())
        .collect();
    assert_eq!(ids2, vec!["base/wire", "base/mask", "base/tag"]);
}
