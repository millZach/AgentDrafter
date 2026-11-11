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
    draw_sheet(&sheet, &fonts, 0.001, MOST_HATCH_DASHES)
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
    let list = draw_sheet(&sheet, &fonts, 0.001, MOST_HATCH_DASHES);
    let Shape::Fill { rule, contours } = &list.items[0].shape else {
        panic!("{:?}", list.items);
    };
    assert_eq!(*rule, FillRule::NonZero);
    let xs: Vec<f64> = contours[0].iter().map(|p| p[0]).collect();
    assert!(xs.iter().any(|x| (x - 10.0).abs() < 1e-9), "{xs:?}");
    let _ = fs::remove_dir_all(&dir);
}

/// Each line of multiline text lands on the box by the point of its ink as
/// far across as the attachment is: its left edge, middle or right edge.
#[test]
fn multiline_text_aligns_each_line_by_its_ink_at_its_attachment() {
    let attachments = ["top", "middle", "bottom"].into_iter().flat_map(|row| {
        [("left", 0.0), ("center", 0.5), ("right", 1.0)]
            .map(|(column, across)| (row, column, across))
    });
    for (row, column, across) in attachments {
        let list = draw(&format!(
            "units: mm\nobjects:\n  - mtext: {{contents: \"I\\nIII\", position: [100, 50, 0], height: 3, width: 30, justify: {row}_{column}}}\n"
        ));
        let points: Vec<Point2> = items_of(&list, 0).iter().flat_map(|i| i.points()).collect();
        let top = points.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
        // The first line reaches 3 down from the top, the second starts 5
        // below the first.
        for line in [true, false] {
            let xs: Vec<f64> = points
                .iter()
                .filter(|p| (p[1] > top - 4.0) == line)
                .map(|p| p[0])
                .collect();
            let left = xs.iter().copied().fold(f64::MAX, f64::min);
            let right = xs.iter().copied().fold(f64::MIN, f64::max);
            let at = left + (right - left) * across;
            assert!(
                (at - 100.0).abs() < 1e-9 && right > left - 1e-9,
                "{row}_{column}, line {}: ink from {left} to {right}",
                if line { 1 } else { 2 }
            );
        }
    }
}

/// Text stretched along its line keeps within the tolerance across as well
/// as up: a made-up SHX font's half circle, drawn eight times wider than it
/// is high, strays no further from its true curve than the tolerance.
#[test]
fn stretched_text_keeps_within_the_tolerance_both_ways() {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-draw-{}", std::process::id()))
        .join("stretched");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // From the pen at (0, 0), a half circle of radius 5 (half the capitals'
    // height of 10), from the west of its centre anticlockwise to its east.
    fs::write(
        dir.join("arc.shx"),
        crate::text::shx::tests::unifont(10, &[(65, &[10, 5, 0x44, 0])]),
    )
    .unwrap();
    let sheet = parse_sheet_description(
        "units: mm\ntext_styles:\n  wide: {font: arc.shx, width_factor: 8}\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 10, style: wide}\n",
    )
    .unwrap();
    let settings = Settings {
        font_folders: vec![PathBuf::from(&dir)],
    };
    let fonts = SheetFonts::find(&sheet, &settings);
    let tolerance = 0.01;
    let list = draw_sheet(&sheet, &fonts, tolerance, MOST_HATCH_DASHES);
    // The true curve: centre (40, 0), 40 across and 5 up.
    let curve: Vec<Point2> = (0..=20_000)
        .map(|i| {
            let t = std::f64::consts::PI * (1.0 + f64::from(i) / 20_000.0);
            [40.0 + 40.0 * t.cos(), 5.0 * t.sin()]
        })
        .collect();
    let from_curve = |p: Point2| {
        curve
            .windows(2)
            .map(|s| {
                let (d, e) = (
                    [s[1][0] - s[0][0], s[1][1] - s[0][1]],
                    [p[0] - s[0][0], p[1] - s[0][1]],
                );
                let f = ((d[0] * e[0] + d[1] * e[1]) / (d[0] * d[0] + d[1] * d[1])).clamp(0.0, 1.0);
                (e[0] - f * d[0]).hypot(e[1] - f * d[1])
            })
            .fold(f64::MAX, f64::min)
    };
    let chords: Vec<[Point2; 2]> = runs(&list.items[0])
        .iter()
        .flat_map(|run| run.windows(2).map(|c| [c[0], c[1]]))
        .collect();
    assert!(chords.len() > 8, "{chords:?}");
    let worst = chords
        .iter()
        .map(|[a, b]| from_curve([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]))
        .fold(0.0, f64::max);
    assert!(worst <= tolerance * 1.01, "a chord strays {worst}");
    let _ = fs::remove_dir_all(&dir);
}

/// Draws a sheet description that names a block library, each written to
/// a folder of the test's own, with no font folders.
fn draw_with_library(test: &str, library: &str, description: &str) -> DisplayList {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-draw-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("blocks.yaml"), library).unwrap();
    fs::write(dir.join("sheet.yaml"), description).unwrap();
    let sheet = crate::description::read_sheet_description(&dir.join("sheet.yaml"))
        .unwrap_or_else(|e| panic!("{e}"));
    let fonts = SheetFonts::find(&sheet, &Settings::default());
    let list = draw_sheet(&sheet, &fonts, 0.001, MOST_HATCH_DASHES);
    let _ = fs::remove_dir_all(&dir);
    list
}

const LIBRARY: &str = r#"
units: mm
linetypes:
  DASHED: {pattern: [2, -1]}
layers:
  own: {color: blue}
  tags: {color: green}
