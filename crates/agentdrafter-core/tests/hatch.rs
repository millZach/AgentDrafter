//! Solid and patterned hatches build, and reading the DWG back gives the
//! same boundary and the same pattern.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, parse_sheet_description, read_dwg, BuildRequest, DrawingContents, Hatch, HatchEdge,
    HatchLoop, Object, Point3, Sheet, Unit,
};

/// How far a number read back may be from the one written. A DWG holds
/// angles in radians and a hatch's pattern lines turned and enlarged, so
/// degrees and pattern lengths do not come back bit for bit.
const TOLERANCE: f64 = 1e-9;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-hatch")
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
        description,
        output: None,
        overwrite: false,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let sheet = parse_sheet_description(text).unwrap();
    let hatches = sheet
        .objects
        .iter()
        .filter(|o| matches!(o, Object::Hatch(_)))
        .count();
    assert_eq!(report.objects.get("hatch").copied().unwrap_or(0), hatches);
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.dwg_version, "AC1032");
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    (sheet, read)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE
}

fn same_point(a: Point3, b: Point3) -> bool {
    a == b
}

/// Whether two hatches are the same: the pattern's name, its lines' angles
/// and lengths, the scale and angle and the arcs' angles to [`TOLERANCE`];
/// every boundary point and radius bit for bit.
fn same_hatch(wrote: &Hatch, read: &Hatch) -> bool {
    let lines_same = wrote.pattern.lines.len() == read.pattern.lines.len()
        && wrote
            .pattern
            .lines
            .iter()
            .zip(&read.pattern.lines)
            .all(|(w, r)| {
                close(w.angle, r.angle)
                    && close(w.origin[0], r.origin[0])
                    && close(w.origin[1], r.origin[1])
                    && close(w.delta[0], r.delta[0])
                    && close(w.delta[1], r.delta[1])
                    && w.dashes.len() == r.dashes.len()
                    && w.dashes.iter().zip(&r.dashes).all(|(a, b)| close(*a, *b))
            });
    let loops_same = wrote.boundary.len() == read.boundary.len()
        && wrote
            .boundary
            .iter()
            .zip(&read.boundary)
            .all(|pair| match pair {
                (HatchLoop::Polyline { points: w }, HatchLoop::Polyline { points: r }) => w == r,
                (HatchLoop::Edges(w), HatchLoop::Edges(r)) => {
                    w.len() == r.len() && w.iter().zip(r).all(|(w, r)| same_edge(w, r))
                }
                _ => false,
            });
    wrote.pattern.name == read.pattern.name
        && lines_same
        && close(wrote.scale, read.scale)
        && close(wrote.angle, read.angle)
        && wrote.layer == read.layer
        && loops_same
}

fn same_edge(wrote: &HatchEdge, read: &HatchEdge) -> bool {
    match (wrote, read) {
        (
            HatchEdge::Line { from, to },
            HatchEdge::Line {
                from: read_from,
                to: read_to,
            },
        ) => same_point(*from, *read_from) && same_point(*to, *read_to),
        (
            HatchEdge::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            },
            HatchEdge::Arc {
                center: read_center,
                radius: read_radius,
                start_angle: read_start,
                end_angle: read_end,
            },
        ) => {
            same_point(*center, *read_center)
                && radius == read_radius
                && close(*start_angle, *read_start)
                && close(*end_angle, *read_end)
        }
        _ => false,
    }
}

/// Asserts that the DWG read back holds the sheet's objects in order, each
/// hatch the same as [`same_hatch`] says and every other object bit for bit.
#[track_caller]
fn assert_same_objects(sheet: &Sheet, read: &DrawingContents) {
    assert_eq!(
        read.objects.len(),
        sheet.objects.len(),
        "{:#?}",
        read.objects
    );
    for (i, (wrote, came_back)) in sheet.objects.iter().zip(&read.objects).enumerate() {
        let same = match (wrote, came_back) {
            (Object::Hatch(w), Object::Hatch(r)) => same_hatch(w, r),
            (Object::Arc(w), Object::Arc(r)) => {
                w.center == r.center
                    && w.radius == r.radius
                    && close(w.start_angle, r.start_angle)
                    && close(w.end_angle, r.end_angle)
                    && w.layer == r.layer
            }
            _ => wrote == came_back,
        };
        assert!(
            same,
            "object {i}: wrote {wrote:#?}, read back {came_back:#?}"
        );
    }
}

