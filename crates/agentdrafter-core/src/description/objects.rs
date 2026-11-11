//! Reads the objects on a sheet and in a block: each is a mapping with one
//! key, its type.
//!
//! A sheet and a block hold the same drawn objects. A sheet also holds
//! inserts of blocks, and a block also holds attribute definitions;
//! `blocks.rs` reads both.

use super::blocks::{attribute_definition, insert, ATTDEF_KEYS, INSERT_KEYS};
use super::hatch::{self, HATCH_KEYS};
use super::layers::layer_name;
use super::strict_yaml::{index_path, key_path, Kind, Node, Pos};
use super::text::{mtext, text, MTEXT_KEYS, TEXT_KEYS};
use super::{
    angle, at, bad_value, boolean, length, plain_number, point, points, scalar, unknown_key,
    wrong_type, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Arc, BlockLibrary, Circle, Ellipse, HatchPattern, Layer, Line, Object, Polyline, Spline,
    FIT_DEGREE, LAYER_ZERO,
};
use crate::text::TextStyle;
use crate::units::Unit;

/// Every object type a sheet description can hold, and the keys each takes.
pub(crate) const OBJECT_TYPES: [(&str, &[&str]); 10] = [
    ("line", &LINE_KEYS),
    ("polyline", &POLYLINE_KEYS),
    ("arc", &ARC_KEYS),
    ("circle", &CIRCLE_KEYS),
    ("ellipse", &ELLIPSE_KEYS),
    ("spline", &SPLINE_KEYS),
    ("hatch", &HATCH_KEYS),
    ("text", &TEXT_KEYS),
    ("mtext", &MTEXT_KEYS),
    ("insert", &INSERT_KEYS),
];

/// Every object type a block in a block library can hold, and the keys
/// each takes: the drawn objects a sheet holds, and attribute definitions.
pub(crate) const BLOCK_OBJECT_TYPES: [(&str, &[&str]); 10] = [
    ("line", &LINE_KEYS),
    ("polyline", &POLYLINE_KEYS),
    ("arc", &ARC_KEYS),
    ("circle", &CIRCLE_KEYS),
    ("ellipse", &ELLIPSE_KEYS),
    ("spline", &SPLINE_KEYS),
    ("hatch", &HATCH_KEYS),
    ("text", &TEXT_KEYS),
    ("mtext", &MTEXT_KEYS),
    ("attdef", &ATTDEF_KEYS),
];

/// What the objects of one sheet or one block may name: the declared unit
/// and the layers, hatch patterns and text styles defined beside them.
pub(super) struct Tables<'a> {
    pub units: Unit,
    pub layers: &'a [Layer],
    pub hatch_patterns: &'a [HatchPattern],
    pub styles: &'a [TextStyle],
}

/// Where an object is: on a sheet, which may insert blocks from its block
/// library and gives each name once, or in a block, which defines each
/// attribute tag once.
pub(super) enum Place<'a> {
    Sheet {
        library: Option<&'a BlockLibrary>,
        /// Each name given so far, and where.
        names: Vec<(String, Pos)>,
    },
    Block {
        /// Each attribute tag defined so far, and where.
        tags: Vec<(String, Pos)>,
    },
}

impl Place<'_> {
    fn types(&self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Place::Sheet { .. } => &OBJECT_TYPES,
            Place::Block { .. } => &BLOCK_OBJECT_TYPES,
        }
    }

    /// Why an object type belongs elsewhere, for the error that refuses it.
    fn elsewhere(&self, kind: &str) -> Option<&'static str> {
        match (self, kind) {
            (Place::Sheet { .. }, "attdef") => {
                Some("an attribute definition belongs in a block in the block library")
            }
            (Place::Block { .. }, "insert") => {
                Some("a block cannot insert another block; nested blocks are not offered yet")
            }
            _ => None,
        }
    }
}

const LINE_KEYS: [&str; 3] = ["from", "to", "layer"];
const POLYLINE_KEYS: [&str; 3] = ["points", "closed", "layer"];
const ARC_KEYS: [&str; 5] = ["center", "radius", "start_angle", "end_angle", "layer"];
const CIRCLE_KEYS: [&str; 3] = ["center", "radius", "layer"];
const ELLIPSE_KEYS: [&str; 6] = [
    "center",
    "major_axis",
    "ratio",
    "start_angle",
    "end_angle",
    "layer",
];
const SPLINE_KEYS: [&str; 4] = ["fit_points", "control_points", "degree", "layer"];

