//! Reads the objects on a sheet: each is a mapping with one key, its type.

use super::layers::layer_name;
use super::strict_yaml::{index_path, key_path, Kind, Node};
use super::{
    angle, at, bad_value, boolean, length, plain_number, point, points, scalar, unknown_key,
    wrong_type, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{Arc, Circle, Ellipse, Layer, Line, Object, Polyline, Spline, LAYER_ZERO};
use crate::units::Unit;

/// Every object type a sheet description can hold, and the keys each takes.
pub(crate) const OBJECT_TYPES: [(&str, &[&str]); 6] = [
    ("line", &LINE_KEYS),
    ("polyline", &POLYLINE_KEYS),
    ("arc", &ARC_KEYS),
    ("circle", &CIRCLE_KEYS),
    ("ellipse", &ELLIPSE_KEYS),
    ("spline", &SPLINE_KEYS),
];

const LINE_KEYS: [&str; 3] = ["from", "to", "layer"];
const POLYLINE_KEYS: [&str; 3] = ["points", "closed", "layer"];
const ARC_KEYS: [&str; 5] = ["centre", "radius", "start_angle", "end_angle", "layer"];
const CIRCLE_KEYS: [&str; 3] = ["centre", "radius", "layer"];
const ELLIPSE_KEYS: [&str; 6] = [
    "centre",
    "major_axis",
    "ratio",
    "start_angle",
    "end_angle",
    "layer",
];
const SPLINE_KEYS: [&str; 4] = ["fit_points", "control_points", "degree", "layer"];

/// The highest degree a spline by control points can have, as in AutoCAD.
pub(crate) const HIGHEST_DEGREE: u8 = 10;

/// The degree of a spline through fit points: AutoCAD always fits a cubic.
pub(crate) const FIT_DEGREE: u8 = 3;

/// The fewest fit points a spline takes. Through two points it would be a line.
pub(crate) const FEWEST_FIT_POINTS: usize = 3;

pub(super) fn object(
    node: &Node,
    path: &str,
    units: Unit,
    layers: &[Layer],
) -> Result<Object, Error> {
    const SHAPE: &str = "a mapping with one key naming the object's type, such as `line:`";
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let known = || {
        OBJECT_TYPES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(entry) = entries
        .iter()
        .find(|e| !OBJECT_TYPES.iter().any(|(name, _)| *name == e.key))
    {
        return Err(unknown_key(
            entry,
            path,
            &format!("unknown object type `{}`", entry.key),
            &format!("known types: {}", known()),
        ));
    }
    let [entry] = entries.as_slice() else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let path = key_path(path, &entry.key);
    let keys = OBJECT_TYPES
        .iter()
        .find(|(name, _)| *name == entry.key)
        .map(|(_, keys)| *keys)
        .unwrap_or_default();
    let article = if entry.key.starts_with(['a', 'e']) {
        "an"
    } else {
        "a"
    };
    let fields = Fields::of(
        &entry.value,
        &path,
        keys,
        &format!("{article} {}", entry.key),
    )?;
    let layer = match fields.get("layer") {
        Some(node) => layer_name(node, &fields.path_of("layer"), layers)?,
        None => LAYER_ZERO.to_owned(),
    };
    match entry.key.as_str() {
        "line" => line(&fields, units, layer).map(Object::Line),
        "polyline" => polyline(&fields, units, layer).map(Object::Polyline),
        "arc" => arc(&fields, units, layer).map(Object::Arc),
        "circle" => circle(&fields, units, layer).map(Object::Circle),
        "ellipse" => ellipse(&fields, units, layer).map(Object::Ellipse),
        "spline" => spline(&fields, units, layer).map(Object::Spline),
        other => unreachable!("object type `{other}` is listed but not read"),
    }
}

fn line(fields: &Fields, units: Unit, layer: String) -> Result<Line, Error> {
    Ok(Line {
        from: point(fields.require("from")?, &fields.path_of("from"), units)?,
        to: point(fields.require("to")?, &fields.path_of("to"), units)?,
        layer,
    })
}

fn polyline(fields: &Fields, units: Unit, layer: String) -> Result<Polyline, Error> {
    let closed = match fields.get("closed") {
        Some(node) => boolean(node, &fields.path_of("closed"))?,
        None => false,
    };
    let path = fields.path_of("points");
    // A closed polyline needs three points to enclose anything.
    let fewest = if closed { 3 } else { 2 };
    let points = points(fields.require("points")?, &path, units, fewest)?;
    Ok(Polyline {
        points,
        closed,
        layer,
    })
}

fn arc(fields: &Fields, units: Unit, layer: String) -> Result<Arc, Error> {
    let centre = point(fields.require("centre")?, &fields.path_of("centre"), units)?;
    let radius = radius(fields, units)?;
    let start_angle = angle(
        fields.require("start_angle")?,
        &fields.path_of("start_angle"),
    )?;
    let end_node = fields.require("end_angle")?;
    let end_angle = angle(end_node, &fields.path_of("end_angle"))?;
    if start_angle == end_angle {
        return Err(bad_value(
            end_node,
            &fields.path_of("end_angle"),
            "an arc's end angle must differ from its start angle, counting whole turns as none; for a whole circle use `circle`",
        ));
    }
    Ok(Arc {
        centre,
        radius,
        start_angle,
        end_angle,
        layer,
    })
}

fn circle(fields: &Fields, units: Unit, layer: String) -> Result<Circle, Error> {
    Ok(Circle {
        centre: point(fields.require("centre")?, &fields.path_of("centre"), units)?,
        radius: radius(fields, units)?,
        layer,
    })
}

/// A radius, a length more than 0.
fn radius(fields: &Fields, units: Unit) -> Result<f64, Error> {
    let node = fields.require("radius")?;
    let path = fields.path_of("radius");
    let radius = length(node, &path, units)?;
    if radius > 0.0 {
        Ok(radius)
    } else {
        Err(bad_value(node, &path, "a radius is more than 0"))
    }
}

fn ellipse(fields: &Fields, units: Unit, layer: String) -> Result<Ellipse, Error> {
    let centre = point(fields.require("centre")?, &fields.path_of("centre"), units)?;
    let axis_node = fields.require("major_axis")?;
    let axis_path = fields.path_of("major_axis");
    let major_axis = point(axis_node, &axis_path, units)?;
    if major_axis.z != 0.0 {
        return Err(bad_value(
            axis_node,
            &axis_path,
            "an ellipse lies flat, parallel to the XY plane, so its major axis has 0 for dz",
        ));
    }
    if major_axis.x == 0.0 && major_axis.y == 0.0 {
        return Err(bad_value(
            axis_node,
            &axis_path,
            "the major axis runs from the centre to one end of the ellipse, so it cannot be [0, 0, 0]",
        ));
    }
    let ratio_node = fields.require("ratio")?;
    let ratio_path = fields.path_of("ratio");
    let ratio = plain_number(
        ratio_node,
        &ratio_path,
        "a ratio, the minor axis over the major axis",
    )?;
    if !(ratio > 0.0 && ratio <= 1.0) {
        return Err(bad_value(
            ratio_node,
            &ratio_path,
            "the ratio of the minor axis to the major axis is more than 0 and at most 1",
        ));
    }
    let (start_angle, end_angle) = match (fields.get("start_angle"), fields.get("end_angle")) {
        (None, None) => (0.0, 360.0),
        (Some(start), Some(end)) => {
            let start_angle = angle(start, &fields.path_of("start_angle"))?;
            let end_angle = angle(end, &fields.path_of("end_angle"))?;
            if start_angle == end_angle {
                return Err(bad_value(
                    end,
                    &fields.path_of("end_angle"),
                    "an elliptical arc's end angle must differ from its start angle; for a whole ellipse leave both out",
                ));
            }
            (start_angle, end_angle)
        }
        (Some(_), None) => return Err(missing_partner(fields, "end_angle", "start_angle")),
        (None, Some(_)) => return Err(missing_partner(fields, "start_angle", "end_angle")),
    };
    Ok(Ellipse {
        centre,
        major_axis,
        ratio,
        start_angle,
        end_angle,
        layer,
    })
}

/// `missing` is required because `given` is there.
fn missing_partner(fields: &Fields, missing: &str, given: &str) -> Error {
    missing_key(
        fields,
        missing,
        format!(
            "missing key `{missing}`: an elliptical arc takes both `start_angle` and `end_angle`, and `{given}` is given; for a whole ellipse leave both out"
        ),
    )
}

/// A key is missing from the mapping `fields` reads, for a reason `message` gives.
fn missing_key(fields: &Fields, key: &str, message: String) -> Error {
    Error::new(ErrorCode::MissingKey, message)
        .at_field(fields.path_of(key))
        .at_position(fields.at.line, fields.at.column)
}

fn spline(fields: &Fields, units: Unit, layer: String) -> Result<Spline, Error> {
    let degree_node = fields.get("degree");
    match (fields.get("fit_points"), fields.get("control_points")) {
        (Some(fit), None) => {
            if let Some(node) = degree_node {
                return Err(bad_value(
                    node,
                    &fields.path_of("degree"),
                    "a spline through fit points is always cubic; `degree` goes with `control_points`",
                ));
            }
            let path = fields.path_of("fit_points");
            let fit_points = points(fit, &path, units, FEWEST_FIT_POINTS)?;
            if let Some(i) = fit_points.windows(2).position(|pair| pair[0] == pair[1]) {
                let Kind::Sequence(items) = &fit.kind else {
                    unreachable!("the fit points were read as a list")
                };
                return Err(bad_value(
                    &items[i + 1],
                    &index_path(&path, i + 1),
                    format!("a curve cannot pass through the same point twice in a row; fit point {} repeats fit point {i}", i + 1),
                ));
            }
            Ok(Spline {
                fit_points,
                control_points: Vec::new(),
                degree: FIT_DEGREE,
                layer,
            })
        }
        (None, Some(control)) => {
            let degree = match degree_node {
                Some(node) => degree(node, &fields.path_of("degree"))?,
                None => FIT_DEGREE,
            };
            let path = fields.path_of("control_points");
            let fewest = usize::from(degree) + 1;
            let control_points = points(control, &path, units, 2)?;
            if control_points.len() < fewest {
                return Err(bad_value(
                    control,
                    &path,
                    format!(
                        "a spline of degree {degree} needs at least {fewest} control points; this one has {}",
                        control_points.len()
                    ),
                ));
            }
            Ok(Spline {
                fit_points: Vec::new(),
                control_points,
                degree,
                layer,
            })
        }
        (Some(_), Some(control)) => Err(at(
            Error::new(
                ErrorCode::BadValue,
                "a spline takes `fit_points` or `control_points`, not both",
            ),
            &fields.path_of("control_points"),
            control.at,
        )),
        (None, None) => Err(missing_key(
            fields,
            "fit_points",
            "missing key `fit_points`: a spline takes `fit_points`, the points it passes through, or `control_points`".to_owned(),
        )),
    }
}

fn degree(node: &Node, path: &str) -> Result<u8, Error> {
    let text = scalar(node, path, "a whole number")?;
    match text.parse::<u8>() {
        Ok(degree)
            if text.bytes().all(|b| b.is_ascii_digit())
                && (1..=HIGHEST_DEGREE).contains(&degree) =>
        {
            Ok(degree)
        }
        _ if text.bytes().all(|b| b.is_ascii_digit()) && !text.is_empty() => Err(bad_value(
            node,
            path,
            format!("a spline's degree is a whole number from 1 to {HIGHEST_DEGREE}"),
        )),
        _ => Err(at(
            Error::new(
                ErrorCode::BadNumber,
                format!(
                    "expected a degree, a whole number from 1 to {HIGHEST_DEGREE}, found `{text}`"
                ),
            ),
            path,
            node.at,
        )),
    }
}
