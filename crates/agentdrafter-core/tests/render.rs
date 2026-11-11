//! Rendering a sheet to a PNG: every object type a sheet description can
//! state shows in the pixels, in its layer's colour, pen and linetype; the
//! PNG carries a stamp of the record it came from, and a PNG whose record
//! has moved on is reported as stale.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, check_stale, render_with, BuildRequest, ErrorCode, RenderReport, RenderRequest,
    Settings, SourceKind, StaleRequest,
};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-render")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A decoded PNG and where the drawing sits on it.
struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    report: RenderReport,
}

impl Image {
    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    /// The pixel at the drawing's point `(x, y)`.
    fn at(&self, x: f64, y: f64) -> (usize, usize) {
        let [left, top] = self.report.top_left;
        let k = self.report.pixels_per_unit;
        (((x - left) * k) as usize, ((top - y) * k) as usize)
    }

    /// How many pixels in the drawing's box from `(x0, y0)` to `(x1, y1)`
    /// are close to `color`.
    fn count(&self, (x0, y0): (f64, f64), (x1, y1): (f64, f64), color: [u8; 3]) -> usize {
        let (a, b) = (self.at(x0, y1), self.at(x1, y0));
        let mut n = 0;
        for y in a.1..b.1.min(self.height) {
            for x in a.0..b.0.min(self.width) {
                let p = self.pixel(x, y);
                if p.iter().zip(color).all(|(&c, w)| c.abs_diff(w) < 60) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Whether the pixel is drawn on, not paper.
    fn inked(&self, x: usize, y: usize) -> bool {
        self.pixel(x, y).iter().any(|&c| c < 200)
    }
}

fn render(dir: &Path, description: &str, request: impl FnOnce(&mut RenderRequest)) -> Image {
    let source = dir.join("sheet.yaml");
    fs::write(&source, description).unwrap();
    let mut req = RenderRequest::new(&source);
    request(&mut req);
    let report = render_with(&req, &Settings::default()).unwrap_or_else(|e| panic!("{e}"));
    decode(&PathBuf::from(&report.png), report)
}

fn decode(path: &Path, report: RenderReport) -> Image {
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgb).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    rgb.truncate(info.buffer_size());
    Image {
        width: info.width as usize,
        height: info.height as usize,
        rgb,
        report,
    }
}

/// Each object type in a cell of its own, 30 units square, on a layer of
/// its own colour.
const EVERY_TYPE: &str = r#"
units: mm
linetypes:
  DASHED: {pattern: [4, -4]}
hatch_patterns:
  GRID:
    lines:
      - {angle: 0, delta: [0, 3]}
      - {angle: 90, delta: [0, 3]}
text_styles:
  notes: {font: romans.shx}
layers:
  line: {color: red, lineweight: 0.5}
  polyline: {color: green, lineweight: 0.5}
  arc: {color: blue, lineweight: 0.5}
  circle: {color: magenta, lineweight: 0.5}
  ellipse: {color: 30, lineweight: 0.5}
  spline: {color: 150, lineweight: 0.5}
  solid: {color: cyan}
  pattern: {color: 210, lineweight: 0.35}
  text: {color: 40, lineweight: 0.5}
  mtext: {color: 1, lineweight: 0.5}
objects:
  - line: {from: [2, 2, 0], to: [28, 28, 0], layer: line}
  - polyline: {points: [[32, 2, 0], [58, 2, 0], [58, 28, 0]], layer: polyline}
  - arc: {center: [75, 15, 0], radius: 12, start_angle: 0, end_angle: 180, layer: arc}
  - circle: {center: [105, 15, 0], radius: 12, layer: circle}
  - ellipse: {center: [135, 15, 0], major_axis: [12, 0, 0], ratio: 0.5, layer: ellipse}
  - spline: {control_points: [[152, 2, 0], [160, 28, 0], [170, 2, 0], [178, 28, 0]], degree: 3, layer: spline}
  - hatch:
      pattern: SOLID
      boundary: [{polyline: {points: [[2, 32, 0], [28, 32, 0], [28, 58, 0], [2, 58, 0]]}}]
      layer: solid
  - hatch:
      pattern: GRID
      boundary: [{polyline: {points: [[32, 32, 0], [58, 32, 0], [58, 58, 0], [32, 58, 0]]}}]
      layer: pattern
  - text: {contents: HELLO, position: [62, 38, 0], height: 10, style: notes, layer: text}
  - mtext: {contents: "AB\nCD", position: [122, 58, 0], height: 8, width: 30, layer: mtext}
"#;

#[test]
fn every_object_type_shows_in_its_layers_colour() {
    let dir = scratch("every-type");
    let image = render(&dir, EVERY_TYPE, |r| r.dpi = 200.0);
    let cell = |column: f64, row: f64| {
        (
            (column * 30.0, row * 30.0),
            (column * 30.0 + 30.0, row * 30.0 + 30.0),
        )
    };
    let cases: [(&str, (f64, f64), [u8; 3]); 10] = [
        ("line", (0.0, 0.0), [255, 0, 0]),
        ("polyline", (1.0, 0.0), [0, 255, 0]),
        ("arc", (2.0, 0.0), [0, 0, 255]),
        ("circle", (3.0, 0.0), [255, 0, 255]),
        ("ellipse", (4.0, 0.0), [255, 127, 0]),
        ("spline", (5.0, 0.0), [0, 127, 255]),
        ("solid hatch", (0.0, 1.0), [0, 255, 255]),
        ("patterned hatch", (1.0, 1.0), [255, 0, 255]),
        ("text", (2.0, 1.0), [255, 191, 0]),
        ("mtext", (4.0, 1.0), [255, 0, 0]),
    ];
    for (what, (column, row), color) in cases {
        let (from, to) = cell(column, row);
        let n = image.count(from, to, color);
        assert!(n > 100, "the {what} shows in {n} pixels of its colour");
    }
    assert_eq!(image.report.objects.values().sum::<usize>(), 10);
    // The solid hatch fills its square; the patterned one leaves gaps.
    let solid = image.count((4.0, 34.0), (26.0, 56.0), [0, 255, 255]);
    let pattern = image.count((34.0, 34.0), (56.0, 56.0), [255, 0, 255]);
    let square = image.count((4.0, 34.0), (26.0, 56.0), [255, 255, 255]) + solid;
    assert_eq!(solid, square, "the solid hatch fills every pixel");
    assert!(
        pattern > square / 20 && pattern < square / 2,
        "{pattern} of {square}"
    );
}

#[test]
fn lineweights_draw_as_widths_on_paper() {
    let dir = scratch("lineweights");
    // 254 dots per inch is 10 pixels a millimetre.
    let image = render(
        &dir,
        "units: mm\nlayers:\n  thin: {lineweight: 0.25}\n  thick: {lineweight: 1.0}\n  hair: {lineweight: 0}\nobjects:\n  - line: {from: [0, 0, 0], to: [0, 20, 0], layer: thin}\n  - line: {from: [10, 0, 0], to: [10, 20, 0], layer: thick}\n  - line: {from: [20, 0, 0], to: [20, 20, 0], layer: hair}\n",
        |r| r.dpi = 254.0,
    );
    let width = |x: f64| {
        let (cx, cy) = image.at(x, 10.0);
        (cx.saturating_sub(20)..cx + 20)
            .map(|px| 255 - u32::from(image.pixel(px, cy)[0]))
            .sum::<u32>() as f64
            / 255.0
    };
    assert!((width(0.0) - 2.5).abs() < 0.3, "{}", width(0.0));
    assert!((width(10.0) - 10.0).abs() < 0.3, "{}", width(10.0));
    // Lineweight 0 is the finest line: one pixel.
    assert!((width(20.0) - 1.0).abs() < 0.3, "{}", width(20.0));
}

#[test]
fn a_linetype_leaves_gaps_where_autocad_does() {
    let dir = scratch("linetype");
    // 20 long in 10 dash, 5 gap: dashes from 0 to 7.5 and 12.5 to 20.
    let image = render(
        &dir,
        "units: mm\nlinetypes:\n  DASH: {pattern: [10, -5]}\nlayers:\n  dashed: {linetype: DASH, lineweight: 0.25}\nobjects:\n  - line: {from: [0, 0, 0], to: [20, 0, 0], layer: dashed}\n",
        |r| r.dpi = 254.0,
    );
    for (x, inked) in [
        (3.0, true),
        (7.0, true),
        (10.0, false),
        (12.0, false),
        (13.0, true),
        (19.0, true),
    ] {
        let (px, py) = image.at(x, 0.0);
        assert_eq!(image.inked(px, py), inked, "at {x}");
    }
}

#[test]
fn text_draws_upright() {
    let dir = scratch("upright");
    // The look-alike's T: a bar across the top of the capitals, a stem down.
    let image = render(
        &dir,
        "units: mm\nobjects:\n  - text: {contents: T, position: [0, 0, 0], height: 20}\n",
        |r| {
            r.scale = Some(1.0);
            r.dpi = 254.0;
        },
    );
    let inked_across = |y: f64| {
        let (_, py) = image.at(0.0, y);
        (0..image.width).filter(|&px| image.inked(px, py)).count()
    };
    assert!(
        inked_across(20.0) > 3 * inked_across(10.0),
        "the bar is at the top"
    );
}

#[test]
fn a_render_reports_where_the_drawing_sits_and_at_what_scale() {
    let dir = scratch("scale");
    let one_line = "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 50, 0]}\n";
    let image = render(&dir, one_line, |r| {
        r.scale = Some(1.0);
        r.dpi = 25.4;
        r.margin = 0.0;
    });
    // 1:1 at 25.4 dots per inch is a pixel a millimetre.
    assert_eq!((image.width, image.height), (100, 50));
    assert_eq!(image.report.pixels_per_unit, 1.0);
    assert_eq!(image.report.scale, 1.0);
    let image = render(&dir, one_line, |r| {
        r.width = Some(400);
        r.overwrite = true;
    });
    assert_eq!(image.width, 400);
    assert!(
        (image.report.scale - (400.0 - 2.0 * 5.0 * 150.0 / 25.4) / 100.0 / (150.0 / 25.4)).abs()
            < 1e-9
    );
    // A drawing in inches at 1:1 is as long on paper as it says.
    let inches = "units: in\nobjects:\n  - line: {from: [0, 0, 0], to: [2, 0, 0]}\n";
    let image = render(&dir, inches, |r| {
        r.scale = Some(1.0);
        r.dpi = 100.0;
        r.margin = 0.0;
        r.overwrite = true;
    });
    assert_eq!(image.width, 200);
    // Without a scale or a size, the longer side is 2000 pixels.
    let image = render(&dir, one_line, |r| r.overwrite = true);
    assert_eq!(image.width, 2000);
}

#[test]
fn a_png_is_stamped_and_reported_stale_when_its_record_moves_on() {
    let dir = scratch("stale");
    let image = render(
        &dir,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
        |_| (),
    );
    let stamp = &image.report.stamp;
    assert_eq!(stamp.source.kind, SourceKind::SheetDescription);
    assert_eq!(stamp.source.path, "sheet.yaml");
    assert_eq!(stamp.source.sha256.len(), 64);
    assert!(stamp.produced_by.starts_with("agentdrafter "));
    let png = dir.join("sheet.png");
    let check = || {
        check_stale(&StaleRequest {
            output: png.clone(),
            source: None,
        })
        .unwrap()
    };
    let fresh = check();
    assert!(!fresh.stale, "{fresh:?}");
    assert_eq!(&fresh.stamp, stamp);

    fs::write(dir.join("sheet.yaml"), "units: mm\nobjects: []\n").unwrap();
    let moved_on = check();
    assert!(moved_on.stale, "{moved_on:?}");
    assert!(moved_on.why.contains("has changed"), "{moved_on:?}");

    fs::remove_file(dir.join("sheet.yaml")).unwrap();
    let gone = check();
    assert!(gone.stale, "{gone:?}");
    assert!(gone.why.contains("cannot be read"), "{gone:?}");

    // Moved together, the PNG still finds its sheet description.
    let moved = scratch("stale-moved");
    let image = render(&moved, "units: mm\nobjects: []\n", |_| ());
    let elsewhere = scratch("stale-elsewhere");
    fs::rename(moved.join("sheet.yaml"), elsewhere.join("sheet.yaml")).unwrap();
    fs::rename(&image.report.png, elsewhere.join("sheet.png")).unwrap();
    let report = check_stale(&StaleRequest {
        output: elsewhere.join("sheet.png"),
        source: None,
    })
    .unwrap();
    assert!(!report.stale, "{report:?}");
}

#[test]
fn a_file_without_a_stamp_is_not_an_output() {
    let dir = scratch("unstamped");
    fs::write(dir.join("photo.png"), b"\x89PNG\r\n\x1a\nnot really").unwrap();
    let error = check_stale(&StaleRequest {
        output: dir.join("photo.png"),
        source: None,
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotStamped, "{error:?}");
}

#[test]
fn an_existing_file_is_replaced_only_when_asked_and_only_if_rendered() {
    let dir = scratch("overwrite");
    let source = dir.join("sheet.yaml");
    fs::write(&source, "units: mm\nobjects: []\n").unwrap();
    let mut request = RenderRequest::new(&source);
    render_with(&request, &Settings::default()).unwrap();
    let error = render_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputExists, "{error:?}");
    request.overwrite = true;
    render_with(&request, &Settings::default()).unwrap();

    let theirs = dir.join("theirs.png");
    fs::write(&theirs, b"someone's picture").unwrap();
    request.output = Some(theirs.clone());
    let error = render_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput, "{error:?}");
    assert_eq!(fs::read(&theirs).unwrap(), b"someone's picture");

    request.output = Some(source.clone());
    let error = render_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
}

#[test]
fn a_request_that_cannot_be_rendered_names_its_field() {
    let dir = scratch("bad-request");
    let source = dir.join("sheet.yaml");
    fs::write(
        &source,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    type Change = fn(&mut RenderRequest);
    let cases: [(Change, ErrorCode, Option<&str>); 5] = [
        (|r| r.dpi = 0.0, ErrorCode::BadValue, Some("dpi")),
        (|r| r.margin = -1.0, ErrorCode::BadValue, Some("margin")),
        (|r| r.scale = Some(0.0), ErrorCode::BadValue, Some("scale")),
        (
            |r| {
                r.scale = Some(1.0);
                r.width = Some(100);
            },
            ErrorCode::Usage,
            None,
        ),
        (|r| r.scale = Some(1e6), ErrorCode::BadValue, None),
    ];
    for (change, code, field) in cases {
        let mut request = RenderRequest::new(&source);
        change(&mut request);
        let error = render_with(&request, &Settings::default()).unwrap_err();
        assert_eq!(error.code, code, "{error:?}");
        assert_eq!(error.field.as_deref(), field, "{error:?}");
    }
    assert!(!dir.join("sheet.png").exists());
}

#[test]
fn a_dwg_renders_as_the_sheet_description_it_was_built_from() {
    let dir = scratch("dwg");
    let description = dir.join("sheet.yaml");
    fs::write(&description, EVERY_TYPE).unwrap();
    build_with(
        &BuildRequest {
            description: description.clone(),
            output: None,
            overwrite: false,
        },
        &Settings::default(),
    )
    .unwrap();
    let from_description = {
        let mut r = RenderRequest::new(&description);
        r.output = Some(dir.join("a.png"));
        let report = render_with(&r, &Settings::default()).unwrap();
        decode(&dir.join("a.png"), report)
    };
    let from_dwg = {
        let mut r = RenderRequest::new(dir.join("sheet.dwg"));
        r.output = Some(dir.join("b.png"));
        let report = render_with(&r, &Settings::default()).unwrap();
        decode(&dir.join("b.png"), report)
    };
    assert_eq!(from_dwg.report.stamp.source.kind, SourceKind::Dwg);
    assert_eq!(from_dwg.report.objects_not_drawn, 0);
    assert_eq!(from_dwg.report.objects, from_description.report.objects);
    assert_eq!(
        (from_dwg.width, from_dwg.height),
        (from_description.width, from_description.height)
    );
    let differ = from_dwg
        .rgb
        .chunks(3)
        .zip(from_description.rgb.chunks(3))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 64))
        .count();
    assert!(
        differ * 1000 < from_dwg.width * from_dwg.height,
        "{differ} pixels differ"
    );
}

