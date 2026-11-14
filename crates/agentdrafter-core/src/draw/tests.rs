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
        ..Settings::default()
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
        ..Settings::default()
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

/// Each line of text gives a run of its characters along its baseline, in
/// the frame the text is set in: from the start of its baseline, one unit
/// of the font's advances along it and the capitals' height up.
#[test]
fn each_line_of_text_reads_as_a_run_along_its_baseline() {
    let list = draw(
        "units: mm\ntext_styles:\n  narrow: {font: romans.shx, width_factor: 0.8}\nobjects:\n  - text: {contents: AB C, position: [10, 20, 0], height: 2.5, style: narrow, rotation: 90}\n  - mtext: {contents: \"ONE\\nTWO\", position: [0, 0, 0], height: 3}\n",
    );
    let read: Vec<(usize, String)> = list
        .text
        .iter()
        .map(|r| (r.object, r.characters.iter().map(|&(c, _)| c).collect()))
        .collect();
    assert_eq!(
        read,
        [(0, "AB C".into()), (1, "ONE".into()), (1, "TWO".into())]
    );
    let turned = &list.text[0];
    near(turned.origin, [10.0, 20.0]);
    near(turned.across, [0.0, 2.0]);
    near(turned.up, [-2.5, 0.0]);
    // The lines of multiline text sit five thirds of the height apart.
    near(list.text[1].up, [0.0, 3.0]);
    assert!((list.text[1].origin[1] - list.text[2].origin[1] - 5.0).abs() < 1e-9);
}

/// An attribute's run is where the insert puts its text, and a text in a
/// block runs where the insert places it, enlarged and turned with it.
#[test]
fn an_inserts_text_reads_where_the_insert_puts_it() {
    let library = LIBRARY.replace(
        "      - attdef: {tag: TAG,",
        "      - text: {contents: FIXED, position: [0, 2, 0], height: 1}\n      - attdef: {tag: TAG,",
    );
    let list = draw_with_library("insert-text", &library, INSERTS);
    let read: Vec<String> = list
        .text
        .iter()
        .map(|r| r.characters.iter().map(|&(c, _)| c).collect())
        .collect();
    assert_eq!(read, ["FIXED", "X"]);
    assert!(list.text.iter().all(|r| r.object == 0));
    // The block's text from (0, 2), doubled about the base point (10, 0)
    // and turned a quarter round the insert's position (100, 50): 2 high,
    // running up the page.
    let fixed = &list.text[0];
    near(fixed.origin, [96.0, 30.0]);
    near(fixed.up, [-2.0, 0.0]);
    assert!(fixed.across[0].abs() < 1e-9 && fixed.across[1] > 0.0);
    // The attribute, 4 high, starts at (90, 30), as it draws.
    near(list.text[1].origin, [90.0, 30.0]);
    near(list.text[1].up, [-4.0, 0.0]);
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
        ("pdf.rs", include_str!("../pdf.rs")),
        ("plot_style.rs", include_str!("../plot_style.rs")),
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

const OWN_PROPERTIES: &str = r#"
units: mm
linetypes:
  DASHED: {pattern: [5, -2.5]}
layers:
  outline: {color: red, lineweight: 0.5}
objects:
  - line: {from: [0, 0, 0], to: [100, 0, 0], layer: outline}
  - line: {from: [0, 10, 0], to: [100, 10, 0], layer: outline, color: 3, linetype: DASHED, lineweight: 0.7}
  - line: {from: [0, 20, 0], to: [100, 20, 0], layer: outline, color: ByBlock, linetype: ByBlock, lineweight: Default}
  - line: {from: [0, 30, 0], to: [100, 30, 0], linetype: DASHED, linetype_scale: 2}
"#;

/// An object's own colour, linetype and lineweight win over its layer's;
/// `ByBlock` on the sheet itself draws white and continuous, and `Default`
/// at AutoCAD's default lineweight.
#[test]
fn an_objects_own_properties_win_over_its_layers() {
    let list = draw(OWN_PROPERTIES);
    let by_layer = items_of(&list, 0)[0];
    assert_eq!(by_layer.color, Color(1));
    assert_eq!(lineweight(by_layer), 0.5);
    assert_eq!(runs(by_layer).len(), 1);
    let own = items_of(&list, 1)[0];
    assert_eq!(own.color, Color(3));
    assert_eq!(lineweight(own), 0.7);
    let dashed = linetype::along(&[[0.0, 10.0], [100.0, 10.0]], Some(&[5.0, -2.5]), false);
    assert_eq!(runs(own), &dashed);
    let by_block = items_of(&list, 2)[0];
    assert_eq!(by_block.color, Color(7));
    assert_eq!(lineweight(by_block), DEFAULT_LINEWEIGHT);
    assert_eq!(runs(by_block).len(), 1);
}

/// A linetype's dashes and gaps are enlarged by the sheet's `ltscale`
/// times the object's own linetype scale.
#[test]
fn a_linetype_is_enlarged_by_the_sheets_ltscale_and_the_objects_own() {
    let at = |list: &DisplayList| runs(items_of(list, 3)[0]).clone();
    let doubled = linetype::along(&[[0.0, 30.0], [100.0, 30.0]], Some(&[10.0, -5.0]), false);
    assert_eq!(at(&draw(OWN_PROPERTIES)), doubled);
    let sheet = OWN_PROPERTIES.replace("units: mm\n", "units: mm\nltscale: 0.25\n");
    let halved = linetype::along(&[[0.0, 30.0], [100.0, 30.0]], Some(&[2.5, -1.25]), false);
    assert_eq!(at(&draw(&sheet)), halved);
    // The sheet's own scale reaches a line that takes its layer's linetype.
    let sheet = sheet
        .replace(
            "  - line: {from: [0, 30, 0], to: [100, 30, 0], linetype: DASHED, linetype_scale: 2}",
            "  - line: {from: [0, 30, 0], to: [100, 30, 0], layer: dashed}",
        )
        .replace("layers:\n", "layers:\n  dashed: {linetype: DASHED}\n");
    let quartered = linetype::along(&[[0.0, 30.0], [100.0, 30.0]], Some(&[1.25, -0.625]), false);
    assert_eq!(at(&draw(&sheet)), quartered);
}

const BY_BLOCK_LIBRARY: &str = r#"
units: mm
linetypes:
  DOTS: {pattern: [0, -1]}
layers:
  own: {color: blue}
blocks:
  B:
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}
      - line: {from: [0, 2, 0], to: [10, 2, 0], layer: own, linetype: DOTS}
      - attdef: {tag: TAG, position: [0, 5, 0], height: 2, color: ByBlock}
