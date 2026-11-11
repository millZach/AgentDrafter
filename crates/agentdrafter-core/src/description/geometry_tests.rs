//! Objects, layers and linetypes in a sheet description.

use super::parse_sheet_description;
use crate::error::{Error, ErrorCode};
use crate::model::{
    Arc, Circle, Ellipse, Layer, Line, Linetype, Lineweight, Object, Point3, Polyline, Sheet,
    Spline,
};

fn sheet(text: &str) -> Sheet {
    parse_sheet_description(text).unwrap_or_else(|e| panic!("{e}"))
}

/// The objects of a sheet in mm whose `objects:` list is `objects`.
fn objects(objects: &str) -> Vec<Object> {
    sheet(&format!("units: mm\nobjects:\n{objects}")).objects
}

fn one(object: &str) -> Object {
    let mut all = objects(&format!("  - {object}\n"));
    assert_eq!(all.len(), 1);
    all.remove(0)
}

fn error(text: &str) -> Error {
    match parse_sheet_description(text) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(error) => error,
    }
}

/// The error for a sheet in mm holding one object, `object`.
fn object_error(object: &str) -> Error {
    error(&format!("units: mm\nobjects:\n  - {object}\n"))
}

#[track_caller]
fn assert_names(e: &Error, code: ErrorCode, field: &str, words: &[&str]) {
    assert_eq!(e.code, code, "code of {e:?}");
    assert_eq!(e.field.as_deref(), Some(field), "field of {e:?}");
    assert!(e.line.is_some() && e.column.is_some(), "position of {e:?}");
    for word in words {
        assert!(e.message.contains(word), "`{word}` in {:?}", e.message);
    }
}

const P0: Point3 = Point3::new(0.0, 0.0, 0.0);

fn on_zero() -> String {
    "0".to_owned()
}

#[test]
fn every_object_type_reads() {
    assert_eq!(
        one("polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 5, 0]], closed: true}"),
        Object::Polyline(Polyline {
            points: vec![P0, Point3::new(10.0, 0.0, 0.0), Point3::new(10.0, 5.0, 0.0)],
            closed: true,
            layer: on_zero(),
        })
    );
    assert_eq!(
        one("arc: {centre: [1, 2, 3], radius: 4, start_angle: 30, end_angle: 120}"),
        Object::Arc(Arc {
            centre: Point3::new(1.0, 2.0, 3.0),
            radius: 4.0,
            start_angle: 30.0,
            end_angle: 120.0,
            layer: on_zero(),
        })
    );
    assert_eq!(
        one("circle: {centre: [1, 2, 3], radius: 4}"),
        Object::Circle(Circle {
            centre: Point3::new(1.0, 2.0, 3.0),
            radius: 4.0,
            layer: on_zero(),
        })
    );
    assert_eq!(
        one("ellipse: {centre: [0, 0, 0], major_axis: [10, 0, 0], ratio: 0.5}"),
        Object::Ellipse(Ellipse {
            centre: P0,
            major_axis: Point3::new(10.0, 0.0, 0.0),
            ratio: 0.5,
            start_angle: 0.0,
            end_angle: 360.0,
            layer: on_zero(),
        })
    );
    assert_eq!(
        one("spline: {fit_points: [[0, 0, 0], [5, 5, 0], [10, 0, 2]]}"),
        Object::Spline(Spline {
            fit_points: vec![P0, Point3::new(5.0, 5.0, 0.0), Point3::new(10.0, 0.0, 2.0)],
            control_points: Vec::new(),
            degree: 3,
            layer: on_zero(),
        })
    );
    assert_eq!(
        one("spline: {control_points: [[0, 0, 0], [5, 5, 0], [10, 0, 0]], degree: 2}"),
        Object::Spline(Spline {
            fit_points: Vec::new(),
            control_points: vec![P0, Point3::new(5.0, 5.0, 0.0), Point3::new(10.0, 0.0, 0.0)],
            degree: 2,
            layer: on_zero(),
        })
    );
}

#[test]
fn defaults_are_an_open_polyline_a_cubic_spline_and_layer_zero() {
    let Object::Polyline(polyline) = one("polyline: {points: [[0, 0, 0], [1, 0, 0]]}") else {
        panic!()
    };
    assert!(!polyline.closed);
    assert_eq!(polyline.layer, "0");
    let Object::Spline(spline) =
        one("spline: {control_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0]]}")
    else {
        panic!()
    };
    assert_eq!(spline.degree, 3);
}

