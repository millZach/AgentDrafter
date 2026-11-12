//! Plotting, in the crate, where the made-up font writers are: text in SHX
//! and TrueType fonts can be found and copied in the PDF, each line where
//! it draws, while it still draws as the display list draws it.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::text::shx::tests::unifont;
use crate::text::truetype::tests::made_up_font;

#[path = "../../tests/support/pdf.rs"]
mod reader;

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-plot-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up SHX font: each of `characters` a stroke up the middle of a
/// cell 4 wide, the space a cell 4 wide with nothing in it, capitals 10
/// tall.
fn shx_font(characters: &str) -> Vec<u8> {
    let stroke: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];
    let space: &[u8] = &[2, 8, 4, 0, 0];
    let mut shapes: Vec<(u16, &[u8])> = characters
        .chars()
        .map(|c| (u16::try_from(u32::from(c)).unwrap(), stroke))
        .collect();
    shapes.push((32, space));
    unifont(10, &shapes)
}

/// A made-up TrueType font: each of `characters` a square 600 wide, the
/// space 250 wide and empty.
fn truetype_font(characters: &str) -> Vec<u8> {
    let mut glyphs: Vec<(char, u16, [i16; 4])> = characters
        .chars()
        .map(|c| (c, 600, [50, 0, 550, 700]))
        .collect();
    glyphs.push((' ', 250, [0, 0, 0, 0]));
    made_up_font(&glyphs)
}

fn plot_in(dir: &Path, description: &str) -> (PlotReport, reader::Pdf) {
    let fonts = dir.join("fonts");
    fs::create_dir_all(&fonts).unwrap();
    fs::write(fonts.join("madeup.shx"), shx_font("TAGPMIEUR-0123456789")).unwrap();
    fs::write(
        fonts.join("madeup.ttf"),
        truetype_font("TAGPMIEUR-0123456789"),
    )
    .unwrap();
    let source = dir.join("sheet.yaml");
    fs::write(&source, description).unwrap();
    let settings = Settings {
        font_folders: vec![fonts],
        ..Settings::default()
    };
    let report = plot_with(&PlotRequest::new(&source), &settings).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        report.font_substitutions,
        Vec::new(),
        "the made-up fonts were found"
    );
    let pdf = reader::Pdf::read(fs::read(&report.pdf).unwrap());
    (report, pdf)
}

#[test]
fn text_in_shx_and_truetype_fonts_can_be_searched_and_copied() {
    let dir = scratch("fonts");
    let (report, pdf) = plot_in(
        &dir,
        "units: mm
text_styles:
  stroked: {font: madeup.shx}
  filled: {font: madeup.ttf}
objects:
  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 100, 0], [0, 100, 0]], closed: true}
  - text: {contents: PUMP P-101, position: [10, 80, 0], height: 5, style: stroked}
  - text: {contents: TAG 2026, position: [10, 60, 0], height: 5, style: filled}
  - text: {contents: TURE, position: [150, 20, 0], height: 4, style: filled, rotation: 90}
  - mtext: {contents: \"MUTE TAP\\nGRIP 42\", position: [60, 40, 0], height: 3, width: 80, style: stroked}
",
    );
    let lines: Vec<String> = pdf.text().into_iter().map(|l| l.text).collect();
    assert_eq!(
        lines,
        ["PUMP P-101", "TAG 2026", "TURE", "MUTE TAP", "GRIP 42"],
        "every line, in the order the sheet draws it"
    );
    assert_eq!(report.text_lines, 5);

    // Each line is set on its baseline, in its frame: the SHX line 5 mm high
    // at (10, 80) on a page whose corner is the drawing's (0, 0).
    let text = pdf.text();
    let mm = |v: f64| v / 25.4 * 72.0;
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    let [a, b, c, d, x, y] = text[0].matrix;
    assert!(
        near(x, mm(10.0)) && near(y, mm(80.0)),
        "{:?}",
        text[0].matrix
    );
    assert!(near(d, mm(5.0)) && near(a, mm(5.0)) && b == 0.0 && c == 0.0);
    // The turned line runs up the page.
    let [a, b, c, d, ..] = text[2].matrix;
    assert!(near(a, 0.0) && near(b, mm(4.0)) && near(c, -mm(4.0)) && near(d, 0.0));

    // The text still draws: the two SHX texts as strokes, after the frame,
    // and the TrueType text as filled outlines.
    assert_eq!(pdf.strokes().len(), 3, "{:?}", pdf.strokes());
    assert!(!pdf.fills().is_empty());
}

