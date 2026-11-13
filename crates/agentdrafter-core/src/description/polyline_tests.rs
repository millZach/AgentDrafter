//! Polylines with arc segments and widths, donuts and 2D solids in a
//! sheet description.

use super::parse_sheet_description;
use crate::error::{Error, ErrorCode};
use crate::model::{Donut, Object, Point3, Polyline, Solid};

fn one(object: &str) -> Object {
    let sheet = parse_sheet_description(&format!("units: mm\nobjects:\n  - {object}\n"))
        .unwrap_or_else(|e| panic!("{e}"));
    sheet.objects.into_iter().next().unwrap()
}

fn polyline(object: &str) -> Polyline {
    match one(object) {
        Object::Polyline(p) => p,
        other => panic!("{other:?}"),
    }
}

fn error(object: &str) -> Error {
    match parse_sheet_description(&format!("units: mm\nobjects:\n  - {object}\n")) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(e) => e,
    }
}

#[track_caller]
fn assert_names(e: &Error, code: ErrorCode, field: &str, words: &[&str]) {
    assert_eq!(e.code, code, "code of {e:?}");
    assert_eq!(e.field.as_deref(), Some(field), "field of {e:?}");
    for word in words {
        assert!(e.message.contains(word), "`{word}` in {:?}", e.message);
    }
}

#[test]
fn a_plain_polyline_has_no_arcs_and_no_widths() {
    let p = polyline("polyline: {points: [[0, 0, 0], [10, 0, 0]]}");
    assert!(p.bulges.is_empty() && p.widths.is_empty());
    assert_eq!(p.width, 0.0);
}

#[test]
fn a_point_may_give_its_segments_bulge_arc_angle_and_widths() {
    let p = polyline(
        "polyline: {points: [{point: [0, 0, 0], bulge: 0.5, start_width: 1, end_width: 0}, \
         {point: [10, 0, 0], arc_angle: 180}, [10, 10, 0], [0, 10, 0]], closed: true}",
    );
    assert_eq!(p.points[1], Point3::new(10.0, 0.0, 0.0));
    assert_eq!(p.bulges[0], 0.5);
    assert!((p.bulges[1] - 1.0).abs() < 1e-12, "{:?}", p.bulges);
    assert_eq!(p.bulges[2], 0.0);
    assert_eq!(
        p.widths,
        vec![[1.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]
    );
    let clockwise =
        polyline("polyline: {points: [{point: [0, 0, 0], arc_angle: -90}, [10, 0, 0]]}");
    assert!((clockwise.bulges[0] + (std::f64::consts::PI / 8.0).tan()).abs() < 1e-12);
}

#[test]
fn a_polyline_takes_a_constant_width_in_any_unit() {
    let p = polyline("polyline: {points: [[0, 0, 0], [10, 0, 0]], width: 0.1 in}");
    assert!((p.width - 2.54).abs() < 1e-12);
    assert_eq!(p.widths_at(0), [p.width, p.width]);
}

#[test]
fn a_wrong_arc_or_width_is_refused() {
    for (object, code, field, words) in [
        (
            "polyline: {points: [{point: [0, 0, 0], bulge: 1, arc_angle: 180}, [1, 0, 0]]}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].arc_angle",
            &["not both"][..],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], arc_angle: 360}, [1, 0, 0]]}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].arc_angle",
            &["less than 360", "circle"],
        ),
        (
            "polyline: {points: [[0, 0, 0], {point: [1, 0, 0], bulge: 1}]}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[1].bulge",
            &["last point of an open polyline"],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], start_width: -1}, [1, 0, 0]]}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].start_width",
            &["0 or more"],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], start_width: 1}, [1, 0, 0]], width: 2}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].start_width",
            &["not both"],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], bulge: 1}, [1, 0, 5]]}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].bulge",
            &["3D polyline"],
        ),
        (
            "polyline: {points: [[0, 0, 0], [1, 0, 5]], width: 1}",
            ErrorCode::BadValue,
            "objects[0].polyline.width",
            &["3D polyline"],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], sweep: 1}, [1, 0, 0]]}",
            ErrorCode::UnknownKey,
            "objects[0].polyline.points[0].sweep",
            &["point, bulge, arc_angle, start_width, end_width"],
        ),
        (
            "polyline: {points: [{bulge: 1}, [1, 0, 0]]}",
            ErrorCode::MissingKey,
            "objects[0].polyline.points[0].point",
            &["point"],
        ),
        (
            "polyline: {points: [{point: [0, 0, 0], bulge: 1, start_width: 1}, [1, 0, 0]], width: 2}",
            ErrorCode::BadValue,
            "objects[0].polyline.points[0].start_width",
            &["not both"],
        ),
    ] {
        assert_names(&error(object), code, field, words);
    }
    // A closed polyline's last point starts the segment back to the first.
    let closed = polyline(
        "polyline: {points: [[0, 0, 0], [1, 0, 0], {point: [1, 1, 0], bulge: 0.3}], closed: true}",
    );
    assert_eq!(closed.bulges[2], 0.3);
}

#[test]
fn a_donut_is_a_ring_or_a_dot() {
    let Object::Donut(ring) =
        one("donut: {center: [5, 5, 0], inside_diameter: 2, outside_diameter: 4}")
    else {
        panic!()
    };
    assert_eq!(
        ring,
        Donut {
            center: Point3::new(5.0, 5.0, 0.0),
            inside_diameter: 2.0,
            outside_diameter: 4.0,
            layer: "0".to_owned(),
            properties: Default::default(),
        }
    );
    // AutoCAD's DONUT: two half circles on the ring's middle, as wide as the ring.
    let p = ring.polyline();
    assert_eq!(
        p.points,
        vec![Point3::new(6.5, 5.0, 0.0), Point3::new(3.5, 5.0, 0.0)]
    );
    assert_eq!(
        (p.bulges.clone(), p.width, p.closed),
        (vec![1.0, 1.0], 1.0, true)
    );
    let Object::Donut(dot) = one("donut: {center: [0, 0, 0], outside_diameter: 1}") else {
        panic!()
    };
    assert_eq!(dot.inside_diameter, 0.0);
    for (object, field, words) in [
        (
            "donut: {center: [0, 0, 0], outside_diameter: 0}",
            "objects[0].donut.outside_diameter",
            &["more than 0"][..],
        ),
        (
            "donut: {center: [0, 0, 0], inside_diameter: 3, outside_diameter: 2}",
            "objects[0].donut.inside_diameter",
            &["less than its outside diameter"],
        ),
    ] {
        assert_names(&error(object), ErrorCode::BadValue, field, words);
    }
}

#[test]
fn a_solid_has_three_or_four_flat_corners() {
    let Object::Solid(solid) =
        one("solid: {corners: [[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]], color: 3}")
    else {
        panic!()
    };
    assert_eq!(solid.corners.len(), 4);
    let Object::Solid(Solid { corners, .. }) =
        one("solid: {corners: [[0, 0, 0], [4, 0, 0], [2, 3, 0]]}")
    else {
        panic!()
    };
    assert_eq!(corners.len(), 3);
    assert_names(
        &error("solid: {corners: [[0, 0, 0], [4, 0, 0]]}"),
        ErrorCode::WrongType,
        "objects[0].solid.corners",
        &["3 or 4 corners"],
    );
    assert_names(
        &error("solid: {corners: [[0, 0, 0], [4, 0, 0], [2, 3, 1]]}"),
        ErrorCode::BadValue,
        "objects[0].solid.corners",
        &["share one `z`"],
    );
}
