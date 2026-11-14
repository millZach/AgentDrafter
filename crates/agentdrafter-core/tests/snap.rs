//! Object snaps and an object's geometry, as queries. Each point AutoCAD's
//! object snaps find comes with its kind and its place in the object, an
//! insert's in its block, nested inserts too, placed as the insert places
//! them; snaps between two objects take the other object, or a snap of it,
//! never coordinates. Every value here was also asked of AutoCAD 2026 on a
//! made-up sheet of every kind of object (see the pull request).

use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    query_with, Answer, ErrorCode, ObjectGeometry, ObjectRef, Point3, Query, QueryReport,
    QueryRequest, Selection, Settings, SnapKind, SnapPoint, SnapRef,
};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-snap")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The example sheet: a strip of two lugs, turned 90° and twice as large,
/// a wire to the first lug's stub, and a plate with a rounded corner.
fn lugs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/snaps/lugs.yaml")
}

/// A made-up sheet of every kind of object, in a folder of its own.
fn every_kind(test: &str) -> PathBuf {
    let dir = scratch(test);
    let sheet = dir.join("kinds.yaml");
    fs::write(
        &sheet,
        r#"units: mm
objects:
  - line: {id: line, from: [0, 0, 0], to: [10, 0, 0]}
  - line: {id: high, from: [0, -5, 4], to: [10, 5, 4]}
  - circle: {id: circle, center: [20, 10, 0], radius: 5}
  - arc: {id: arc, center: [50, 0, 0], radius: 10, start_angle: 45, end_angle: 135}
  - ellipse: {id: ellipse, center: [80, 0, 0], major_axis: [0, 6, 0], ratio: 0.5}
  - spline: {id: spline, fit_points: [[100, 0, 0], [110, 10, 0], [120, 0, 0]]}
  - solid: {id: solid, corners: [[0, 30, 0], [10, 30, 0], [5, 40, 0]]}
  - donut: {id: donut, center: [30, 30, 0], inside_diameter: 2, outside_diameter: 6}
  - hatch:
      id: hatch
      pattern: SOLID
      boundary:
        - polyline: {points: [[50, 30, 0], [70, 30, 0], [70, 50, 0], [50, 50, 0]]}
        - polyline: {points: [[55, 35, 0], [60, 35, 0], [60, 40, 0], [55, 40, 0]]}
  - text: {id: justified, contents: IXI, position: [0, 60, 0], height: 5, justify: middle_center}
  - text: {id: left, contents: IXI, position: [0, 70, 0], height: 5}
  - table: {id: table, position: [100, 60, 0], columns: [10, 20], rows: [5, 5], cells: [[A, B], [C, D]]}
  - leader: {id: leader, points: [[150, 60, 0], [160, 40, 0]], annotation: {text: NOTE, height: 2}}
"#,
    )
    .unwrap();
    sheet
}

fn ask(source: &Path, query: Query) -> QueryReport {
    try_ask(source, query).unwrap_or_else(|e| panic!("{e:#?}"))
}

fn try_ask(source: &Path, query: Query) -> Result<QueryReport, agentdrafter_core::Error> {
    query_with(
        &QueryRequest {
            source: source.to_owned(),
            sheet: None,
            query,
        },
        &Settings::default(),
    )
}

fn snap(source: &Path, request: SnapRef) -> (QueryReport, Vec<SnapPoint>) {
    let report = ask(source, Query::Snap(request));
    let Answer::Snap { points, .. } = &report.answer else {
        panic!("not a snap: {report:#?}");
    };
    let points = points.clone();
    (report, points)
}

fn of(id: &str) -> SnapRef {
    SnapRef {
        id: Some(id.to_owned()),
        ..SnapRef::default()
    }
}

fn kind(id: &str, kind: SnapKind) -> SnapRef {
    SnapRef {
        snap: Some(kind),
        ..of(id)
    }
}

fn close(at: Point3, want: [f64; 3]) {
    let got = [at.x, at.y, at.z];
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "{got:?} is not {want:?}");
    }
}

/// The point of `kind` named `which` among `points`.
fn named<'a>(points: &'a [SnapPoint], kind: SnapKind, which: Option<&str>) -> &'a SnapPoint {
    points
        .iter()
        .find(|p| p.snap == kind && p.which.as_deref() == which)
        .unwrap_or_else(|| panic!("no {kind:?} {which:?} in {points:#?}"))
}