/// A made-up block library: a square on layer 0 with a visible attribute
/// in its middle and an invisible one below it.
const BLOCK_LIBRARY: &str = r#"
units: mm
layers:
  tags: {color: green, lineweight: 0.5}
  hidden: {color: blue, lineweight: 0.5}
blocks:
  MARK:
    objects:
      - polyline: {points: [[-10, -10, 0], [10, -10, 0], [10, 10, 0], [-10, 10, 0]], closed: true}
      - attdef: {tag: TAG, position: [0, 0, 0], height: 8, justify: middle_center, layer: tags}
      - attdef: {tag: NOTE, position: [0, -20, 0], height: 8, justify: middle_center, invisible: true, layer: hidden}
"#;

/// Two inserts of the square on a red layer, the second halved and turned
/// 45 degrees.
const INSERTS: &str = r#"
units: mm
block_library: blocks.yaml
layers:
  marks: {color: red, lineweight: 0.5}
objects:
  - insert: {block: MARK, position: [30, 30, 0], attributes: {TAG: AB, NOTE: HIDDEN}, layer: marks}
  - insert: {block: MARK, position: [80, 30, 0], scale: 0.5, rotation: 45, attributes: {TAG: CD, NOTE: HIDDEN}, layer: marks}
  - line: {from: [0, 0, 0], to: [110, 0, 0]}
