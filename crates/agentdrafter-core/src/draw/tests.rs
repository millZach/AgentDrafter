//! Tests of the drawing pipeline on whole sheets: every object type a sheet
//! description can state draws, with its layer's colour, pen and linetype.

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::description::parse_sheet_description;
use crate::settings::Settings;
use crate::text::truetype::tests::made_up_font;

/// Draws a sheet description with no font folders, so every font is the
/// look-alike.
fn draw(description: &str) -> DisplayList {
    let sheet = parse_sheet_description(description).unwrap_or_else(|e| panic!("{e}"));
    let fonts = SheetFonts::find(&sheet, &Settings::default());
    draw_sheet(&sheet, &fonts, 0.001)
}

fn items_of(list: &DisplayList, object: usize) -> Vec<&Item> {
    list.items.iter().filter(|i| i.object == object).collect()
}

fn runs(item: &Item) -> &Vec<Vec<Point2>> {
    match &item.shape {
        Shape::Stroke { runs, .. } => runs,
        Shape::Fill { .. } => panic!("{item:?} is a fill"),
    }
}

fn lineweight(item: &Item) -> f64 {
    match &item.shape {
        Shape::Stroke { lineweight, .. } => *lineweight,
        Shape::Fill { .. } => panic!("{item:?} is a fill"),
    }
}

const EVERY_TYPE: &str = r#"
units: mm
linetypes:
  DASHED: {pattern: [5, -2.5]}
hatch_patterns:
  LINES:
    lines:
      - {angle: 45, delta: [0, 2]}
text_styles:
  notes: {font: romans.shx, width_factor: 0.8}
layers:
  outline: {color: red, lineweight: 0.5}
  hidden: {color: yellow, linetype: DASHED, lineweight: 0.25}
  fill: {color: 4}
objects:
  - line: {from: [0, 0, 0], to: [100, 0, 0], layer: outline}
  - polyline: {points: [[0, 10, 0], [50, 10, 0], [50, 40, 0]], layer: hidden}
  - arc: {center: [70, 20, 0], radius: 10, start_angle: 0, end_angle: 180, layer: outline}
  - circle: {center: [70, 50, 0], radius: 5}
  - ellipse: {center: [20, 60, 0], major_axis: [15, 0, 0], ratio: 0.5, layer: hidden}
  - spline: {fit_points: [[0, 80, 0], [20, 90, 0], [40, 80, 0]], layer: outline}
  - hatch:
      pattern: SOLID
      boundary: [{polyline: {points: [[80, 70, 0], [90, 70, 0], [90, 80, 0]]}}]
      layer: fill
  - hatch:
      pattern: LINES
      boundary: [{polyline: {points: [[60, 80, 0], [75, 80, 0], [75, 95, 0], [60, 95, 0]]}}]
  - text: {contents: PUMP, position: [0, 100, 0], height: 5, style: notes}
  - mtext: {contents: "NOTE\nTWO", position: [50, 120, 0], height: 2.5, width: 30}
"#;

#[test]
fn every_object_type_draws_with_its_layers_colour_and_pen() {
    let list = draw(EVERY_TYPE);
    let objects: Vec<usize> = list.items.iter().map(|i| i.object).collect();
    for object in 0..10 {
        assert!(
            objects.contains(&object),
            "object {object} draws nothing: {objects:?}"
        );
    }
    let line = items_of(&list, 0)[0];
    assert_eq!(line.color, Color(1));
    assert_eq!(lineweight(line), 0.5);
    assert_eq!(runs(line), &vec![vec![[0.0, 0.0], [100.0, 0.0]]]);
    // A layer with no lineweight draws with AutoCAD's default.
    let circle = items_of(&list, 3)[0];
    assert_eq!(circle.color, Color(7));
    assert_eq!(lineweight(circle), DEFAULT_LINEWEIGHT);
    let solid = items_of(&list, 6)[0];
    assert_eq!(solid.color, Color(4));
    assert!(matches!(
        solid.shape,
        Shape::Fill {
            rule: FillRule::EvenOdd,
            ..
        }
    ));
    let pattern = items_of(&list, 7)[0];
    assert!(runs(pattern).len() > 5, "{pattern:?}");
}

#[test]
fn a_linetype_dashes_each_segment_of_a_flat_polyline() {
    let list = draw(EVERY_TYPE);
    let polyline = items_of(&list, 1)[0];
    assert_eq!(polyline.color, Color(2));
    // Segments 50 and 30 long in 5 dash, 2.5 gap: 7 and 5 dashes, each
    // segment ending in a dash at its corner.
    let dashes = runs(polyline);
    assert_eq!(dashes.len(), 7 + 5, "{dashes:?}");
    assert!(dashes.iter().any(|d| d.last() == Some(&[50.0, 10.0])));
    assert!(dashes.iter().any(|d| d.first() == Some(&[50.0, 10.0])));
}

