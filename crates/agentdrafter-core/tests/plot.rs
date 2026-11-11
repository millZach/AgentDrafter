//! Plotting a sheet to a PDF: the page is the sheet's exact size, a CTB
//! plot style sets each pen's colour and lineweight, the text can be found
//! and copied, and the PDF is stamped with what produced it, so `stale`
//! says when it has moved on.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, check_stale, plot_set_with, plot_with, ErrorCode, PlotReport, PlotRequest,
    PlotSetRequest, SetBuildRequest, Settings, SourceKind, StaleRequest, BUILT_IN_PLOT_STYLE,
};

#[path = "support/ctb.rs"]
mod ctb;
#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/pdf.rs"]
mod pdf;

use pdf::Pdf;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-plot")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn plot(source: &Path, request: impl FnOnce(&mut PlotRequest), settings: &Settings) -> PlotReport {
    let mut req = PlotRequest::new(source);
    request(&mut req);
    plot_with(&req, settings).unwrap_or_else(|e| panic!("{e}"))
}

fn read(path: impl AsRef<Path>) -> Pdf {
    Pdf::read(fs::read(path).unwrap())
}

/// The points in `mm` millimetres.
fn points(mm: f64) -> f64 {
    mm / 25.4 * 72.0
}

#[test]
fn an_11x17_sheet_plots_at_exactly_431_8_by_279_4_mm() {
    let dir = scratch("eleven-by-seventeen");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = plot_set_with(
        &PlotSetRequest {
            set: set.clone(),
            overwrite: false,
            plot_style: None,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!((report.plotted, report.failed), (3, 0));
    for sheet in &report.sheets {
        let result = sheet.result.as_ref().unwrap();
        // 17 by 11 inches is 1224 by 792 points, to the point.
        let pdf = read(&result.pdf);
        assert_eq!(
            pdf.media_box(),
            [0.0, 0.0, 1224.0, 792.0],
            "{}",
            sheet.sheet
        );
        assert_eq!(result.sheet_size.width_mm, 431.8);
        assert_eq!(result.sheet_size.height_mm, 279.4);
        assert_eq!(result.sheet_size.width_points, 1224.0);
        assert_eq!(result.sheet_size.height_points, 792.0);
        assert_eq!(result.scale, 1.0);
        assert_eq!(result.extents.min, [0.0, 0.0]);
        assert_eq!(result.extents.max, [17.0, 11.0]);
        assert!(result.pdf.ends_with(&format!("{}.pdf", sheet.sheet)));
    }

    // The DWG the set builds plots on the same page.
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:?}", built.error);
    let dwg = set.join("dwg").join("DEMO-001.dwg");
    let from_dwg = plot(
        &dwg,
        |r| r.output = Some(dir.join("from-dwg.pdf")),
        &Settings::default(),
    );
    assert_eq!(
        read(dir.join("from-dwg.pdf")).media_box(),
        [0.0, 0.0, 1224.0, 792.0]
    );
    assert_eq!(from_dwg.sheet_size.width_mm, 431.8);

    // A sheet drawn in millimetres to the same size plots on the same page.
    let mm = dir.join("mm.yaml");
    fs::write(
        &mm,
        "units: mm\nobjects:\n  - polyline: {points: [[0, 0, 0], [431.8, 0, 0], [431.8, 279.4, 0], [0, 279.4, 0]], closed: true}\n",
    )
    .unwrap();
    let report = plot(&mm, |_| {}, &Settings::default());
    assert_eq!(
        read(dir.join("mm.pdf")).media_box(),
        [0.0, 0.0, 1224.0, 792.0]
    );
    assert_eq!(
        (report.sheet_size.width_mm, report.sheet_size.height_mm),
        (431.8, 279.4)
    );
}

#[test]
fn the_page_is_the_extents_at_one_to_one_and_the_drawing_sits_on_it() {
    let dir = scratch("extents");
    let source = dir.join("sheet.yaml");
    // A 100 by 50 mm frame with its corner at (20, 30), and a line inside
    // it: the page is the frame, and the line sits on the page as it sits
    // in the frame.
    fs::write(
        &source,
        "units: mm\nlayers:\n  box: {color: 1, lineweight: 0.5}\nobjects:\n  - polyline: {points: [[20, 30, 0], [120, 30, 0], [120, 80, 0], [20, 80, 0]], closed: true, layer: box}\n  - line: {from: [30, 40, 0], to: [110, 70, 0], layer: box}\n",
    )
    .unwrap();
    let report = plot(&source, |_| {}, &Settings::default());
    let pdf = read(&report.pdf);
    let [_, _, w, h] = pdf.media_box();
    assert!((w - points(100.0)).abs() < 1e-3, "{w}");
    assert!((h - points(50.0)).abs() < 1e-3, "{h}");
    assert_eq!(report.sheet_size.width_mm, 100.0);
    assert_eq!(report.sheet_size.height_mm, 50.0);
    // The line runs from (10, 10) to (90, 40) mm on the page. The frame lies
    // on the page's edge, which AutoCAD's plot of the same sheet leaves
    // out, and so does this.
    let drawn = pdf.points();
    let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() < 1e-3 && (p[1] - q[1]).abs() < 1e-3;
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert!(near(drawn[0], [points(10.0), points(10.0)]), "{drawn:?}");
    assert!(near(drawn[1], [points(90.0), points(40.0)]), "{drawn:?}");

    // A sheet that draws nothing has no size to plot.
    let empty = dir.join("empty.yaml");
    fs::write(&empty, "units: mm\nobjects: []\n").unwrap();
    let error = plot_with(&PlotRequest::new(&empty), &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert!(error.message.contains("draws nothing"), "{}", error.message);
    // Nor does one that draws a single straight line.
    let flat = dir.join("flat.yaml");
    fs::write(
        &flat,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    let error = plot_with(&PlotRequest::new(&flat), &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert!(
        error.message.contains("width and a height"),
        "{}",
        error.message
    );
}

/// Three lines on layers of colours 1, 2 and 3, of lineweights 0.25, 0.5
/// and the default, inside a frame on the page's edge, which draws nothing.
const THREE_PENS: &str = "units: mm
layers:
  red: {color: 1, lineweight: 0.25}
  yellow: {color: 2, lineweight: 0.5}
  green: {color: 3}
objects:
  - polyline: {points: [[-10, -10, 0], [110, -10, 0], [110, 30, 0], [-10, 30, 0]], closed: true}
  - line: {from: [0, 0, 0], to: [100, 0, 0], layer: red}
  - line: {from: [0, 10, 0], to: [100, 10, 0], layer: yellow}
  - line: {from: [0, 20, 0], to: [100, 20, 0], layer: green}
";

/// What lies along the page's edge, such as a template's trim line, is not
/// drawn, as AutoCAD 2026 left it out of its plot of a made-up probe on
/// paper the sheet's size; what only touches the edge is drawn.
#[test]
fn what_lies_on_the_pages_edge_is_left_out_as_autocad_leaves_it_out() {
    let dir = scratch("edge");
    let source = dir.join("sheet.yaml");
    fs::write(
        &source,
        "units: in
objects:
  - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true}
  - polyline: {points: [[2, 0, 0], [4, 0, 0], [4, 2, 0], [6, 2, 0], [6, 0, 0], [8, 0, 0]]}
  - line: {from: [17, 3, 0], to: [17, 5, 0]}
  - line: {from: [0, 5, 0], to: [17, 5, 0]}
",
    )
    .unwrap();
    let report = plot(&source, |_| {}, &Settings::default());
    let drawn = read(&report.pdf).points();
    // The polyline's middle, from (4, 0) up, across and down to (6, 0), each
    // segment a stroke of its own as a flat polyline's linetype restarts at
    // each vertex, and the line across the page, from edge to edge.
    let inches = |x: f64, y: f64| [x * 72.0, y * 72.0];
    assert_eq!(
        drawn,
        [
            inches(4.0, 0.0),
            inches(4.0, 2.0),
            inches(4.0, 2.0),
            inches(6.0, 2.0),
            inches(6.0, 2.0),
            inches(6.0, 0.0),
            inches(0.0, 5.0),
            inches(17.0, 5.0),
        ]
    );
}

#[test]
fn a_ctb_plot_style_sets_the_colour_and_lineweight_of_each_pen() {
    let dir = scratch("ctb");
    let styles = dir.join("plot styles");
    fs::create_dir_all(&styles).unwrap();
    // Pen 1 plots blue at 1 mm; pen 2 its own colour and lineweight; pen 3
    // dark green at 0.5 mm.
    fs::write(
        styles.join("Made-Up.ctb"),
        ctb::made_up_ctb(&[
            (1, ctb::rgb(0, 0, 255), 21),
            (2, ctb::OBJECT_COLOR, 0),
            (3, ctb::rgb(0, 128, 0), 14),
        ]),
    )
    .unwrap();
    let source = dir.join("sheet.yaml");
    fs::write(&source, THREE_PENS).unwrap();
    let settings = Settings {
        plot_style_folders: vec![styles.clone()],
        ..Settings::default()
    };
    // Named in another case, as AutoCAD finds a file.
    let report = plot(
        &source,
        |r| r.plot_style = Some("made-up.ctb".into()),
        &settings,
    );
    assert_eq!(report.plot_style.name, "made-up.ctb");
    assert!(report
        .plot_style
        .file
        .as_deref()
        .unwrap()
        .ends_with("Made-Up.ctb"));
    let pens = read(&report.pdf).strokes();
    assert_eq!(pens.len(), 3, "{pens:?}");
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
    assert_eq!(pens[0].rgb, [0.0, 0.0, 1.0]);
    assert!(near(pens[0].width, points(1.0)), "{pens:?}");
    assert_eq!(pens[1].rgb, [1.0, 1.0, 0.0]);
    assert!(near(pens[1].width, points(0.5)), "{pens:?}");
    assert!(near(pens[2].rgb[1], 128.0 / 255.0) && pens[2].rgb[0] == 0.0);
    assert!(near(pens[2].width, points(0.5)), "{pens:?}");

    // With no plot style named, every pen plots black at its own
    // lineweight, as AutoCAD's monochrome.ctb plots.
    let report = plot(&source, |r| r.overwrite = true, &settings);
    assert_eq!(report.plot_style.name, BUILT_IN_PLOT_STYLE);
    assert_eq!(report.plot_style.file, None);
    let pens = read(&report.pdf).strokes();
    assert!(pens.iter().all(|p| p.rgb == [0.0, 0.0, 0.0]), "{pens:?}");
    assert!(near(pens[0].width, points(0.25)));
    assert!(near(pens[1].width, points(0.5)));
    assert!(near(pens[2].width, points(0.25)), "the default lineweight");
}

#[test]
fn a_plot_style_that_cannot_be_found_or_read_says_why() {
    let dir = scratch("ctb-missing");
    let source = dir.join("sheet.yaml");
    fs::write(&source, THREE_PENS).unwrap();
    let request = |name: &str| PlotRequest {
        plot_style: Some(name.into()),
        ..PlotRequest::new(&source)
    };
    // No folder set.
    let error = plot_with(&request("monochrome.ctb"), &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert_eq!(error.field.as_deref(), Some("plot_style"));
    assert!(
        error.message.contains("no plot style folder"),
        "{}",
        error.message
    );
    // Not in the folder.
    let settings = Settings {
        plot_style_folders: vec![dir.clone()],
        ..Settings::default()
    };
    let error = plot_with(&request("monochrome.ctb"), &settings).unwrap_err();
    assert!(
        error.message.contains("none of the plot style folders"),
        "{}",
        error.message
    );
    // A path, not a name: a plot style is never read from outside its folders.
    let error = plot_with(&request("../monochrome.ctb"), &settings).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    let error = plot_with(&request("sheet.yaml"), &settings).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    // Not a CTB.
    fs::write(dir.join("broken.ctb"), "not a plot style").unwrap();
    let error = plot_with(&request("broken.ctb"), &settings).unwrap_err();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert!(error.message.contains("not a CTB"), "{}", error.message);
    // Nothing was written.
    assert!(!dir.join("sheet.pdf").exists());
}

#[test]
fn the_text_of_a_sheet_can_be_searched_and_copied() {
    let dir = scratch("text");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = plot_set_with(
        &PlotSetRequest {
            set: set.clone(),
            overwrite: false,
            plot_style: None,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:?}", reply.error);
    let pdf = read(set.join("dwg").join("DEMO-003.pdf"));
    let lines: Vec<String> = pdf.text().into_iter().map(|l| l.text).collect();
    // The template's text, the multiline notes line by line, the title
    // block's own text and each visible attribute's value.
    for expected in [
        "AREA 300 GENERAL ARRANGEMENT",
        "NOTES:",
        "1. THIS IS A MADE-UP DRAWING OF THE DEMO PLANT.",
        "2. DIMENSIONS ARE IN INCHES.",
        "DEMO PLANT",
        "DWG NO.",
        "DEMO-003",
        "3 OF 3",
        "A",
        "2026-11-11",
        "AD",
    ] {
        assert!(
            lines.iter().any(|l| l == expected),
            "`{expected}` is not among {lines:?}"
        );
    }
    let report = &reply.result.unwrap().sheets[2];
    assert_eq!(report.result.as_ref().unwrap().text_lines, lines.len());
    // The title is set on its baseline, at its height, where it draws: the
    // template's at (1, 10) inches, 0.25 inches high; the title block's
    // attribute, 0.2 inches high, below it.
    let titles: Vec<[f64; 6]> = pdf
        .text()
        .into_iter()
        .filter(|l| l.text == "AREA 300 GENERAL ARRANGEMENT")
        .map(|l| l.matrix)
        .collect();
    assert_eq!(titles.len(), 2, "{titles:?}");
    let [_, _, _, up, x, y] = titles[1];
    assert!(
        (x - 72.0).abs() < 0.01 && (y - 720.0).abs() < 0.01,
        "{titles:?}"
    );
    assert!((up - 18.0).abs() < 0.01, "{titles:?}");
    assert!((titles[0][3] - 14.4).abs() < 0.01, "{titles:?}");
}

#[test]
fn the_pdf_records_what_produced_it_and_goes_stale_when_that_moves_on() {
    let dir = scratch("stale");
    let library = dir.join("blocks.yaml");
    fs::write(
        &library,
        "units: mm\nblocks:\n  MARK:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 2}\n",
    )
    .unwrap();
    let source = dir.join("sheet.yaml");
    let sheet = "units: mm\nblock_library: blocks.yaml\nobjects:\n  - line: {from: [0, 0, 0], to: [50, 20, 0]}\n  - insert: {block: MARK, position: [25, 10, 0]}\n";
    fs::write(&source, sheet).unwrap();
    let report = plot(&source, |_| {}, &Settings::default());
    let kinds: Vec<SourceKind> = report.stamp.sources.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [SourceKind::SheetDescription, SourceKind::BlockLibrary]
    );
    assert_eq!(report.stamp.sources[0].path, "sheet.yaml");
    assert_eq!(
        read(&report.pdf).info("/Producer").as_deref(),
        Some(report.stamp.produced_by.as_str())
    );
    let stale = |output: &Path| {
        check_stale(&StaleRequest {
            output: output.to_path_buf(),
            source: None,
        })
        .unwrap()
    };
    let pdf = PathBuf::from(&report.pdf);
    let fresh = stale(&pdf);
    assert!(!fresh.stale, "{}", fresh.why);
    assert_eq!(fresh.stamp, report.stamp);

    fs::write(
        &library,
        fs::read_to_string(&library)
            .unwrap()
            .replace("radius: 2", "radius: 3"),
    )
    .unwrap();
    let moved = stale(&pdf);
    assert!(moved.stale);
    assert_eq!(
        moved.sources.iter().map(|s| s.stale).collect::<Vec<_>>(),
        [false, true]
    );
    assert!(moved.why.contains("block library"), "{}", moved.why);

    // A drawing set's sheet is stamped with its template, its rows and its
    // block library, and goes stale when its rows change.
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = plot_set_with(
        &PlotSetRequest {
            set: set.clone(),
            overwrite: false,
            plot_style: None,
        },
        &Settings::default(),
    );
    let result = reply.result.unwrap().sheets[0].result.clone().unwrap();
    let named: Vec<(SourceKind, &str)> = result
        .stamp
        .sources
        .iter()
        .map(|s| (s.kind, s.path.as_str()))
        .collect();
    assert_eq!(
        named,
        [
            (SourceKind::Template, "../templates/ansi-b.yaml"),
            (SourceKind::Rows, "../rows.yaml"),
            (SourceKind::BlockLibrary, "../blocks.yaml"),
        ]
    );
    let sheet_pdf = PathBuf::from(&result.pdf);
    assert!(!stale(&sheet_pdf).stale);
    let rows = set.join("rows.yaml");
    fs::write(
        &rows,
        fs::read_to_string(&rows)
            .unwrap()
            .replace("REV: A", "REV: B"),
    )
    .unwrap();
    let moved = stale(&sheet_pdf);
    assert!(moved.stale);
    assert!(moved.why.contains("rows file"), "{}", moved.why);
}

#[test]
fn a_pdf_is_replaced_only_when_asked_and_only_when_agentdrafter_plotted_it() {
    let dir = scratch("overwrite");
    let source = dir.join("sheet.yaml");
    fs::write(&source, THREE_PENS).unwrap();
    let first = plot(&source, |_| {}, &Settings::default());
    // Plotting again gives the same bytes: nothing in the PDF depends on
    // when it was plotted.
    let bytes = fs::read(&first.pdf).unwrap();
    let error = plot_with(&PlotRequest::new(&source), &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputExists);
    plot(&source, |r| r.overwrite = true, &Settings::default());
    assert_eq!(fs::read(&first.pdf).unwrap(), bytes);

    // A PDF someone else wrote is never replaced.
    let theirs = dir.join("theirs.pdf");
    fs::write(&theirs, b"%PDF-1.4\n% someone's own drawing\n").unwrap();
    let error = plot_with(
        &PlotRequest {
            output: Some(theirs.clone()),
            overwrite: true,
            ..PlotRequest::new(&source)
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput);
    assert_eq!(
        fs::read(&theirs).unwrap(),
        b"%PDF-1.4\n% someone's own drawing\n"
    );
    let error = check_stale(&StaleRequest {
        output: theirs,
        source: None,
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotStamped);
}

#[test]
fn a_set_plots_every_sheet_it_can_and_says_which_failed() {
    let dir = scratch("set-failed");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let rows = set.join("rows.yaml");
    // The second row loses its title.
    let text = fs::read_to_string(&rows).unwrap();
    fs::write(
        &rows,
        text.replacen("      TITLE: AREA 200 GENERAL ARRANGEMENT\n", "", 1),
    )
    .unwrap();
    let reply = plot_set_with(
        &PlotSetRequest {
            set: set.clone(),
            overwrite: false,
            plot_style: None,
        },
        &Settings::default(),
    );
    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::SheetsFailed);
    let report = reply.result.unwrap();
    assert_eq!((report.plotted, report.failed), (2, 1));
    assert_eq!(
        report.sheets[1].error.as_ref().unwrap().code,
        ErrorCode::MissingValue
    );
    assert!(set.join("dwg").join("DEMO-001.pdf").exists());
    assert!(!set.join("dwg").join("DEMO-002.pdf").exists());
    assert!(set.join("dwg").join("DEMO-003.pdf").exists());
}