#[test]
fn each_character_of_the_invisible_text_is_as_wide_as_it_draws() {
    let dir = scratch("advances");
    let source = dir.join("sheet.yaml");
    fs::write(
        &source,
        "units: mm\nobjects:\n  - text: {contents: AB, position: [0, 0, 0], height: 10}\n  - line: {from: [0, -5, 0], to: [50, 20, 0]}\n",
    )
    .unwrap();
    let sheet = crate::read_sheet_description(&source).unwrap();
    let fonts = SheetFonts::find(&sheet, &Settings::default());
    let list = draw::draw_sheet(&sheet, &fonts, 0.01, draw::MOST_HATCH_DASHES);
    let run = &list.text[0];
    assert_eq!(
        run.characters.iter().map(|&(c, _)| c).collect::<String>(),
        "AB"
    );
    // Where the pen goes after the run is where the text's own measure
    // says it ends.
    let (font, _) = fonts.style(crate::text::STANDARD);
    let measured = font.line("AB").advance;
    let drawn: f64 = run.characters.iter().map(|&(_, a)| a).sum();
    assert!((drawn - measured).abs() < 1e-12, "{drawn} {measured}");
}

#[test]
fn a_pdf_whose_text_shows_a_stamp_is_not_one_agentdrafter_plotted() {
    let dir = scratch("stamp-shown");
    let source = dir.join("sheet.yaml");
    fs::write(
        &source,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 50, 0]}\n",
    )
    .unwrap();
    // The stamp of a real plot beside it, whose source has not moved on.
    let plotted = plot_with(&PlotRequest::new(&source), &Settings::default()).unwrap();
    let shown = format!("/AgentDrafter ({})", plotted.stamp.to_json());
    // Someone's PDF that shows that stamp in its page's text, as a page
    // about AgentDrafter might, with no stamp in its document information.
    let theirs = dir.join("theirs.pdf");
    let bytes = crate::pdf::tests::their_pdf(&shown, &[("Title", "How a PDF is stamped")]);
    fs::write(&theirs, &bytes).unwrap();

    let error = crate::check_stale(&crate::StaleRequest {
        output: theirs.clone(),
        source: None,
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotStamped, "{error:?}");
    let error = plot_with(
        &PlotRequest {
            output: Some(theirs.clone()),
            overwrite: true,
            ..PlotRequest::new(&source)
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput, "{error:?}");
    assert_eq!(fs::read(&theirs).unwrap(), bytes);
}

#[test]
fn a_plot_saved_again_with_object_streams_is_still_ours() {
    let dir = scratch("object-streams");
    let source = dir.join("sheet.yaml");
    fs::write(
        &source,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 50, 0]}\n",
    )
    .unwrap();
    plot_with(&PlotRequest::new(&source), &Settings::default()).unwrap();
    let plotted = fs::read(dir.join("sheet.pdf")).unwrap();
    // Our PDF as a reader that saves with object streams writes it: its
    // trailer in a cross-reference stream, its document information inside
    // an object stream.
    let saved = reader::with_object_streams(&plotted);
    assert!(!saved.windows(7).any(|w| w == b"trailer"));
    assert!(!saved.windows(13).any(|w| w == b"/AgentDrafter"));
    let again = dir.join("saved.pdf");
    fs::write(&again, &saved).unwrap();
    assert_eq!(crate::pdf::stamp_in(&saved), crate::pdf::stamp_in(&plotted));
    let fresh = crate::check_stale(&crate::StaleRequest {
        output: again.clone(),
        source: Some(source.clone()),
    })
    .unwrap();
    assert!(!fresh.stale, "{}", fresh.why);
    // So `--overwrite` replaces it, as one of ours.
    plot_with(
        &PlotRequest {
            output: Some(again.clone()),
            overwrite: true,
            ..PlotRequest::new(&source)
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(fs::read(&again).unwrap(), plotted);

    // A stamp shown in the page's text is still no stamp when the PDF is
    // saved so: only the document information's entry is read.
    let shown = format!(
        "/AgentDrafter ({})",
        crate::pdf::stamp_in(&plotted).unwrap()
    );
    let theirs = reader::with_object_streams(&crate::pdf::tests::their_pdf(
        &shown,
        &[("Title", "How a PDF is stamped")],
    ));
    assert_eq!(crate::pdf::stamp_in(&theirs), None);
    let theirs_path = dir.join("theirs.pdf");
    fs::write(&theirs_path, &theirs).unwrap();
    let error = plot_with(
        &PlotRequest {
            output: Some(theirs_path.clone()),
            overwrite: true,
            ..PlotRequest::new(&source)
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput, "{error:?}");
    assert_eq!(fs::read(&theirs_path).unwrap(), theirs);
}