#[test]
fn angles_lose_whole_turns() {
    let Object::Arc(arc) =
        one("arc: {centre: [0, 0, 0], radius: 1, start_angle: -90, end_angle: 450}")
    else {
        panic!()
    };
    assert_eq!((arc.start_angle, arc.end_angle), (270.0, 90.0));
    let Object::Ellipse(ellipse) = one(
        "ellipse: {centre: [0, 0, 0], major_axis: [0, 3, 0], ratio: 1, start_angle: 360, end_angle: 180}",
    ) else {
        panic!()
    };
    assert_eq!((ellipse.start_angle, ellipse.end_angle), (0.0, 180.0));
    assert!(!ellipse.is_whole());
}

#[test]
fn lengths_with_their_own_unit_are_converted_everywhere() {
    let sheet = sheet(
        "units: mm
linetypes:
  DASHED: {pattern: [0.5 in, -2.5mm]}
objects:
  - circle: {centre: [1 in, 0, 0], radius: 1 cm}
  - ellipse: {centre: [0, 0, 0], major_axis: [1 m, 0, 0], ratio: 0.25}
",
    );
    assert_eq!(sheet.linetypes[0].pattern, vec![12.7, -2.5]);
    assert_eq!(
        sheet.objects,
        vec![
            Object::Circle(Circle {
                centre: Point3::new(25.4, 0.0, 0.0),
                radius: 10.0,
                layer: on_zero(),
            }),
            Object::Ellipse(Ellipse {
                centre: P0,
                major_axis: Point3::new(1000.0, 0.0, 0.0),
                ratio: 0.25,
                start_angle: 0.0,
                end_angle: 360.0,
                layer: on_zero(),
            }),
        ]
    );
}

const STYLED: &str = "\
units: mm
linetypes:
  DASHED:
    description: Dashed __ __ __
    pattern: [12.7, -6.35]
  dot: {pattern: [0, -2]}
layers:
  outline: {colour: 7, lineweight: 0.5}
  hidden: {colour: red, linetype: DASHED, lineweight: 0.25 mm}
  \"0\": {colour: 3}
  notes: {}
objects:
  - line: {from: [0, 0, 0], to: [10, 0, 0], layer: hidden}
  - line: {from: [0, 0, 0], to: [10, 0, 0], layer: 0}
";

#[test]
fn layers_carry_colour_linetype_and_lineweight_from_a_linetype_definition() {
    let sheet = sheet(STYLED);
    assert_eq!(
        sheet.linetypes,
        vec![
            Linetype {
                name: "DASHED".into(),
                description: "Dashed __ __ __".into(),
                pattern: vec![12.7, -6.35],
            },
            Linetype {
                name: "dot".into(),
                description: String::new(),
                pattern: vec![0.0, -2.0],
            },
        ]
    );
    let weight = |mm| Some(Lineweight::from_millimetres(mm).unwrap());
    assert_eq!(
        sheet.layers,
        vec![
            Layer {
                lineweight: weight(0.5),
                ..Layer::new("outline")
            },
            Layer {
                colour: 1,
                linetype: "DASHED".into(),
                lineweight: weight(0.25),
                ..Layer::new("hidden")
            },
            Layer {
                colour: 3,
                ..Layer::new("0")
            },
            Layer::new("notes"),
        ]
    );
    assert_eq!(Layer::new("notes").colour, 7);
    assert_eq!(Layer::new("notes").linetype, "Continuous");
    assert_eq!(
        sheet.objects[0],
        Object::Line(Line {
            from: P0,
            to: Point3::new(10.0, 0.0, 0.0),
            layer: "hidden".into(),
        })
    );
    assert_eq!(sheet.objects[1].layer(), "0");
}

#[test]
fn every_colour_name_and_index_reads() {
    for (name, index) in [
        ("red", 1),
        ("yellow", 2),
        ("green", 3),
        ("cyan", 4),
        ("blue", 5),
        ("magenta", 6),
        ("white", 7),
        ("1", 1),
        ("255", 255),
        ("'42'", 42),
    ] {
        let sheet = sheet(&format!(
            "units: mm\nlayers: {{a: {{colour: {name}}}}}\nobjects: []\n"
        ));
        assert_eq!(sheet.layers[0].colour, index, "{name}");
    }
}

#[test]
fn every_standard_lineweight_reads_in_millimetres_whatever_the_unit() {
    for weight in Lineweight::ALL {
        for written in [
            format!("{weight}"),
            format!("{weight} mm"),
            format!("{weight}mm"),
        ] {
            let sheet = sheet(&format!(
                "units: in\nlayers: {{a: {{lineweight: {written}}}}}\nobjects: []\n"
            ));
            assert_eq!(sheet.layers[0].lineweight, Some(weight), "{written}");
        }
    }
}

