//! Text and multiline text build, and reading the DWG back gives the same
//! contents, style, height, width factor, rotation and justification, with
//! the same text styles. Fonts the font folders do not hold are reported
//! with the look-alike measured in their place.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, parse_sheet_description, read_dwg, BuildReport, BuildRequest, DrawingContents,
    Object, Settings, Sheet, TextStyle,
};

/// How far an angle read back may be from the one written, in degrees. A
/// DWG holds angles in radians, so degrees do not come back bit for bit.
const ANGLE_TOLERANCE: f64 = 1e-9;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-text")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds `text` with no font folders and returns the sheet, the build's
/// report and what the DWG reads back as.
fn build_and_read(test: &str, text: &str) -> (Sheet, BuildReport, DrawingContents) {
    let dir = scratch(test);
    let description = dir.join("sheet.yaml");
    fs::write(&description, text).unwrap();
    let report = build_with(
        &BuildRequest {
            description,
            output: None,
            overwrite: false,
        },
        &Settings::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let sheet = parse_sheet_description(text).unwrap();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.dwg_version, "AC1032");
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.text_styles_not_read, 0, "{read:?}");
    (sheet, report, read)
}

/// The text styles a DWG reads back as: `Standard` first, as every drawing
/// has it, then the sheet's own in order.
fn styles_in_the_dwg(sheet: &Sheet) -> Vec<TextStyle> {
    let standard = sheet
        .text_styles
        .iter()
        .find(|s| s.name == "Standard")
        .cloned()
        .unwrap_or_else(TextStyle::standard);
    std::iter::once(standard)
        .chain(
            sheet
                .text_styles
                .iter()
                .filter(|s| s.name != "Standard")
                .cloned(),
        )
        .collect()
}

/// Asserts that the DWG read back holds the sheet's objects, in order:
/// everything bit for bit but rotations, which match to [`ANGLE_TOLERANCE`].
#[track_caller]
fn assert_same_objects(sheet: &Sheet, read: &DrawingContents) {
    assert_eq!(
        read.objects.len(),
        sheet.objects.len(),
        "{:#?}",
        read.objects
    );
    let close = |a: f64, b: f64| (a - b).abs() <= ANGLE_TOLERANCE;
    for (i, (wrote, came_back)) in sheet.objects.iter().zip(&read.objects).enumerate() {
        let same = match (wrote, came_back) {
            (Object::Text(w), Object::Text(r)) => {
                let mut r2 = r.clone();
                r2.rotation = w.rotation;
                close(w.rotation, r.rotation) && *w == r2
            }
            (Object::MText(w), Object::MText(r)) => {
                let mut r2 = r.clone();
                r2.rotation = w.rotation;
                close(w.rotation, r.rotation) && *w == r2
            }
            _ => wrote == came_back,
        };
        assert!(
            same,
            "object {i}: wrote {wrote:#?}, read back {came_back:#?}"
        );
    }
}

#[test]
fn the_text_example_reads_back_with_its_styles_and_every_justification() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/text.yaml"))
            .unwrap();
    let (sheet, report, read) = build_and_read("example", &text);
    assert_eq!(read.text_styles, styles_in_the_dwg(&sheet));
    assert_same_objects(&sheet, &read);
    let justified: std::collections::BTreeSet<String> = sheet
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Text(t) => Some(format!("text {:?}", t.justify)),
            Object::MText(t) => Some(format!("mtext {:?}", t.justify)),
            _ => None,
        })
        .collect();
    assert_eq!(
        justified.len(),
        13 + 9,
        "every justification: {justified:?}"
    );
    assert_eq!(report.objects.get("text"), Some(&14));
    assert_eq!(report.objects.get("mtext"), Some(&10));
}

#[test]
fn the_fixed_width_strings_build_and_read_back() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/text-widths.yaml"),
    )
    .unwrap();
    let (sheet, report, read) = build_and_read("widths", &text);
    assert_eq!(read.text_styles, styles_in_the_dwg(&sheet));
    assert_same_objects(&sheet, &read);
    assert_eq!(report.objects.get("text"), Some(&120));
}

#[test]
fn text_reads_back_with_its_style_height_width_factor_rotation_and_justification() {
    let (sheet, _, read) = build_and_read(
        "text",
        "units: mm
text_styles:
  notes: {font: romans.shx, width_factor: 0.75}
  title: {font: arial.ttf}
layers: {labels: {color: cyan}}
objects:
  - text: {contents: PUMP P-101, position: [10, 20, 0], height: 2.5, style: notes}
  - text: {contents: '50% OPEN, 100%% SURE', position: [-5.5, 1e3, 2], height: 0.125 in, style: title, width_factor: 1.25, rotation: 33.3, justify: top_right, layer: labels}
  - text: {contents: 'Ünïcode ° ±', position: [0, 0, 0], height: 7, rotation: 270, justify: middle}
",
    );
    assert_eq!(read.text_styles, styles_in_the_dwg(&sheet));
    assert_same_objects(&sheet, &read);
}

#[test]
fn mtext_reads_back_with_its_lines_box_and_codes_left_alone() {
    let (sheet, _, read) = build_and_read(
        "mtext",
        r"units: in
text_styles:
  Standard: {font: simplex.shx, width_factor: 0.9}
objects:
  - mtext:
      contents: |
        NOTES:
        1. ALL {DIMENSIONS} IN INCHES.
        2. SEE C:\DRAWINGS\SHEET-2 FOR 100% OF THE %%D.
      position: [1, 10, 0]
      height: 0.125
      width: 4
  - mtext: {contents: ONE LINE, position: [0, 0, 0], height: 3 mm, width_factor: 0.7, rotation: 90, justify: middle_center}
",
    );
    assert_eq!(read.text_styles, styles_in_the_dwg(&sheet));
    assert_eq!(read.text_styles[0].font, "simplex.shx");
    assert_same_objects(&sheet, &read);
}

#[test]
fn a_build_reports_each_style_whose_font_it_could_not_find() {
    let (_, report, _) = build_and_read(
        "substitutions",
        "units: mm
text_styles:
  notes: {font: romans.shx}
  unused: {font: arial.ttf}
objects:
  - text: {contents: A, position: [0, 0, 0], height: 1, style: notes}
  - text: {contents: B, position: [0, 5, 0], height: 1}
",
    );
    let found: Vec<(&str, &str, &str)> = report
        .font_substitutions
        .iter()
        .map(|s| (s.style.as_str(), s.font.as_str(), s.used.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("Standard", "txt.shx", "Hershey Roman Simplex"),
            ("notes", "romans.shx", "Hershey Roman Simplex"),
            ("unused", "arial.ttf", "Hershey Roman Simplex"),
        ]
    );
    assert!(report
        .font_substitutions
        .iter()
        .all(|s| s.why == "no font folder is set"));
}

#[test]
fn a_sheet_without_text_reports_no_substitution() {
    let (_, report, read) = build_and_read(
        "no-text",
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 1, 0]}\n",
    );
    assert!(report.font_substitutions.is_empty());
    assert_eq!(read.text_styles, [TextStyle::standard()]);
}