"#;

#[test]
fn an_insert_and_its_visible_attributes_show_and_its_invisible_ones_do_not() {
    let dir = scratch("inserts");
    fs::write(dir.join("blocks.yaml"), BLOCK_LIBRARY).unwrap();
    let image = render(&dir, INSERTS, |r| r.dpi = 200.0);
    let red = [255, 0, 0];
    let green = [0, 255, 0];
    // The square, on layer 0, draws in the insert's layer's red.
    assert!(image.count((18.0, 18.0), (42.0, 42.0), red) > 300);
    assert!(
        image.count((19.0, 19.0), (21.0, 41.0), red) > 50,
        "its left side"
    );
    // The visible attribute says AB in the square's middle, in green.
    assert!(image.count((22.0, 25.0), (38.0, 35.0), green) > 50);
    // The invisible one draws nothing anywhere.
    assert_eq!(image.count((0.0, -5.0), (110.0, 45.0), [0, 0, 255]), 0);
    // The second square is halved and turned into a diamond: its sides
    // pass through (76.5, 33.5), and no corner sits at (75, 35).
    assert!(image.count((75.5, 32.5), (77.5, 34.5), red) > 10);
    assert_eq!(image.count((74.0, 34.5), (75.5, 36.0), red), 0);
    assert!(image.count((76.0, 27.0), (84.0, 33.0), green) > 10);
    assert_eq!(image.report.objects.get("insert"), Some(&2));
}

