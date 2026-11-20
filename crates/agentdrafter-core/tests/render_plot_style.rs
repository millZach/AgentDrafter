//! Rendering through a plot style, to show what prints: a PNG drawn
//! through a CTB draws each pen in the colour and lineweight the PDF
//! plotted through the same CTB draws it, monochrome draws exactly as a
//! plot with no plot style named, and the reply says which it drew. With
//! neither, a render draws each object in its own colour, as before.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    check_stale, plot_with, render_with, DrawnThrough, ErrorCode, PlotReport, PlotRequest,
    RenderReport, RenderRequest, Settings, StaleRequest, BUILT_IN_PLOT_STYLE,
};
use serde_json::json;

#[path = "support/ctb.rs"]
mod ctb;
#[path = "support/pdf.rs"]
mod pdf;

use pdf::Pdf;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-render-plot-style")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Four lines 10 mm apart on layers of colours 1, 2, 3 and 4, of
/// lineweights 0.25, 0.5, the default and 1; the last on a layer that does
/// not plot.
const FOUR_PENS: &str = "units: mm
layers:
  red: {color: 1, lineweight: 0.25}
  yellow: {color: 2, lineweight: 0.5}
  green: {color: 3}
  trim: {color: 4, lineweight: 1, plot: false}
objects:
  - line: {from: [0, 0, 0], to: [100, 0, 0], layer: red}
  - line: {from: [0, 10, 0], to: [100, 10, 0], layer: yellow}
  - line: {from: [0, 20, 0], to: [100, 20, 0], layer: green}
  - line: {from: [0, 30, 0], to: [100, 30, 0], layer: trim}
";

/// The heights of the four lines, in drawing units.
const LINES: [f64; 4] = [0.0, 10.0, 20.0, 30.0];

/// Ten pixels to the millimetre, so a lineweight is easy to count.
const DPI: f64 = 254.0;

/// A made-up CTB: pen 1 plots blue at 1 mm, pen 2 its object's own colour
/// and lineweight, pen 3 dark green at 0.5 mm and screened to 50%, which
/// neither the PDF nor the PNG follows yet; every other pen black at its
/// object's own lineweight.
fn made_up_ctb(folder: &Path) {
    let text = ctb::made_up_ctb_text(
        &[
            (1, ctb::rgb(0, 0, 255), 21),
            (2, ctb::OBJECT_COLOR, 0),
            (3, ctb::rgb(0, 128, 0), 14),
        ],
        "",
    );
    // Pen 3 is the plot style's section 2.
    let at = text.find("\n 2{\n").unwrap();
    let screen = at + text[at..].find("screen=100").unwrap();
    let text = format!(
        "{}screen=50{}",
        &text[..screen],
        &text[screen + "screen=100".len()..]
    );
    fs::write(folder.join("Made-Up.ctb"), ctb::ctb_file(&text)).unwrap();
}

/// The sheet and the settings that find the made-up CTB, in `dir`.
fn sheet_and_styles(dir: &Path) -> (PathBuf, Settings) {
    let styles = dir.join("plot styles");
    fs::create_dir_all(&styles).unwrap();
    made_up_ctb(&styles);
    let source = dir.join("sheet.yaml");
    fs::write(&source, FOUR_PENS).unwrap();
    let settings = Settings {
        plot_style_folders: vec![styles],
        ..Settings::default()
    };
    (source, settings)
}

/// A decoded PNG and where the drawing sits on it.
struct Image {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    report: RenderReport,
}