fn hatches(sheet: &Sheet) -> Vec<&Hatch> {
    sheet
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Hatch(h) => Some(h),
            _ => None,
        })
        .collect()
}

#[test]
fn the_hatch_example_reads_back_with_every_boundary_and_pattern() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hatch.yaml"))
            .unwrap();
    let (sheet, read) = build_and_read("example", &text);
    let hatches = hatches(&sheet);
    assert!(hatches.iter().any(|h| h.is_solid()), "a solid hatch");
    assert!(hatches.iter().any(|h| !h.is_solid()), "a patterned hatch");
    let loops: Vec<&HatchLoop> = hatches.iter().flat_map(|h| &h.boundary).collect();
    assert!(loops
        .iter()
        .any(|l| matches!(l, HatchLoop::Polyline { .. })));
    let edges: Vec<&HatchEdge> = loops
        .iter()
        .filter_map(|l| match l {
            HatchLoop::Edges(edges) => Some(edges),
            HatchLoop::Polyline { .. } => None,
        })
        .flatten()
        .collect();
    assert!(edges.iter().any(|e| matches!(e, HatchEdge::Line { .. })));
    assert!(edges.iter().any(|e| matches!(e, HatchEdge::Arc { .. })));
    assert_same_objects(&sheet, &read);
}

#[test]
fn solid_hatches_read_back_with_their_boundaries() {
    let (sheet, read) = build_and_read(
        "solid",
        "units: mm
layers: {fill: {color: 4}}
objects:
  - hatch:
      pattern: SOLID
      boundary:
        - polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 5, 0], [0, 5, 0]]}
  - hatch:
      pattern: SOLID
      boundary:
        - polyline: {points: [[0, 0, 0], [30, 0, 0], [30, 30, 0], [0, 30, 0]]}
        - polyline: {points: [[10, 10, 0], [20, 10, 0], [20, 20, 0], [10, 20, 0]]}
      layer: fill
  - hatch:
      pattern: SOLID
      boundary:
        - edges:
            - line: {from: [-20, 0, 0], to: [20, 0, 0]}
            - arc: {center: [0, 0, 0], radius: 20, start_angle: 0, end_angle: 180}
",
    );
    assert_same_objects(&sheet, &read);
    let Object::Hatch(hatch) = &read.objects[0] else {
        panic!("{:?}", read.objects[0])
    };
    assert!(hatch.is_solid());
    assert!(hatch.pattern.lines.is_empty());
    assert_eq!((hatch.scale, hatch.angle), (1.0, 0.0));
}

#[test]
fn patterned_hatches_read_back_with_their_pattern_scale_and_angle() {
    let (sheet, read) = build_and_read(
        "patterned",
        "units: mm
hatch_patterns:
  STRIPES: {lines: [{delta: [0, 2]}]}
  CROSS:
    lines:
      - {angle: 45, delta: [0, 3.175]}
      - {angle: 135, origin: [1.5, -0.25], delta: [0.5, 3.175]}
  MIXED:
    lines:
      - {angle: 30, origin: [2, 1], delta: [1.25, 4], dashes: [3, -1, 0, -1]}
      - {angle: 300, delta: [-2, -6], dashes: [-0.5, 2.5]}
objects:
  - hatch:
      pattern: STRIPES
      boundary:
        - polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]]}
  - hatch:
      pattern: CROSS
      scale: 2.5
      boundary:
        - polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]]}
  - hatch:
      pattern: MIXED
      scale: 0.75
      angle: 200
      boundary:
        - polyline: {points: [[0, 0, 0], [10, 0, 0], [5, 8, 0]]}
  - hatch:
      pattern: MIXED
      angle: -90
      boundary:
        - polyline: {points: [[0, 0, 0], [10, 0, 0], [5, 8, 0]]}