#[test]
fn an_insert_snaps_to_its_blocks_own_geometry_placed_nested_inserts_too() {
    let (report, points) = snap(&lugs(), of("lugs/strip"));
    assert_eq!(
        report.message,
        "29 snap points of the insert `lugs/strip`, on sheet `lugs`: 8 endpoints, 6 midpoints, 2 \
         centers, 8 quadrants, 3 insertion points, 2 nodes"
    );
    // The first lug's stub ends where the wire does: STRIP's objects[0] is
    // an insert of LUG, whose objects[1] is the stub, turned 90° and twice
    // as large about the strip's position.
    let stub = named(
        &points,
        SnapKind::Endpoint,
        Some("objects[0].objects[1].to"),
    );
    close(stub.at, [50.0, 52.0, 0.0]);
    assert_eq!(stub.blocks, ["STRIP", "LUG"]);
    // A quadrant turns with the insert that turns its circle.
    let quadrant = named(&points, SnapKind::Quadrant, Some("objects[1].objects[0].0"));
    close(quadrant.at, [34.0, 43.0, 0.0]);
    // The insert's own point, and each attribute's where it is drawn; an
    // insert nested in the block is part of the strip, as in AutoCAD.
    close(
        named(&points, SnapKind::Insertion, Some("position")).at,
        [50.0, 40.0, 0.0],
    );
    close(
        named(&points, SnapKind::Insertion, Some("objects[0].objects[2]")).at,
        [44.0, 40.0, 0.0],
    );
    assert!(!points
        .iter()
        .any(|p| p.which.as_deref() == Some("objects[0].position")));

    // Asked by its place, the one point comes back.
    let (report, points) = snap(
        &lugs(),
        SnapRef {
            which: Some("objects[0].objects[1].to".to_owned()),
            ..kind("lugs/strip", SnapKind::Endpoint)
        },
    );
    assert_eq!(points.len(), 1);
    assert_eq!(
        report.message,
        "the endpoint of the insert `lugs/strip`, on sheet `lugs`: [50, 52, 0]"
    );
}

#[test]
fn a_wire_meets_the_lug_it_ends_on() {
    let (report, points) = snap(
        &lugs(),
        SnapRef {
            with: Some(ObjectRef {
                id: Some("lugs/strip".to_owned()),
                name: None,
            }),
            ..kind("lugs/wire", SnapKind::Intersection)
        },
    );
    assert_eq!(
        report.message,
        "2 intersections of the line `lugs/wire` with the insert `lugs/strip`, on sheet `lugs`"
    );
    // In the order they lie along the wire, named by it.
    let frame = named(&points, SnapKind::Intersection, Some("0"));
    assert_eq!(frame.with_part.as_deref(), Some("objects[2].segments[2]"));
    let end = named(&points, SnapKind::Intersection, Some("1"));
    close(end.at, [50.0, 52.0, 0.0]);
    assert_eq!(end.with_part.as_deref(), Some("objects[0].objects[1]"));
}

#[test]
fn a_polylines_arc_segment_has_its_midpoint_on_the_arc_and_its_centre() {
    let (_, points) = snap(&lugs(), of("lugs/plate"));
    close(
        named(&points, SnapKind::Center, Some("segments[1]")).at,
        [30.0, 10.0, 0.0],
    );
    let half = 10.0 / 2f64.sqrt();
    close(
        named(&points, SnapKind::Midpoint, Some("segments[1]")).at,
        [30.0 + half, 10.0 - half, 0.0],
    );
    close(
        named(&points, SnapKind::Quadrant, Some("segments[1].0")).at,
        [40.0, 10.0, 0.0],
    );
    close(
        named(&points, SnapKind::Quadrant, Some("segments[1].270")).at,
        [30.0, 0.0, 0.0],
    );
    // A closed polyline's last segment runs back to its first point.
    close(
        named(&points, SnapKind::Midpoint, Some("segments[4]")).at,
        [0.0, 15.0, 0.0],
    );
}