blocks:
  B:
    base_point: [10, 0, 0]
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0]}
      - circle: {center: [10, 0, 0], radius: 1, layer: own}
      - attdef: {tag: TAG, position: [0, 5, 0], height: 2, layer: tags}
      - attdef: {tag: HIDDEN, position: [0, -5, 0], height: 2, invisible: true}
"#;

const INSERTS: &str = r#"
units: mm
block_library: blocks.yaml
layers:
  red: {color: red, lineweight: 0.5}
objects:
  - insert: {block: B, position: [100, 50, 0], scale: 2, rotation: 90, attributes: {TAG: X, HIDDEN: Y}, layer: red}
"#;

#[track_caller]
fn near(a: Point2, b: Point2) {
    assert!(
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
        "{a:?} is not {b:?}"
    );
}

/// An insert draws its block's objects moved so the base point sits at its
/// position, doubled and turned a quarter; the line on layer 0 draws with
/// the insert's layer, the circle on its own; the visible attribute draws
/// its value and the invisible one, and the definitions, draw nothing.
#[test]
fn an_insert_draws_its_block_placed_with_its_visible_attributes() {
    let list = draw_with_library("insert", LIBRARY, INSERTS);
    let colors: Vec<Color> = list.items.iter().map(|i| i.color).collect();
    assert_eq!(colors, [Color(1), Color(5), Color(3)], "{:?}", list.items);
    assert!(list.items.iter().all(|i| i.object == 0));
    // The line, from (0, 0) and (10, 0) in the block's own coordinates.
    let line = runs(&list.items[0]);
    assert_eq!(line.len(), 1);
    near(line[0][0], [100.0, 30.0]);
    near(line[0][1], [100.0, 50.0]);
    assert_eq!(lineweight(&list.items[0]), 0.5);
    // The circle round the base point, at the insert's position, radius 2.
    for p in list.items[1].points() {
        assert!(
            ((p[0] - 100.0).hypot(p[1] - 50.0) - 2.0).abs() < 1e-3,
            "{p:?}"
        );
    }
    assert_eq!(lineweight(&list.items[1]), DEFAULT_LINEWEIGHT);
    // The attribute, 4 high and turned a quarter, starts at (90, 30): it
    // reaches up to 4 towards -X and runs along +Y.
    let text: Vec<Point2> = list.items[2].points().collect();
    assert!(
        text.iter()
            .all(|p| (86.0 - 1e-9..=90.0 + 1e-9).contains(&p[0])
                && (30.0 - 1e-9..36.0).contains(&p[1])),
        "{text:?}"
    );
    assert!(text.iter().any(|p| (p[0] - 86.0).abs() < 1e-9));
    // The extents take in the block's box, placed, (0, -1) to (11, 1) in
    // the block, and the attribute.
    let extents = list.extents().unwrap();
    near(extents.min, [86.0, 30.0]);
    near(extents.max, [102.0, 52.0]);
}

/// A turned insert's extents are its block's box turned, which reaches past
/// what it draws, as AutoCAD 2026's extents of the blocks example did.
#[test]
fn a_turned_inserts_extents_are_its_block_box_turned() {
    let sheet = INSERTS.replace("rotation: 90", "rotation: 45");
    let list = draw_with_library("turned", LIBRARY, &sheet);
    // The box from (0, -1) to (11, 1), doubled about the base point (10,
    // 0) and turned 45 degrees: its corner (2, -2) from the base point
    // reaches furthest right and (2, 2) furthest up, past the circle of
    // radius 2 round the insert's position.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let reach = 4.0 * half;
    let extents = list.extents().unwrap();
    near(extents.max, [100.0 + reach, 50.0 + reach]);
    let drawn = list.items.iter().flat_map(Item::points);
    let right = drawn.map(|p| p[0]).fold(f64::MIN, f64::max);
    assert!(right < 100.0 + reach - 0.5, "{right}");
}

/// Layer 0's pen inside a block is the insert's layer's: its colour,
/// linetype and lineweight.
#[test]
fn an_object_on_layer_zero_in_a_block_takes_the_inserts_layer() {
    let sheet = INSERTS.replace(
        "layers:\n  red: {color: red, lineweight: 0.5}",
        "linetypes:\n  DASHED: {pattern: [2, -1]}\nlayers:\n  red: {color: red, lineweight: 0.5}\n  hidden: {color: 30, linetype: DASHED, lineweight: 0.7}",
    )
    .replace("rotation: 90, attributes: {TAG: X, HIDDEN: Y}, layer: red}", "attributes: {TAG: X, HIDDEN: Y}, layer: hidden}");
    let list = draw_with_library("layer-zero", LIBRARY, &sheet);
    let line = &list.items[0];
    assert_eq!(line.color, Color(30));
    assert_eq!(lineweight(line), 0.7);
    // The block's line, 10 long, is doubled to 20 where the insert puts
    // it, and the linetype is laid out along that at its own lengths, as
    // AutoCAD 2026 plotted a made-up probe: not doubled with the block.
    let placed = linetype::along(&[[80.0, 50.0], [100.0, 50.0]], Some(&[2.0, -1.0]), false);
    let dashes = runs(line);
    assert_eq!(dashes.len(), placed.len(), "{dashes:?}");
    for (dash, placed) in dashes.iter().zip(&placed) {
        assert_eq!(dash.len(), placed.len());
        for (p, q) in dash.iter().zip(placed) {
            near(*p, *q);
        }
    }
    assert!(dashes.len() > 5, "{dashes:?}");
    // The circle keeps its own layer.
    assert_eq!(list.items[1].color, Color(5));
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