#[test]
fn an_unknown_layer_lists_the_sheets_layers() {
    let e = error(
        "units: mm\nlayers: {outline: {}, hidden: {}}\nobjects:\n  - circle: {centre: [0, 0, 0], radius: 1, layer: Outline}\n",
    );
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].circle.layer",
        &["unknown layer `Outline`", "0, outline, hidden"],
    );
    let e = object_error("line: {from: [0, 0, 0], to: [1, 0, 0], layer: walls}");
    assert_eq!(e.message, "unknown layer `walls`; the sheet's layers: 0");
}

#[test]
fn an_unknown_linetype_lists_the_sheets_linetypes() {
    let e = error(
        "units: mm\nlinetypes: {DASHED: {pattern: [1, -1]}}\nlayers:\n  hidden: {linetype: HIDDEN}\nobjects: []\n",
    );
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "layers.hidden.linetype",
        &["unknown linetype `HIDDEN`", "Continuous, DASHED"],
    );
    assert_eq!(e.line, Some(4));
}

#[test]
fn a_colour_outside_the_index_is_refused_with_the_choices() {
    for bad in [
        "0",
        "256",
        "purple",
        "1.5",
        "-3",
        "+5",
        "Red",
        "",
        "[1, 2, 3]",
    ] {
        let e = error(&format!(
            "units: mm\nlayers:\n  a: {{colour: {bad}}}\nobjects: []\n"
        ));
        let code = if bad.starts_with('[') {
            ErrorCode::WrongType
        } else {
            ErrorCode::BadValue
        };
        assert_names(&e, code, "layers.a.colour", &[]);
        if code == ErrorCode::BadValue {
            assert!(e.message.contains("1 to 255"), "{bad}: {}", e.message);
            assert!(
                e.message
                    .contains("red, yellow, green, cyan, blue, magenta, white"),
                "{bad}: {}",
                e.message
            );
        }
    }
}

#[test]
fn a_lineweight_off_the_standard_list_is_refused_with_the_choices() {
    let lineweight = |value: &str| {
        error(&format!(
            "units: mm\nlayers:\n  a: {{lineweight: {value}}}\nobjects: []\n"
        ))
    };
    let all = "0, 0.05, 0.09, 0.13, 0.15, 0.18, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5, 0.53, 0.6, 0.7, 0.8, 0.9, 1, 1.06, 1.2, 1.4, 1.58, 2, 2.11";
    assert_names(
        &lineweight("0.33"),
        ErrorCode::BadValue,
        "layers.a.lineweight",
        &[all],
    );
    assert_names(
        &lineweight("0.35 in"),
        ErrorCode::BadValue,
        "layers.a.lineweight",
        &["millimetres, not in", all],
    );
    assert_names(
        &lineweight("0.35 pt"),
        ErrorCode::UnknownUnit,
        "layers.a.lineweight",
        &[all],
    );
    assert_names(
        &lineweight("thick"),
        ErrorCode::BadNumber,
        "layers.a.lineweight",
        &[all],
    );
}

#[test]
fn layer_and_linetype_names_follow_autocads_rules() {
    let e = error("units: mm\nlayers:\n  Walls: {}\n  WALLS: {}\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::DuplicateKey,
        "layers.WALLS",
        &["`WALLS`", "`Walls`", "line 3", "ignores case"],
    );
    assert_eq!(e.line, Some(4));
    for bad in [
        "\"a/b\"",
        "\"a:b\"",
        "\"a,b\"",
        "\" a\"",
        "\"a \"",
        "\"\"",
        "\"a\\tb\"",
    ] {
        let e = error(&format!("units: mm\nlayers:\n  {bad}: {{}}\nobjects: []\n"));
        assert_eq!(e.code, ErrorCode::BadValue, "{bad}: {e:?}");
        assert!(e.field.as_deref().unwrap().starts_with("layers."), "{e:?}");
    }
    let long = "x".repeat(256);
    let e = error(&format!(
        "units: mm\nlayers:\n  {long}: {{}}\nobjects: []\n"
    ));
    assert!(e.message.contains("at most 255"), "{e:?}");
    for reserved in ["Continuous", "CONTINUOUS", "ByLayer", "byblock"] {
        let e = error(&format!(
            "units: mm\nlinetypes:\n  {reserved}: {{pattern: [1, -1]}}\nobjects: []\n"
        ));
        assert_names(
            &e,
            ErrorCode::BadValue,
            &format!("linetypes.{reserved}"),
            &["AutoCAD's own linetype"],
        );
    }
}