#[test]
fn a_dwg_with_inserts_renders_as_its_sheet_description() {
    let dir = scratch("inserts-dwg");
    fs::write(dir.join("blocks.yaml"), BLOCK_LIBRARY).unwrap();
    let description = dir.join("sheet.yaml");
    fs::write(&description, INSERTS).unwrap();
    build_with(
        &BuildRequest {
            description: description.clone(),
            output: None,
            overwrite: false,
        },
        &Settings::default(),
    )
    .unwrap();
    let render_to = |source: PathBuf, png: &str| {
        let mut r = RenderRequest::new(source);
        r.output = Some(dir.join(png));
        let report = render_with(&r, &Settings::default()).unwrap();
        decode(&dir.join(png), report)
    };
    let from_description = render_to(description.clone(), "a.png");
    let from_dwg = render_to(dir.join("sheet.dwg"), "b.png");
    assert_eq!(from_dwg.report.objects_not_drawn, 0);
    assert_eq!(from_dwg.report.objects, from_description.report.objects);
    assert_eq!(
        (from_dwg.width, from_dwg.height),
        (from_description.width, from_description.height)
    );
    let differ = from_dwg
        .rgb
        .chunks(3)
        .zip(from_description.rgb.chunks(3))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 64))
        .count();
    assert!(
        differ * 1000 < from_dwg.width * from_dwg.height,
        "{differ} pixels differ"
    );
}