#[test]
fn each_kind_of_object_snaps_as_autocad_snaps_it() {
    let sheet = every_kind("every-kind");
    let own = |id: &str| snap(&sheet, of(&format!("kinds/{id}"))).1;

    let line = own("line");
    close(
        named(&line, SnapKind::Endpoint, Some("to")).at,
        [10.0, 0.0, 0.0],
    );
    close(named(&line, SnapKind::Midpoint, None).at, [5.0, 0.0, 0.0]);

    let circle = own("circle");
    close(named(&circle, SnapKind::Center, None).at, [20.0, 10.0, 0.0]);
    close(
        named(&circle, SnapKind::Quadrant, Some("90")).at,
        [20.0, 15.0, 0.0],
    );
    assert_eq!(circle.len(), 5);

    // An arc has only the quadrants that lie on it.
    let arc = own("arc");
    let quadrants: Vec<_> = arc
        .iter()
        .filter(|p| p.snap == SnapKind::Quadrant)
        .collect();
    assert_eq!(quadrants.len(), 1);
    close(quadrants[0].at, [50.0, 10.0, 0.0]);
    close(named(&arc, SnapKind::Midpoint, None).at, [50.0, 10.0, 0.0]);

    // An ellipse's quadrants are the ends of its axes.
    let ellipse = own("ellipse");
    close(
        named(&ellipse, SnapKind::Quadrant, Some("0")).at,
        [80.0, 6.0, 0.0],
    );
    close(
        named(&ellipse, SnapKind::Quadrant, Some("90")).at,
        [77.0, 0.0, 0.0],
    );
    assert!(!ellipse.iter().any(|p| p.snap == SnapKind::Endpoint));

    // A spline's ends are named by its points; its midpoint is halfway
    // along it, here on its mirror line.
    let spline = own("spline");
    close(
        named(&spline, SnapKind::Endpoint, Some("fit_points[2]")).at,
        [120.0, 0.0, 0.0],
    );
    assert!((named(&spline, SnapKind::Midpoint, None).at.x - 110.0).abs() < 1e-6);

    // A solid's edges run round its outline, the last back to the first.
    let solid = own("solid");
    close(
        named(&solid, SnapKind::Midpoint, Some("segments[2]")).at,
        [2.5, 35.0, 0.0],
    );

    // A donut is two half circles round its middle circle.
    let donut = own("donut");
    close(
        named(&donut, SnapKind::Endpoint, Some("points[1]")).at,
        [28.0, 30.0, 0.0],
    );
    close(
        named(&donut, SnapKind::Quadrant, Some("90")).at,
        [30.0, 32.0, 0.0],
    );
    close(named(&donut, SnapKind::Center, None).at, [30.0, 30.0, 0.0]);

    // A hatch is not snapped to, as AutoCAD does not by default.
    let (report, hatch) = snap(&sheet, of("kinds/hatch"));
    assert!(hatch.is_empty());
    assert!(
        report
            .message
            .starts_with("nothing matches: a hatch has no point"),
        "{}",
        report.message
    );

    // Justified text has a node where its baseline starts; left text none.
    let justified = own("justified");
    close(
        named(&justified, SnapKind::Insertion, None).at,
        [0.0, 60.0, 0.0],
    );
    let node = named(&justified, SnapKind::Node, None).at;
    assert!(node.x < 0.0 && (node.y - 57.5).abs() < 1e-9, "{node:?}");
    assert!(!own("left").iter().any(|p| p.snap == SnapKind::Node));

    // A table's borders run whole along each row and column.
    let table = own("table");
    close(
        named(&table, SnapKind::Insertion, None).at,
        [100.0, 60.0, 0.0],
    );
    close(
        named(&table, SnapKind::Midpoint, Some("borders[0]")).at,
        [115.0, 60.0, 0.0],
    );

    // A leader steeper than 15° is hooked before its annotation.
    let leader = own("leader");
    close(
        named(&leader, SnapKind::Endpoint, Some("hook")).at,
        [157.5, 40.0, 0.0],
    );
}