#[test]
fn a_linetype_pattern_follows_autocads_rules() {
    let pattern = |value: &str| {
        error(&format!(
            "units: mm\nlinetypes:\n  A: {{pattern: {value}}}\nobjects: []\n"
        ))
    };
    for bad in [
        "[1]",
        "[]",
        "[-1, 1]",
        "[1, 1]",
        "[1, 0]",
        "[1, -1, 1, -1, 1, -1, 1, -1, 1, -1, 1, -1, 1]",
    ] {
        assert_names(
            &pattern(bad),
            ErrorCode::BadValue,
            "linetypes.A.pattern",
            &[],
        );
    }
    assert_names(
        &pattern("[1, -1 furlong]"),
        ErrorCode::UnknownUnit,
        "linetypes.A.pattern[1]",
        &["mm, cm, m, in, ft"],
    );
    let e = error(
        "units: mm\nlinetypes:\n  A: {pattern: [1, -1], description: This description is longer than forty-seven chars}\nobjects: []\n",
    );
    assert_names(&e, ErrorCode::BadValue, "linetypes.A.description", &["47"]);
    let e = error("units: mm\nlinetypes:\n  A: {description: dashes}\nobjects: []\n");
    assert_names(&e, ErrorCode::MissingKey, "linetypes.A.pattern", &[]);
}

#[test]
fn unknown_keys_in_layers_linetypes_and_objects_list_the_allowed_keys() {
    let e = error("units: mm\nlayers:\n  a: {color: red}\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "layers.a.color",
        &["a layer takes colour, linetype, lineweight"],
    );
    let e = error("units: mm\nlinetypes:\n  a: {dashes: [1, -1]}\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "linetypes.a.dashes",
        &["a linetype takes description, pattern"],
    );
    for (object, key, allowed) in [
        (
            "polyline: {points: [], close: true}",
            "close",
            "a polyline takes points, closed, layer",
        ),
        (
            "arc: {center: [0, 0, 0]}",
            "center",
            "an arc takes centre, radius, start_angle, end_angle, layer",
        ),
        (
            "circle: {diameter: 2}",
            "diameter",
            "a circle takes centre, radius, layer",
        ),
        (
            "ellipse: {minor_axis: 2}",
            "minor_axis",
            "an ellipse takes centre, major_axis, ratio, start_angle, end_angle, layer",
        ),
        (
            "spline: {points: []}",
            "points",
            "a spline takes fit_points, control_points, degree, layer",
        ),
    ] {
        let e = object_error(object);
        let type_name = object.split(':').next().unwrap();
        assert_names(
            &e,
            ErrorCode::UnknownKey,
            &format!("objects[0].{type_name}.{key}"),
            &[allowed],
        );
    }
}

#[test]
fn a_polyline_needs_two_points_or_three_when_closed() {
    assert_names(
        &object_error("polyline: {points: [[0, 0, 0]]}"),
        ErrorCode::WrongType,
        "objects[0].polyline.points",
        &["at least 2 points"],
    );
    assert_names(
        &object_error("polyline: {points: [[0, 0, 0], [1, 0, 0]], closed: true}"),
        ErrorCode::WrongType,
        "objects[0].polyline.points",
        &["at least 3 points"],
    );
    assert_names(
        &object_error("polyline: {points: [[0, 0, 0], [1, 0, 0]], closed: yes}"),
        ErrorCode::WrongType,
        "objects[0].polyline.closed",
        &["`true` or `false`"],
    );
    assert_names(
        &object_error("polyline: {points: [[0, 0, 0], [1, 0]]}"),
        ErrorCode::WrongType,
        "objects[0].polyline.points[1]",
        &["three coordinates"],
    );
}

#[test]
fn a_radius_is_more_than_zero() {
    for (object, field) in [
        (
            "circle: {centre: [0, 0, 0], radius: 0}",
            "objects[0].circle.radius",
        ),
        (
            "arc: {centre: [0, 0, 0], radius: -1, start_angle: 0, end_angle: 90}",
            "objects[0].arc.radius",
        ),
    ] {
        assert_names(
            &object_error(object),
            ErrorCode::BadValue,
            field,
            &["more than 0"],
        );
    }
    assert_names(
        &object_error("circle: {centre: [0, 0, 0]}"),
        ErrorCode::MissingKey,
        "objects[0].circle.radius",
        &[],
    );
}