",
    );
    assert_same_objects(&sheet, &read);
    let Object::Hatch(turned) = &read.objects[3] else {
        panic!("{:?}", read.objects[3])
    };
    assert!(close(turned.angle, 270.0), "{turned:?}");
    assert_eq!(turned.pattern.name, "MIXED");
    assert_eq!(turned.pattern.lines.len(), 2);
}

/// A loop of lines and arcs reads back edge for edge: each line from and
/// to the same points, each arc the same, whichever way the loop walks it.
#[test]
fn loops_of_lines_and_arcs_read_back_walked_either_way() {
    let (sheet, read) = build_and_read(
        "edges",
        "units: mm
objects:
  # A slot: every arc walked forwards.
  - hatch:
      pattern: SOLID
      boundary:
        - edges:
            - line: {from: [0, 0, 0], to: [30, 0, 0]}
            - arc: {center: [30, 5, 0], radius: 5, start_angle: 270, end_angle: 90}
            - line: {from: [30, 10, 0], to: [0, 10, 0]}
            - arc: {center: [0, 5, 0], radius: 5, start_angle: 90, end_angle: 270}
  # A square with a round notch: the arc walked backwards.
  - hatch:
      pattern: SOLID
      boundary:
        - edges:
            - line: {from: [0, 0, 0], to: [40, 0, 0]}
            - line: {from: [40, 0, 0], to: [40, 40, 0]}
            - line: {from: [40, 40, 0], to: [30, 40, 0]}
            - arc: {center: [20, 40, 0], radius: 10, start_angle: 180, end_angle: 0}
            - line: {from: [10, 40, 0], to: [0, 40, 0]}
            - line: {from: [0, 40, 0], to: [0, 0, 0]}
  # Starting with an arc walked backwards.
  - hatch:
      pattern: SOLID
      boundary:
        - edges:
            - arc: {center: [0, 0, 0], radius: 8, start_angle: 0, end_angle: 180}
            - line: {from: [8, 0, 0], to: [8, -8, 0]}
            - line: {from: [8, -8, 0], to: [-8, -8, 0]}
            - line: {from: [-8, -8, 0], to: [-8, 0, 0]}
  # A disc of two arcs, as a circle is written.
  - hatch:
      pattern: SOLID
      boundary:
        - edges:
            - arc: {center: [5, 5, 0], radius: 2.5, start_angle: 0, end_angle: 180}
            - arc: {center: [5, 5, 0], radius: 2.5, start_angle: 180, end_angle: 0}
",
    );
    assert_same_objects(&sheet, &read);
}

/// A hatch lies at the z its boundary shares, and lengths with their own
/// unit reach the DWG converted, pattern lengths among them.
#[test]
fn a_hatch_keeps_its_elevation_and_converted_lengths() {
    let (sheet, read) = build_and_read(
        "elevation",
        "units: in
hatch_patterns:
  FINE: {lines: [{angle: 45, origin: [25.4 mm, 0], delta: [0, 3.175 mm], dashes: [6.35 mm, -1.5875mm]}]}
objects:
  - hatch:
      pattern: FINE
      boundary:
        - polyline: {points: [[0, 0, 2.5], [4, 0, 2.5], [4, 3, 2.5]]}
        - edges:
            - line: {from: [1.5, 0.5, 2.5], to: [2.5, 0.5, 2.5]}
            - arc: {center: [2.5, 1, 2.5], radius: 0.5, start_angle: 270, end_angle: 90}
            - line: {from: [2.5, 1.5, 2.5], to: [1.5, 0.5, 2.5]}
",
    );
    assert_eq!(read.units, Some(Unit::In));
    assert_same_objects(&sheet, &read);
    let Object::Hatch(hatch) = &read.objects[0] else {
        panic!("{:?}", read.objects[0])
    };
    let line = &hatch.pattern.lines[0];
    assert!(close(line.origin[0], 1.0), "{line:?}");
    assert!(close(line.delta[1], 0.125), "{line:?}");
    assert!(close(line.dashes[0], 0.25) && close(line.dashes[1], -0.0625));
    let HatchLoop::Polyline { points } = &hatch.boundary[0] else {
        panic!("{hatch:?}")
    };
    assert!(points.iter().all(|p| p.z == 2.5));
}
