//! Each object type builds, and reading the DWG back gives the same
//! geometry, on the same layer, with the same layers and linetypes.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, parse_sheet_description, read_dwg, BuildRequest, DrawingContents, Layer, Object, Point3,
    Sheet, Unit,
};

/// How far an angle read back may be from the one written, in degrees. A
/// DWG holds angles in radians, so degrees do not come back bit for bit.
const ANGLE_TOLERANCE: f64 = 1e-9;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-geometry")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds `text` as a sheet description and returns what the sheet
/// description says and what the DWG reads back as.
fn build_and_read(test: &str, text: &str) -> (Sheet, DrawingContents) {
    let dir = scratch(test);
    let description = dir.join("sheet.yaml");
    fs::write(&description, text).unwrap();
    let report = build(&BuildRequest {
        description: description.clone(),
        output: None,
        overwrite: false,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let sheet = parse_sheet_description(text).unwrap();
    let mut counts = std::collections::BTreeMap::new();
    for object in &sheet.objects {
        *counts.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    assert_eq!(report.objects, counts);
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.dwg_version, "AC1032");
    assert_eq!(read.units, Some(sheet.units));
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.layers_not_read, 0, "{read:?}");
    assert_eq!(read.linetypes_not_read, 0, "{read:?}");
    (sheet, read)
}

/// Asserts that the DWG read back holds the sheet's objects, in order:
/// every coordinate and length bit for bit, angles to [`ANGLE_TOLERANCE`].
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
            (Object::Arc(w), Object::Arc(r)) => {
                w.center == r.center
                    && w.radius == r.radius
                    && w.layer == r.layer
                    && w.properties == r.properties
                    && close(w.start_angle, r.start_angle)
                    && close(w.end_angle, r.end_angle)
            }
            (Object::Ellipse(w), Object::Ellipse(r)) => {
                w.center == r.center
                    && w.major_axis == r.major_axis
                    && w.ratio == r.ratio
                    && w.layer == r.layer
                    && w.properties == r.properties
                    && close(w.start_angle, r.start_angle)
                    && close(w.end_angle, r.end_angle)
            }
            _ => wrote == came_back,
        };
        assert!(same, "object {i}: wrote {wrote:?}, read back {came_back:?}");
    }
}

/// The layers a DWG reads back as: layer 0 first, as every drawing has it,
/// then the sheet's own in order.
fn layers_in_the_dwg(sheet: &Sheet) -> Vec<Layer> {
    let zero = sheet
        .layers
        .iter()
        .find(|l| l.name == "0")
        .cloned()
        .unwrap_or_else(|| Layer::new("0"));
    std::iter::once(zero)
        .chain(sheet.layers.iter().filter(|l| l.name != "0").cloned())
        .collect()
}

#[test]
fn the_geometry_example_reads_back_with_its_layers_linetypes_and_objects() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/geometry.yaml"),
    )
    .unwrap();
    let (sheet, read) = build_and_read("example", &text);

    let types: std::collections::BTreeSet<&str> =
        sheet.objects.iter().map(Object::type_name).collect();
    assert_eq!(
        types.into_iter().collect::<Vec<_>>(),
        ["arc", "circle", "ellipse", "line", "polyline", "spline"],
        "the example holds every object type"
    );
    assert_eq!(read.linetypes, sheet.linetypes);
    assert_eq!(read.layers, layers_in_the_dwg(&sheet));
    assert_same_objects(&sheet, &read);
}

#[test]
fn lines_read_back_in_3d_on_their_layer() {
    let (sheet, read) = build_and_read(
        "lines",
        "units: mm
layers: {walls: {}}
objects:
  - line: {from: [0, 0, 0], to: [100, 50, 0]}
  - line: {from: [12.5, -3.25, 7], to: [0.1, 1e-3, -2.5], layer: walls}
",
    );
    assert_same_objects(&sheet, &read);
    assert_eq!(read.objects[1].layer(), "walls");
}