"#;

/// A block's `ByBlock` objects and attributes draw with the insert's own
/// colour, linetype and lineweight, or its layer's where the insert's are
/// `ByLayer`; a linetype a block's object names comes to the sheet with it.
#[test]
fn a_by_block_object_takes_the_inserts_properties() {
    let sheet = r#"
units: mm
block_library: blocks.yaml
linetypes:
  DASHED: {pattern: [2, -1]}
layers:
  red: {color: red, lineweight: 0.5}
objects:
  - insert: {block: B, position: [0, 0, 0], attributes: {TAG: X}, layer: red, color: 6, linetype: DASHED, lineweight: 0.7}
  - insert: {block: B, position: [0, 50, 0], attributes: {TAG: Y}, layer: red}
"#;
    let list = draw_with_library("by-block", BY_BLOCK_LIBRARY, sheet);
    let own = items_of(&list, 0);
    assert_eq!(own[0].color, Color(6));
    assert_eq!(lineweight(own[0]), 0.7);
    let dashed = linetype::along(&[[0.0, 0.0], [10.0, 0.0]], Some(&[2.0, -1.0]), false);
    assert_eq!(runs(own[0]), &dashed);
    assert_eq!(own[1].color, Color(5));
    let dotted = linetype::along(&[[0.0, 2.0], [10.0, 2.0]], Some(&[0.0, -1.0]), false);
    assert_eq!(runs(own[1]), &dotted);
    assert!(own[2..].iter().all(|i| i.color == Color(6)), "{own:?}");
    let by_layer = items_of(&list, 1);
    assert_eq!(by_layer[0].color, Color(1));
    assert_eq!(lineweight(by_layer[0]), 0.5);
    assert_eq!(runs(by_layer[0]).len(), 1);
    assert!(by_layer[2..].iter().all(|i| i.color == Color(1)));
}