#[test]
fn perpendicular_tangent_and_nearest_are_taken_from_a_snap_of_another_object() {
    let sheet = every_kind("from");
    let center = |id: &str| SnapRef {
        snap: Some(SnapKind::Center),
        ..of(id)
    };
    // A perpendicular reaches past a line's end, as AutoCAD's does.
    let (report, points) = snap(
        &sheet,
        SnapRef {
            from: Some(Box::new(center("kinds/circle"))),
            ..kind("kinds/line", SnapKind::Perpendicular)
        },
    );
    close(points[0].at, [20.0, 0.0, 0.0]);
    let Answer::Snap {
        from: Some(from), ..
    } = &report.answer
    else {
        panic!("{report:#?}");
    };
    close(from.at, [20.0, 10.0, 0.0]);
    // The nearest point keeps to the line.
    let (_, points) = snap(
        &sheet,
        SnapRef {
            from: Some(Box::new(center("kinds/circle"))),
            ..kind("kinds/line", SnapKind::Nearest)
        },
    );
    assert_eq!(points.len(), 1);
    close(points[0].at, [10.0, 0.0, 0.0]);
    // Two tangents from a line's end to a circle, the nearer first.
    let (_, points) = snap(
        &sheet,
        SnapRef {
            from: Some(Box::new(SnapRef {
                snap: Some(SnapKind::Endpoint),
                which: Some("from".to_owned()),
                ..of("kinds/line")
            })),
            ..kind("kinds/circle", SnapKind::Tangent)
        },
    );
    assert_eq!(points.len(), 2);
    for p in &points {
        let (dx, dy) = (p.at.x - 20.0, p.at.y - 10.0);
        assert!((dx.hypot(dy) - 5.0).abs() < 1e-9);
        // The radius there is square to the line from the line's end.
        assert!((dx * p.at.x + dy * p.at.y).abs() < 1e-9);
    }
    assert_eq!(points[0].which.as_deref(), Some("0"));
}

#[test]
fn an_apparent_intersection_is_seen_from_above_and_an_intersection_meets_in_space() {
    let sheet = every_kind("apparent");
    let with = |request: SnapRef| SnapRef {
        with: Some(ObjectRef {
            id: Some("kinds/line".to_owned()),
            name: None,
        }),
        ..request
    };
    let (report, points) = snap(&sheet, with(kind("kinds/high", SnapKind::Intersection)));
    assert!(points.is_empty());
    assert_eq!(
        report.message,
        "nothing matches: the line `kinds/high` and the line `kinds/line` do not cross"
    );
    let (_, points) = snap(
        &sheet,
        with(kind("kinds/high", SnapKind::ApparentIntersection)),
    );
    // On the first object, at its height.
    close(points[0].at, [5.0, 0.0, 4.0]);
}

#[test]
fn a_snap_that_cannot_be_answered_as_asked_says_which_field() {
    let sheet = every_kind("errors");
    let error = |request: SnapRef| try_ask(&sheet, Query::Snap(request)).unwrap_err();

    let e = error(SnapRef {
        which: Some("points[9]".to_owned()),
        ..kind("kinds/line", SnapKind::Endpoint)
    });
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("query.which"));
    assert!(
        e.message.contains("its endpoints: from, to"),
        "{}",
        e.message
    );

    // A point to snap from is one point.
    let e = error(SnapRef {
        from: Some(Box::new(kind("kinds/line", SnapKind::Endpoint))),
        ..kind("kinds/circle", SnapKind::Perpendicular)
    });
    assert_eq!(e.code, ErrorCode::BadValue);
    assert_eq!(e.field.as_deref(), Some("query.from.which"));

    let e = error(kind("kinds/line", SnapKind::Intersection));
    assert_eq!(e.field.as_deref(), Some("query.with"));
    let e = error(kind("kinds/line", SnapKind::Tangent));
    assert_eq!(e.field.as_deref(), Some("query.from"));
    let e = error(SnapRef {
        name: Some("x".to_owned()),
        ..of("kinds/line")
    });
    assert_eq!(e.code, ErrorCode::Usage);

    // An object no ID names is not an error: nothing matches.
    let (report, points) = snap(&sheet, of("kinds/nope"));
    assert!(points.is_empty());
    assert_eq!(
        report.message,
        "nothing matches: no object on the sheet `kinds` has the ID `kinds/nope`, the object to snap to"
    );
    // Nor is a snap the object has none of.
    let (report, _) = snap(&sheet, kind("kinds/line", SnapKind::Center));
    assert_eq!(
        report.message,
        "nothing matches: a line has no center snap, as in AutoCAD: `kinds/line` has none"
    );
}

fn geometry(source: &Path, id: &str) -> ObjectGeometry {
    let report = ask(
        source,
        Query::Geometry(Selection {
            id: Some(id.to_owned()),
            ..Selection::default()
        }),
    );
    let Answer::Geometry { objects, .. } = report.answer else {
        panic!("not a geometry");
    };
    objects.into_iter().next().expect("one object")
}