#[test]
fn an_arc_and_an_angle_are_checked() {
    assert_names(
        &object_error("arc: {centre: [0, 0, 0], radius: 1, start_angle: 10, end_angle: 370}"),
        ErrorCode::BadValue,
        "objects[0].arc.end_angle",
        &["`circle`"],
    );
    assert_names(
        &object_error("arc: {centre: [0, 0, 0], radius: 1, start_angle: 90 deg, end_angle: 0}"),
        ErrorCode::BadNumber,
        "objects[0].arc.start_angle",
        &["degrees", "no unit"],
    );
}

#[test]
fn an_ellipse_is_checked() {
    let ellipse = |rest: &str| object_error(&format!("ellipse: {{centre: [0, 0, 0], {rest}}}"));
    assert_names(
        &ellipse("major_axis: [1, 0, 1], ratio: 0.5"),
        ErrorCode::BadValue,
        "objects[0].ellipse.major_axis",
        &["dz"],
    );
    assert_names(
        &ellipse("major_axis: [0, 0, 0], ratio: 0.5"),
        ErrorCode::BadValue,
        "objects[0].ellipse.major_axis",
        &[],
    );
    for ratio in ["0", "1.5", "-0.5"] {
        assert_names(
            &ellipse(&format!("major_axis: [1, 0, 0], ratio: {ratio}")),
            ErrorCode::BadValue,
            "objects[0].ellipse.ratio",
            &["more than 0 and at most 1"],
        );
    }
    assert_names(
        &ellipse("major_axis: [1, 0, 0], ratio: 1 mm"),
        ErrorCode::BadNumber,
        "objects[0].ellipse.ratio",
        &["no unit"],
    );
    assert_names(
        &ellipse("major_axis: [1, 0, 0], ratio: 0.5, start_angle: 10"),
        ErrorCode::MissingKey,
        "objects[0].ellipse.end_angle",
        &["both"],
    );
    assert_names(
        &ellipse("major_axis: [1, 0, 0], ratio: 0.5, end_angle: 10"),
        ErrorCode::MissingKey,
        "objects[0].ellipse.start_angle",
        &[],
    );
    assert_names(
        &ellipse("major_axis: [1, 0, 0], ratio: 0.5, start_angle: 0, end_angle: 720"),
        ErrorCode::BadValue,
        "objects[0].ellipse.end_angle",
        &[],
    );
}

#[test]
fn a_spline_is_checked() {
    assert_names(
        &object_error("spline: {fit_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]], control_points: [[0, 0, 0], [1, 1, 0]]}"),
        ErrorCode::BadValue,
        "objects[0].spline.control_points",
        &["not both"],
    );
    assert_names(
        &object_error("spline: {degree: 3}"),
        ErrorCode::MissingKey,
        "objects[0].spline.fit_points",
        &["control_points"],
    );
    assert_names(
        &object_error("spline: {fit_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]], degree: 2}"),
        ErrorCode::BadValue,
        "objects[0].spline.degree",
        &["cubic"],
    );
    assert_names(
        &object_error("spline: {fit_points: [[0, 0, 0], [1, 1, 0]]}"),
        ErrorCode::WrongType,
        "objects[0].spline.fit_points",
        &["at least 3 points"],
    );
    assert_names(
        &object_error("spline: {fit_points: [[0, 0, 0], [1, 1, 0], [1, 1, 0], [2, 0, 0]]}"),
        ErrorCode::BadValue,
        "objects[0].spline.fit_points[2]",
        &["fit point 2 repeats fit point 1"],
    );
    assert_names(
        &object_error("spline: {control_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]]}"),
        ErrorCode::BadValue,
        "objects[0].spline.control_points",
        &["degree 3 needs at least 4 control points"],
    );
    for degree in ["0", "11", "300"] {
        assert_names(
            &object_error(&format!(
                "spline: {{control_points: [[0, 0, 0], [1, 1, 0]], degree: {degree}}}"
            )),
            ErrorCode::BadValue,
            "objects[0].spline.degree",
            &["1 to 10"],
        );
    }
    for degree in ["2.5", "two", "-1", "+2"] {
        assert_names(
            &object_error(&format!(
                "spline: {{control_points: [[0, 0, 0], [1, 1, 0]], degree: {degree}}}"
            )),
            ErrorCode::BadNumber,
            "objects[0].spline.degree",
            &["whole number"],
        );
    }
}