#[test]
fn a_3d_polyline_draws_continuous_whatever_its_linetype() {
    let list = draw(
        "units: mm\nlinetypes:\n  DASHED: {pattern: [5, -2.5]}\nlayers:\n  hidden: {linetype: DASHED}\nobjects:\n  - polyline: {points: [[0, 0, 0], [50, 0, 10], [50, 50, 20]], layer: hidden}\n  - line: {from: [0, 0, 0], to: [0, 0, 30], layer: hidden}\n",
    );
    assert_eq!(
        runs(items_of(&list, 0)[0]),
        &vec![vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]]]
    );
    // A line straight up is seen from above as a dot.
    assert_eq!(runs(items_of(&list, 1)[0]), &vec![vec![[0.0, 0.0]]]);
}

#[test]
fn text_draws_upright_from_its_baseline_and_turns_with_its_rotation() {
    let list = draw(
        "units: mm\nobjects:\n  - text: {contents: I, position: [10, 20, 0], height: 7}\n  - text: {contents: I, position: [10, 20, 0], height: 7, rotation: 90}\n",
    );
    let points = |object| -> Vec<Point2> {
        items_of(&list, object)
            .iter()
            .flat_map(|i| i.points())
            .collect()
    };
    // The look-alike's I is one stroke from the baseline to the capitals.
    let upright = points(0);
    let low = upright.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
    let high = upright.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!(
        (low - 20.0).abs() < 1e-9 && (high - 27.0).abs() < 1e-9,
        "{upright:?}"
    );
    // Turned a quarter anticlockwise, it lies along -X from its start.
    let turned = points(1);
    assert!(
        turned
            .iter()
            .all(|p| (p[1] - 20.0) > 0.0 && p[0] <= 10.0 + 1e-9),
        "{turned:?}"
    );
    let left = turned.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    assert!((left - 3.0).abs() < 1e-9, "{turned:?}");
}

#[test]
fn multiline_text_puts_each_line_five_thirds_of_its_height_below_the_last() {
    let list = draw("units: mm\nobjects:\n  - mtext: {contents: \"I\\nI\\nI\", position: [0, 50, 0], height: 3}\n");
    let points: Vec<Point2> = items_of(&list, 0).iter().flat_map(|i| i.points()).collect();
    let mut tops: Vec<f64> = points
        .iter()
        .map(|p| p[1])
        .filter(|y| (y - y.round()).abs() < 1e-9)
        .collect();
    tops.sort_by(f64::total_cmp);
    tops.dedup();
    // Each I runs from its baseline up 3: the first from 47 to 50, then 5
    // lower each time.
    assert_eq!(tops, vec![37.0, 40.0, 42.0, 45.0, 47.0, 50.0]);
}

#[test]
fn a_truetype_font_draws_filled_outlines() {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-draw-{}", std::process::id()))
        .join("truetype");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("madeup.ttf"),
        made_up_font(&[('A', 800, [0, 0, 700, 700])]),
    )
    .unwrap();
    let sheet = parse_sheet_description(
        "units: mm\ntext_styles:\n  title: {font: madeup.ttf}\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 10, style: title}\n",
    )
    .unwrap();
    let settings = Settings {
        font_folders: vec![PathBuf::from(&dir)],
    };
    let fonts = SheetFonts::find(&sheet, &settings);
    let list = draw_sheet(&sheet, &fonts, 0.001);
    let Shape::Fill { rule, contours } = &list.items[0].shape else {
        panic!("{:?}", list.items);
    };
    assert_eq!(*rule, FillRule::NonZero);
    let xs: Vec<f64> = contours[0].iter().map(|p| p[0]).collect();
    assert!(xs.iter().any(|x| (x - 10.0).abs() < 1e-9), "{xs:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_extents_take_in_a_hatchs_boundary() {
    let list = draw(
        "units: mm\nhatch_patterns:\n  SPARSE:\n    lines:\n      - {angle: 0, origin: [0, 5], delta: [0, 100]}\nobjects:\n  - hatch: {pattern: SPARSE, boundary: [{polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]]}}]}\n",
    );
    let extents = list.extents().unwrap();
    assert_eq!((extents.min, extents.max), ([0.0, 0.0], [10.0, 10.0]));
}

/// The drawing pipeline is plain computation: it opens no file, starts no
/// thread and reads no clock or environment, so it builds for
/// `wasm32-unknown-unknown` and runs in the viewer's page (ADR-0011).
#[test]
fn the_pipeline_needs_no_file_system_thread_or_clock() {
    let sources = [
        ("draw/mod.rs", include_str!("mod.rs")),
        ("draw/color.rs", include_str!("color.rs")),
        ("draw/curves.rs", include_str!("curves.rs")),
        ("draw/hatch.rs", include_str!("hatch.rs")),
        ("draw/linetype.rs", include_str!("linetype.rs")),
        ("draw/text.rs", include_str!("text.rs")),
        ("raster.rs", include_str!("../raster.rs")),
        ("text/outline.rs", include_str!("../text/outline.rs")),
    ];
    for (name, source) in sources {
        let code = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "std::fs",
            "std::thread",
            "std::time",
            "std::env",
            "std::process",
            "std::net",
            "File::",
        ] {
            assert!(!code.contains(forbidden), "{name} uses {forbidden}");
        }
    }
}