fn near(a: Option<f64>, b: f64) {
    let a = a.expect("a value");
    assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "{a} is not {b}");
}

#[test]
fn an_objects_length_area_extents_and_angles_are_autocads() {
    // The plate: 30 straight, a quarter of a circle of 10, 20, 40 and 30.
    let plate = geometry(&lugs(), "lugs/plate");
    near(plate.measures.length, 120.0 + 5.0 * PI);
    near(plate.measures.area, 1200.0 - (100.0 - 25.0 * PI));
    let extents = plate.measures.extents.unwrap();
    assert_eq!((extents.min, extents.max), ([0.0; 3], [40.0, 30.0, 0.0]));
    assert_eq!(plate.without, ["angle"]);

    let sheet = every_kind("geometry");
    let get = |id: &str| geometry(&sheet, &format!("kinds/{id}"));
    let line = get("line");
    near(line.measures.length, 10.0);
    near(line.measures.angle, 0.0);
    near(get("high").measures.angle, 45.0);
    let circle = get("circle");
    near(circle.measures.length, 10.0 * PI);
    near(circle.measures.area, 25.0 * PI);
    let arc = get("arc");
    near(arc.measures.length, 10.0 * PI / 2.0);
    near(arc.measures.start_angle, 45.0);
    near(arc.measures.end_angle, 135.0);
    near(arc.measures.total_angle, 90.0);
    assert_eq!(arc.without, ["area"]);
    near(get("ellipse").measures.area, PI * 6.0 * 3.0);
    // A hole inside the outer loop is not filled.
    near(get("hatch").measures.area, 400.0 - 25.0);
    near(get("solid").measures.area, 50.0);
    // A donut is measured along its middle circle, and its extents are its
    // outside circle's, as AutoCAD's are.
    let donut = get("donut");
    near(donut.measures.length, 2.0 * PI * 2.0);
    near(donut.measures.area, PI * 4.0);
    let extents = donut.measures.extents.unwrap();
    assert!((extents.min[0] - 27.0).abs() < 1e-6 && (extents.max[1] - 33.0).abs() < 1e-6);
    // Text has none of them: its extents are its font's, which `query
    // extents` gives for the sheet.
    assert_eq!(
        get("justified").without,
        ["length", "area", "extents", "angle"]
    );
}

/// A made-up sheet of the cases a review found: a parabola and a point
/// above it, a hatch whose hole all but fills it, lines that meet end to
/// end or are seen end on, a line tangent to an ellipse, and a line through
/// a polyline that doubles back.
fn review(test: &str) -> PathBuf {
    let dir = scratch(test);
    let sheet = dir.join("review.yaml");
    fs::write(
        &sheet,
        r#"units: mm
objects:
  - spline: {id: parabola, degree: 2, control_points: [[-1000, 1000, 0], [0, -1000, 0], [1000, 1000, 0]]}
  - circle: {id: above, center: [0, 500.9, 0], radius: 1}
  - hatch:
      id: ring
      pattern: SOLID
      boundary:
        - edges:
            - arc: {center: [0, 5000, 0], radius: 1000, start_angle: 0, end_angle: 180}
            - arc: {center: [0, 5000, 0], radius: 1000, start_angle: 180, end_angle: 0}
        - edges:
            - arc: {center: [0, 5000, 0], radius: 999.99, start_angle: 10, end_angle: 190}
            - arc: {center: [0, 5000, 0], radius: 999.99, start_angle: 190, end_angle: 10}
  - line: {id: first, from: [3000, 0, 0], to: [3010, 0, 0]}
  - line: {id: next, from: [3010, 0, 0], to: [3020, 0, 0]}
  - line: {id: upright, from: [4000, 0, -1], to: [4000, 0, 1]}
  - line: {id: across, from: [3999, 0, 0], to: [4001, 0, 0]}
  - ellipse: {id: oval, center: [0, 0, 0], major_axis: [2, 0, 0], ratio: 0.5}
  - line: {id: touching, from: [2, -1, 0], to: [2, 1, 0]}
  - circle: {id: kissing, center: [3, 0, 0], radius: 1}
  - line: {id: through, from: [6000, 0, 0], to: [6010, 0, 0]}
  - polyline: {id: hairpin, points: [[6009, -1, 0], [6009, 1, 0], [6001, 1, 0], [6001, -1, 0]]}
"#,
    )
    .unwrap();
    sheet
}