#[test]
fn polylines_read_back_flat_or_3d_open_or_closed() {
    let (sheet, read) = build_and_read(
        "polylines",
        "units: mm
objects:
  - polyline: {points: [[0, 0, 0], [10, 0, 0]]}
  - polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]], closed: true}
  - polyline: {points: [[1.5, 2.25, -4], [3, 4, -4], [5, 0.125, -4]]}
  - polyline: {points: [[0, 0, 0], [10, 0, 5], [10, 10, 10]]}
  - polyline: {points: [[0, 0, 1], [10, 0, 2], [10, 10, 3]], closed: true}
",
    );
    assert_same_objects(&sheet, &read);
}

#[test]
fn arcs_read_back_with_their_angles() {
    let (sheet, read) = build_and_read(
        "arcs",
        "units: mm
objects:
  - arc: {center: [0, 0, 0], radius: 10, start_angle: 0, end_angle: 90}
  - arc: {center: [5, -5, 2], radius: 2.5, start_angle: 270, end_angle: 45}
  - arc: {center: [0, 0, 0], radius: 1, start_angle: 33.3, end_angle: 300.7}
",
    );
    assert_same_objects(&sheet, &read);
}

#[test]
fn circles_read_back() {
    let (sheet, read) = build_and_read(
        "circles",
        "units: mm
objects:
  - circle: {center: [0, 0, 0], radius: 10}
  - circle: {center: [-7.5, 3, 12], radius: 0.001}
",
    );
    assert_same_objects(&sheet, &read);
}

#[test]
fn ellipses_read_back_whole_or_in_part() {
    let (sheet, read) = build_and_read(
        "ellipses",
        "units: mm
objects:
  - ellipse: {center: [0, 0, 0], major_axis: [10, 0, 0], ratio: 0.5}
  - ellipse: {center: [1, 2, 3], major_axis: [-3, 4, 0], ratio: 1}
  - ellipse: {center: [0, 0, 0], major_axis: [10, 10, 0], ratio: 0.25, start_angle: 20, end_angle: 200}
  - ellipse: {center: [0, 0, 0], major_axis: [0, 8, 0], ratio: 0.75, start_angle: 300, end_angle: 60}
",
    );
    assert_same_objects(&sheet, &read);
}

#[test]
fn splines_read_back_by_fit_points_or_control_points() {
    let (sheet, read) = build_and_read(
        "splines",
        "units: mm
objects:
  - spline: {fit_points: [[0, 0, 0], [10, 10, 0], [20, 0, 0], [30, 10, 0]]}
  - spline: {fit_points: [[0, 0, 0], [5, 5, 5], [10, 0, 10]]}
  - spline: {control_points: [[0, 0, 0], [10, 10, 0], [20, 0, 0], [30, 10, 0]]}
  - spline: {control_points: [[0, 0, 0], [10, 10, 0], [20, 0, 0]], degree: 2}
  - spline: {control_points: [[0, 0, 0], [10, 10, 0]], degree: 1}
  - spline: {control_points: [[0, 0, 0], [1, 2, 0], [2, 0, 1], [3, 2, 1], [4, 0, 2], [5, 2, 2]], degree: 5}
",
    );
    assert_same_objects(&sheet, &read);
}

#[test]
fn layers_and_linetypes_read_back_including_layer_zero() {
    let (sheet, read) = build_and_read(
        "layers",
        "units: in
linetypes:
  PHANTOM: {description: Phantom ___ _ _ ___, pattern: [1.25, -0.25, 0.25, -0.25, 0.25, -0.25]}
  dots: {pattern: [0, -2 mm]}
layers:
  '0': {color: 2, linetype: dots}
  heavy: {color: 250, lineweight: 2.11}
  light: {color: blue, linetype: PHANTOM, lineweight: 0}
objects:
  - circle: {center: [0, 0, 0], radius: 1, layer: heavy}
  - circle: {center: [0, 0, 0], radius: 2, layer: light}
  - circle: {center: [0, 0, 0], radius: 3}
",
    );
    assert_eq!(read.linetypes, sheet.linetypes);
    assert_eq!(read.linetypes[1].pattern, vec![0.0, -2.0 / 25.4]);
    assert_eq!(read.layers, layers_in_the_dwg(&sheet));
    assert_eq!(read.layers[0].color, 2);
    assert_same_objects(&sheet, &read);
}