impl Image {
    fn read(report: RenderReport) -> Image {
        let file = fs::File::open(&report.png).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
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

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    /// The pen a horizontal line at height `y` draws with, read across it
    /// halfway along: the colour of its most inked pixel, and its width in
    /// millimetres of paper, the ink summed down the column, so that the
    /// pixels anti-aliasing half fills count as half. None when it draws
    /// nothing.
    fn pen(&self, y: f64) -> Option<([u8; 3], f64)> {
        let [left, top] = self.report.top_left;
        let k = self.report.pixels_per_unit;
        let x = ((50.0 - left) * k) as usize;
        let centre = ((top - y) * k) as usize;
        let rows = centre.saturating_sub(30)..(centre + 30).min(self.height);
        let ink = |p: [u8; 3]| p.iter().map(|&c| 255 - u32::from(c)).max().unwrap();
        let darkest = rows
            .clone()
            .map(|row| self.pixel(x, row))
            .max_by_key(|&p| ink(p))?;
        if ink(darkest) == 0 {
            return None;
        }
        let full = f64::from(ink(darkest));
        let covered: f64 = rows
            .map(|row| f64::from(ink(self.pixel(x, row))) / full)
            .sum();
        Some((darkest, covered / (DPI / 25.4)))
    }
}

fn render(source: &Path, settings: &Settings, ask: impl FnOnce(&mut RenderRequest)) -> Image {
    let mut request = RenderRequest {
        dpi: DPI,
        scale: Some(1.0),
        overwrite: true,
        ..RenderRequest::new(source)
    };
    ask(&mut request);
    Image::read(render_with(&request, settings).unwrap_or_else(|e| panic!("{e}")))
}

fn plot(source: &Path, settings: &Settings, plot_style: Option<&str>) -> (PlotReport, Pdf) {
    let request = PlotRequest {
        overwrite: true,
        plot_style: plot_style.map(str::to_owned),
        ..PlotRequest::new(source)
    };
    let report = plot_with(&request, settings).unwrap_or_else(|e| panic!("{e}"));
    let pdf = Pdf::read(fs::read(&report.pdf).unwrap());
    (report, pdf)
}

/// The PDF's pens, each its colour in bytes and its width in millimetres,
/// in the order the lines are drawn.
fn plotted_pens(pdf: &Pdf) -> Vec<([u8; 3], f64)> {
    pdf.strokes()
        .iter()
        .map(|pen| {
            let byte = |c: f64| (c * 255.0).round() as u8;
            (
                [byte(pen.rgb[0]), byte(pen.rgb[1]), byte(pen.rgb[2])],
                pen.width / 72.0 * 25.4,
            )
        })
        .collect()
}

/// Whether the PNG draws each line that plots with the pen the PDF plots
/// it with: the same colour, and the same width to within the half a
/// pixel the anti-aliasing of its two edges can move it.
fn assert_same_pens(image: &Image, pdf: &Pdf) {
    let plotted = plotted_pens(pdf);
    // The PDF leaves out the line on the layer that does not plot.
    assert_eq!(plotted.len(), 3, "{plotted:?}");
    for (y, (rgb, width)) in LINES.iter().zip(&plotted) {
        let (drawn, drawn_width) = image.pen(*y).unwrap_or_else(|| panic!("no line at {y}"));
        let near = drawn.iter().zip(rgb).all(|(&a, &b)| a.abs_diff(b) <= 2);
        assert!(near, "the line at {y}: PNG {drawn:?}, PDF {rgb:?}");
        assert!(
            (drawn_width - width).abs() < 0.06,
            "the line at {y}: PNG {drawn_width} mm, PDF {width} mm"
        );
    }
    assert_eq!(image.pen(LINES[3]), None, "a layer that does not plot");
}

/// The text of the PNG's text chunk `keyword`, if it has one.
fn text_chunk(path: &str, keyword: &str) -> Option<String> {
    let file = fs::File::open(path).unwrap();
    let reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let info = reader.info();
    info.utf8_text
        .iter()
        .find(|chunk| chunk.keyword == keyword)
        .map(|chunk| chunk.get_text().unwrap())
}

#[test]
fn a_png_through_a_ctb_draws_each_pen_as_the_pdf_through_it_does() {
    let dir = scratch("ctb");
    let (source, settings) = sheet_and_styles(&dir);
    // Named in another case, as a plot finds it.
    let image = render(&source, &settings, |r| {
        r.plot_style = Some("made-up.ctb".into())
    });
    let (report, pdf) = plot(&source, &settings, Some("made-up.ctb"));
    assert_same_pens(&image, &pdf);
    // Blue at 1 mm, the object's own yellow at 0.5 mm, and dark green at
    // 0.5 mm at full strength: the screening is followed by neither.
    let pens: Vec<_> = LINES[..3].iter().map(|&y| image.pen(y).unwrap()).collect();
    assert_eq!(
        pens.iter().map(|p| p.0).collect::<Vec<_>>(),
        [[0, 0, 255], [255, 255, 0], [0, 128, 0]]
    );
    // The reply names the plot style, the file and the screening it does
    // not follow, as the plot's does.
    let DrawnThrough::PlotStyle(used) = &image.report.drawn_through else {
        panic!("{:?}", image.report.drawn_through);
    };
    assert_eq!(used, &report.plot_style);
    assert_eq!(used.name, "made-up.ctb");
    assert_eq!(used.not_followed.len(), 1, "{:?}", used.not_followed);
    assert_eq!(used.not_followed[0].setting, "screen");
    assert_eq!(used.not_followed[0].colors, [3]);
    let reply = serde_json::to_value(&image.report).unwrap();
    assert_eq!(
        reply["drawn_through"],
        json!({"plot_style": serde_json::to_value(&report.plot_style).unwrap()})
    );
}

#[test]
fn monochrome_draws_exactly_as_a_plot_with_no_plot_style_named() {
    let dir = scratch("monochrome");
    let (source, settings) = sheet_and_styles(&dir);
    let image = render(&source, &settings, |r| r.monochrome = true);
    let (report, pdf) = plot(&source, &settings, None);
    assert_eq!(report.plot_style.name, BUILT_IN_PLOT_STYLE);
    assert_same_pens(&image, &pdf);
    for y in &LINES[..3] {
        assert_eq!(image.pen(*y).unwrap().0, [0, 0, 0], "the line at {y}");
    }
    // Every pixel is grey: nothing draws in colour.
    assert!(image.rgb.chunks(3).all(|p| p[0] == p[1] && p[1] == p[2]));
    assert_eq!(image.report.drawn_through, DrawnThrough::Monochrome);
    let reply = serde_json::to_value(&image.report).unwrap();
    assert_eq!(reply["drawn_through"], json!("monochrome"));
}

#[test]
fn neither_draws_each_object_in_its_own_colour_as_before() {
    let dir = scratch("colour");
    let (source, settings) = sheet_and_styles(&dir);
    let image = render(&source, &settings, |_| {});
    let pens: Vec<_> = LINES.iter().map(|&y| image.pen(y).unwrap()).collect();
    // Each layer's colour and lineweight, and the layer that does not plot
    // drawn as a screen shows it.
    assert_eq!(
        pens.iter().map(|p| p.0).collect::<Vec<_>>(),
        [[255, 0, 0], [255, 255, 0], [0, 255, 0], [0, 255, 255]]
    );
    for ((_, width), own) in pens.iter().zip([0.25, 0.5, 0.25, 1.0]) {
        assert!((width - own).abs() < 0.06, "{width} mm, not {own} mm");
    }
    assert_eq!(image.report.drawn_through, DrawnThrough::Colour);
    let reply = serde_json::to_value(&image.report).unwrap();
    assert_eq!(reply["drawn_through"], json!("colour"));
    // The PNG names no plot style.
    assert_eq!(text_chunk(&image.report.png, "Software"), None);
}

#[test]
fn the_stamp_holds_no_plot_style_and_the_png_names_it_as_the_pdf_does() {
    let dir = scratch("stamp");
    let (source, settings) = sheet_and_styles(&dir);
    // Each render replaces the last, so each PNG's chunk is read at once.
    let colour = render(&source, &settings, |_| {}).report;
    let through = render(&source, &settings, |r| {
        r.plot_style = Some("Made-Up.ctb".into())
    })
    .report;
    // The PNG names the plot style where a PDF's Creator names it.
    assert_eq!(
        text_chunk(&through.png, "Software").as_deref(),
        Some("agentdrafter render, plot style Made-Up.ctb")
    );
    let monochrome = render(&source, &settings, |r| r.monochrome = true).report;
    assert_eq!(
        text_chunk(&monochrome.png, "Software"),
        Some(format!(
            "agentdrafter render, plot style {BUILT_IN_PLOT_STYLE}"
        ))
    );
    let (_, pdf) = plot(&source, &settings, Some("Made-Up.ctb"));
    assert_eq!(
        pdf.info("/Creator").as_deref(),
        Some("agentdrafter plot, plot style Made-Up.ctb")
    );
    // The same stamp whatever the pens are drawn through, as a PDF's stamp
    // is the same whatever plot style it plots through.
    assert_eq!(through.stamp, colour.stamp);
    assert_eq!(monochrome.stamp, colour.stamp);
    // And `stale` reads its stamp as any PNG's.
    let stale = check_stale(&StaleRequest {
        output: PathBuf::from(&monochrome.png),
        source: None,
    })
    .unwrap();
    assert!(!stale.stale, "{stale:?}");
}

#[test]
fn a_plot_style_and_monochrome_together_are_refused() {
    let dir = scratch("both");
    let (source, settings) = sheet_and_styles(&dir);
    let request = RenderRequest {
        plot_style: Some("made-up.ctb".into()),
        monochrome: true,
        ..RenderRequest::new(&source)
    };
    let error = render_with(&request, &settings).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage);
    assert_eq!(error.field.as_deref(), Some("monochrome"));
    assert!(error.message.contains("not both"), "{}", error.message);
    assert!(!dir.join("sheet.png").exists());
}