/// A donut and a 2D solid fill their area, and a wide polyline fills its
/// band, each with no stroke of its own, as AutoCAD 2026 plotted a made-up
/// probe of them on a layer of 1 mm lineweight.
#[test]
fn donuts_solids_and_wide_polylines_fill_with_no_lineweight() {
    let list = draw(
        "units: mm\nlayers:\n  heavy: {color: red, lineweight: 1}\nobjects:\n\
         - donut: {center: [0, 0, 0], inside_diameter: 2, outside_diameter: 4, layer: heavy}\n\
         - solid: {corners: [[10, 0, 0], [14, 0, 0], [12, 3, 0]], layer: heavy}\n\
         - polyline: {points: [[20, 0, 0], [30, 0, 0]], width: 2, layer: heavy}\n",
    );
    for object in 0..3 {
        let items = items_of(&list, object);
        assert_eq!(items.len(), 1, "{items:?}");
        assert!(matches!(items[0].shape, Shape::Fill { .. }), "{items:?}");
        assert_eq!(items[0].color, Color(1));
    }
    // The donut's ring reaches from radius 1 to 2.
    for p in items_of(&list, 0)[0].points() {
        let r = p[0].hypot(p[1]);
        assert!((0.999..=2.001).contains(&r), "{p:?}");
    }
    let extents = list.extents().unwrap();
    near(extents.min, [-2.0, -2.0]);
    near(extents.max, [30.0, 3.0]);
}

const LAYER_STATES: &str = r#"
units: mm
layers:
  shown: {color: 1}
  off: {color: 2, on: false}
  frozen: {color: 3, frozen: true}
  trim: {color: 4, plot: false}
objects:
  - line: {from: [0, 0, 0], to: [10, 0, 0], layer: shown}
  - line: {from: [0, 20, 0], to: [10, 20, 0], layer: off}
  - line: {from: [0, 40, 0], to: [10, 40, 0], layer: frozen}
  - line: {from: [-5, -5, 0], to: [15, -5, 0], layer: trim}
"#;

/// Off and frozen layers draw nothing; a layer that does not plot draws on
/// a screen and not on paper. Extents count off and no-plot layers, not
/// frozen ones, as AutoCAD's do.
#[test]
fn a_layers_states_decide_what_draws_and_what_the_extents_take_in() {
    let sheet = parse_sheet_description(LAYER_STATES).unwrap();
    let fonts = SheetFonts::find(&sheet, &Settings::default());
    let screen = draw_sheet_for(&sheet, &fonts, 0.001, MOST_HATCH_DASHES, Purpose::Screen);
    let paper = draw_sheet_for(&sheet, &fonts, 0.001, MOST_HATCH_DASHES, Purpose::Plot);
    let drawn = |list: &DisplayList| {
        let mut objects: Vec<usize> = list.items.iter().map(|i| i.object).collect();
        objects.dedup();
        objects
    };
    assert_eq!(drawn(&screen), [0, 3]);
    assert_eq!(drawn(&paper), [0]);
    for list in [&screen, &paper] {
        let extents = list.extents().unwrap();
        // The trim line and the line that is off count; the frozen one does not.
        assert!(
            extents.min[0] <= -5.0 && extents.min[1] <= -5.0,
            "{extents:?}"
        );
        assert!((extents.max[1] - 20.0).abs() < 0.5, "{extents:?}");
    }
}

/// An insert on a frozen layer draws nothing; one on a layer that is off
/// hides only what takes its layer, as AutoCAD does.
#[test]
fn an_inserts_layer_hides_what_takes_it() {
    let library = r#"
units: mm
layers:
  own: {color: blue}
blocks:
  B:
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0]}
      - line: {from: [0, 5, 0], to: [10, 5, 0], layer: own}
"#;
    let sheet = |state: &str| {
        format!(
            "units: mm\nblock_library: blocks.yaml\nlayers:\n  host: {{color: red, {state}}}\nobjects:\n  - insert: {{block: B, position: [0, 0, 0], layer: host}}\n"
        )
    };
    let off = draw_with_library("insert-off", library, &sheet("on: false"));
    let colors: Vec<Color> = off.items.iter().map(|i| i.color).collect();
    assert_eq!(colors, [Color(5)], "only the block's own-layer line draws");
    let frozen = draw_with_library("insert-frozen", library, &sheet("frozen: true"));
    assert!(frozen.items.is_empty() && frozen.extents().is_none());
}