/// The highest degree a spline by control points can have, as in AutoCAD.
pub(crate) const HIGHEST_DEGREE: u8 = 10;

/// The fewest fit points a spline takes. Through two points it would be a line.
pub(crate) const FEWEST_FIT_POINTS: usize = 3;

pub(super) fn object(
    node: &Node,
    path: &str,
    tables: &Tables,
    place: &mut Place,
) -> Result<Object, Error> {
    const SHAPE: &str = "a mapping with one key naming the object's type, such as `line:`";
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let types = place.types();
    let known = || {
        types
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(entry) = entries
        .iter()
        .find(|e| !types.iter().any(|(name, _)| *name == e.key))
    {
        let what = match place.elsewhere(&entry.key) {
            Some(why) => format!("object type `{}` is not allowed here: {why}", entry.key),
            None => format!("unknown object type `{}`", entry.key),
        };
        return Err(unknown_key(
            entry,
            path,
            &what,
            &format!("known types: {}", known()),
        ));
    }
    let [entry] = entries.as_slice() else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let path = key_path(path, &entry.key);
    let keys = types
        .iter()
        .find(|(name, _)| *name == entry.key)
        .map(|(_, keys)| *keys)
        .unwrap_or_default();
    let article = if entry.key.starts_with(['a', 'e', 'i', 'o', 'u']) || entry.key == "mtext" {
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
        Some(node) => layer_name(node, &fields.path_of("layer"), tables.layers)?,
        None => LAYER_ZERO.to_owned(),
    };
    let units = tables.units;
    match (entry.key.as_str(), place) {
        ("line", _) => line(&fields, units, layer).map(Object::Line),
        ("polyline", _) => polyline(&fields, units, layer).map(Object::Polyline),
        ("arc", _) => arc(&fields, units, layer).map(Object::Arc),
        ("circle", _) => circle(&fields, units, layer).map(Object::Circle),
        ("ellipse", _) => ellipse(&fields, units, layer).map(Object::Ellipse),
        ("spline", _) => spline(&fields, units, layer).map(Object::Spline),
        ("hatch", _) => {
            hatch::hatch(&fields, units, layer, tables.hatch_patterns).map(Object::Hatch)
        }
        ("text", _) => text(&fields, units, layer, tables.styles).map(Object::Text),
        ("mtext", _) => mtext(&fields, units, layer, tables.styles).map(Object::MText),
        ("attdef", Place::Block { tags }) => {
            attribute_definition(&fields, tables, layer, tags).map(Object::AttributeDefinition)
        }
        ("insert", Place::Sheet { library, names }) => {
            insert(&fields, units, layer, *library, names).map(Object::Insert)
        }
        (other, _) => unreachable!("object type `{other}` is listed but not read"),
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
    let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
    let radius = radius(fields, units)?;
    let (start_angle, end_angle) = arc_angles(fields, "for a whole circle use `circle`")?;
    Ok(Arc {
        center,
        radius,
        start_angle,
        end_angle,
        layer,
    })
}

/// An arc's `start_angle` and `end_angle`, both required, which must differ
/// once whole turns are taken off. `whole_circle` says what to write for a
/// whole circle instead.
pub(super) fn arc_angles(fields: &Fields, whole_circle: &str) -> Result<(f64, f64), Error> {
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
            format!("an arc's end angle must differ from its start angle, counting whole turns as none; {whole_circle}"),
        ));
    }
    Ok((start_angle, end_angle))
}

fn circle(fields: &Fields, units: Unit, layer: String) -> Result<Circle, Error> {
    Ok(Circle {
        center: point(fields.require("center")?, &fields.path_of("center"), units)?,
        radius: radius(fields, units)?,
        layer,
    })
}

/// A radius, a length more than 0.
pub(super) fn radius(fields: &Fields, units: Unit) -> Result<f64, Error> {
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
    let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
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
        center,
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