#[test]
fn a_plot_style_a_plot_cannot_find_or_read_is_refused_in_the_same_words() {
    let dir = scratch("unknown");
    let (source, settings) = sheet_and_styles(&dir);
    fs::write(
        settings.plot_style_folders[0].join("broken.ctb"),
        "not a plot style",
    )
    .unwrap();
    for (name, folders, code) in [
        ("nowhere.ctb", &settings, ErrorCode::ReadFailed),
        ("made-up.ctb", &Settings::default(), ErrorCode::ReadFailed),
        ("../made-up.ctb", &settings, ErrorCode::BadValue),
        ("broken.ctb", &settings, ErrorCode::ReadFailed),
    ] {
        let rendered = render_with(
            &RenderRequest {
                plot_style: Some(name.into()),
                ..RenderRequest::new(&source)
            },
            folders,
        )
        .unwrap_err();
        let plotted = plot_with(
            &PlotRequest {
                plot_style: Some(name.into()),
                ..PlotRequest::new(&source)
            },
            folders,
        )
        .unwrap_err();
        assert_eq!(rendered.code, code, "{name}: {rendered:?}");
        assert_eq!(rendered.message, plotted.message, "{name}");
        assert_eq!(rendered.field, plotted.field, "{name}");
        assert_eq!(rendered.file, plotted.file, "{name}");
    }
    assert!(!dir.join("sheet.png").exists(), "nothing was written");
}