#[test]
fn lengths_with_their_own_unit_reach_the_dwg_converted() {
    let (_, read) = build_and_read(
        "converted",
        "units: in
objects:
  - circle: {center: [25.4 mm, 0, 0], radius: 1 ft}
",
    );
    let [Object::Circle(circle)] = read.objects.as_slice() else {
        panic!("{:?}", read.objects)
    };
    assert_eq!(circle.center, Point3::new(1.0, 0.0, 0.0));
    assert_eq!(circle.radius, 12.0);
    assert_eq!(read.units, Some(Unit::In));
}

#[test]
fn an_objects_own_properties_and_the_sheets_ltscale_read_back() {
    let (sheet, read) = build_and_read(
        "own-properties",
        "units: mm
ltscale: 2.5
linetypes:
  DASHED: {pattern: [5, -2.5]}
layers:
  wiring: {color: red}
objects:
         - line: {from: [0, 0, 0], to: [10, 0, 0], layer: wiring, color: 3, linetype: DASHED, lineweight: 0.35, linetype_scale: 0.5}
         - line: {from: [0, 1, 0], to: [10, 1, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}
         - polyline: {points: [[0, 2, 0], [10, 2, 0]], linetype: Continuous, lineweight: Default}
         - arc: {center: [0, 0, 0], radius: 5, start_angle: 0, end_angle: 90, color: 250}
         - circle: {center: [0, 0, 0], radius: 3, lineweight: 2.11}
         - ellipse: {center: [0, 0, 0], major_axis: [4, 0, 0], ratio: 0.5, color: ByLayer}
         - spline: {fit_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]], linetype_scale: 3}
         - hatch: {pattern: SOLID, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}], color: 8}
         - text: {contents: A, position: [0, 0, 0], height: 1, color: 2}
         - mtext: {contents: B, position: [0, 0, 0], height: 1, lineweight: 0.5}
",
    );
    assert_eq!(read.ltscale, 2.5);
    assert_eq!(read.objects.len(), sheet.objects.len());
    for (wrote, came_back) in sheet.objects.iter().zip(&read.objects) {
        assert_eq!(wrote.properties(), came_back.properties(), "{wrote:?}");
    }
    assert_same_objects(&sheet, &read);
}

#[test]
fn arcs_widths_donuts_and_solids_read_back() {
    let (sheet, read) = build_and_read(
        "arcs-widths",
        "units: mm
objects:
         - polyline: {points: [{point: [0, 0, 0], bulge: 0.5, start_width: 1, end_width: 0.5}, {point: [10, 0, 0], arc_angle: -90}, [10, 10, 0]], closed: true}
         - polyline: {points: [[0, 20, 2], [10, 20, 2]], width: 0.75}
         - donut: {center: [30, 0, 0], inside_diameter: 1, outside_diameter: 3}
         - solid: {corners: [[40, 0, 0], [44, 0, 0], [44, 2, 0], [40, 2, 0]]}
         - solid: {corners: [[50, 0, 0], [54, 0, 0], [52, 3, 0]]}
",
    );
    assert_eq!(read.objects.len(), 5);
    assert_eq!(read.objects[0], sheet.objects[0]);
    assert_eq!(read.objects[1], sheet.objects[1]);
    // A donut is written as AutoCAD's DONUT writes it, and reads back as
    // that polyline.
    let Object::Donut(donut) = &sheet.objects[2] else {
        panic!()
    };
    assert_eq!(read.objects[2], Object::Polyline(donut.polyline()));
    assert_eq!(read.objects[3], sheet.objects[3]);
    assert_eq!(read.objects[4], sheet.objects[4]);
}