fn with(request: SnapRef, id: &str) -> SnapRef {
    SnapRef {
        with: Some(ObjectRef {
            id: Some(id.to_owned()),
            name: None,
        }),
        ..request
    }
}

#[test]
fn every_foot_and_the_nearest_point_on_a_spline_are_found() {
    let sheet = review("parabola");
    let from = || Some(Box::new(kind("review/above", SnapKind::Center)));
    // y = x² / 1000 from [0, 500.9]: the feet at x = 0 and x = ±30.
    let (_, points) = snap(
        &sheet,
        SnapRef {
            from: from(),
            ..kind("review/parabola", SnapKind::Perpendicular)
        },
    );
    let mut xs: Vec<f64> = points.iter().map(|p| p.at.x).collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs.len(), 3, "{points:#?}");
    for (x, want) in xs.iter().zip([-30.0, 0.0, 30.0]) {
        assert!((x - want).abs() < 1e-6, "{xs:?}");
    }
    let (_, points) = snap(
        &sheet,
        SnapRef {
            from: from(),
            ..kind("review/parabola", SnapKind::Nearest)
        },
    );
    assert_eq!(points.len(), 1);
    assert!((points[0].at.x.abs() - 30.0).abs() < 1e-6, "{points:#?}");
    assert!((points[0].at.y - 0.9).abs() < 1e-6, "{points:#?}");
}

#[test]
fn a_hole_that_all_but_fills_its_hatch_is_a_hole() {
    let ring = geometry(&review("ring"), "review/ring");
    near(
        ring.measures.area,
        PI * (1000.0f64.powi(2) - 999.99f64.powi(2)),
    );
}

#[test]
fn a_line_seen_end_on_meets_one_through_it_and_collinear_lines_do_not_intersect() {
    let sheet = review("parallel");
    // AutoCAD 2026's intersection snap found none between collinear lines,
    // whether they overlap or only meet end to end: their endpoints are
    // snapped to as endpoints.
    for kind_of in [SnapKind::Intersection, SnapKind::ApparentIntersection] {
        let (_, points) = snap(&sheet, with(kind("review/first", kind_of), "review/next"));
        assert!(points.is_empty(), "{points:#?}");
    }
    let (_, points) = snap(
        &sheet,
        with(
            kind("review/upright", SnapKind::Intersection),
            "review/across",
        ),
    );
    assert_eq!(points.len(), 1, "{points:#?}");
    close(points[0].at, [4000.0, 0.0, 0.0]);
}

#[test]
fn a_line_or_a_circle_tangent_to_an_ellipse_meets_it_once() {
    let sheet = review("tangent");
    let (_, points) = snap(
        &sheet,
        with(
            kind("review/touching", SnapKind::Intersection),
            "review/oval",
        ),
    );
    assert_eq!(points.len(), 1, "{points:#?}");
    close(points[0].at, [2.0, 0.0, 0.0]);
    let (_, points) = snap(
        &sheet,
        with(
            kind("review/kissing", SnapKind::Intersection),
            "review/oval",
        ),
    );
    assert_eq!(points.len(), 1, "{points:#?}");
    // Where two curves touch, the point is found to the square root of
    // the arithmetic's precision.
    assert!((points[0].at.x - 2.0).abs() < 1e-6 && points[0].at.y.abs() < 1e-6);
}

#[test]
fn intersections_are_numbered_along_the_first_object() {
    let sheet = review("order");
    for kind_of in [SnapKind::Intersection, SnapKind::ApparentIntersection] {
        let (_, points) = snap(
            &sheet,
            with(kind("review/through", kind_of), "review/hairpin"),
        );
        let first = named(&points, kind_of, Some("0"));
        close(first.at, [6001.0, 0.0, 0.0]);
        close(named(&points, kind_of, Some("1")).at, [6009.0, 0.0, 0.0]);
    }
}

#[test]
fn a_which_on_a_snap_of_one_point_says_it_takes_none() {
    let sheet = review("which");
    let e = try_ask(
        &sheet,
        Query::Snap(SnapRef {
            which: Some("x".to_owned()),
            from: Some(Box::new(kind("review/above", SnapKind::Center))),
            ..kind("review/parabola", SnapKind::Nearest)
        }),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("query.which"));
    assert!(
        e.message
            .contains("one nearest point, which takes no `which`"),
        "{}",
        e.message
    );
}
