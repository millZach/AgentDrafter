//! Reads polylines, with their arc segments and widths, donuts and 2D
//! solids.
//!
//! A polyline's point is `[x, y, z]`, or a mapping when it carries more:
//! `{point: [x, y, z], bulge: ..., start_width: ..., end_width: ...}`, with
//! `arc_angle` in place of `bulge` if wanted, as Zach decided on
//! 2026-11-13.

use super::strict_yaml::{index_path, Kind, Node};
use super::{bad_value, boolean, length, plain_number, point, sequence, wrong_type, Fields};
use crate::draw::bulge_of;
use crate::error::{Error, ErrorCode};
use crate::model::{Donut, Point3, Polyline, Properties, Solid};
use crate::units::Unit;

/// The keys a polyline's point takes when it is a mapping.
pub(crate) const VERTEX_KEYS: [&str; 5] =
    ["point", "bulge", "arc_angle", "start_width", "end_width"];

/// What a point of a polyline gives besides where it is.
#[derive(Default)]
struct Extras {
    bulge: Option<f64>,
    widths: Option<[f64; 2]>,
    /// The first key that gives an arc or a width, for an error.
    first: Option<(String, Node)>,
}

pub(super) fn polyline(fields: &Fields, units: Unit, layer: String) -> Result<Polyline, Error> {
    let closed = match fields.get("closed") {
        Some(node) => boolean(node, &fields.path_of("closed"))?,
        None => false,
    };
    let path = fields.path_of("points");
    let node = fields.require("points")?;
    let items = sequence(node, &path, "a list of points")?;
    // A closed polyline needs three points to enclose anything.
    let fewest = if closed { 3 } else { 2 };
    if items.len() < fewest {
        return Err(super::at(
            Error::new(
                ErrorCode::WrongType,
                format!("expected at least {fewest} points, found {}", items.len()),
            ),
            &path,
            node.at,
        ));
    }
    let mut points = Vec::with_capacity(items.len());
    let mut extras = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = index_path(&path, i);
        let (p, extra) = vertex(item, &at, units, i + 1 == items.len() && !closed)?;
        points.push(p);
        extras.push(extra);
    }
    let width = match fields.get("width") {
        Some(node) => {
            let path = fields.path_of("width");
            let width = length(node, &path, units)?;
            if width < 0.0 {
                return Err(bad_value(node, &path, "a width is 0 or more"));
            }
            if let Some(e) = extras.iter().find(|e| e.widths.is_some()) {
                let (key, at) = e.first.clone().expect("a point with a width names its key");
                return Err(bad_value(
                    &at,
                    &key,
                    "a polyline gives one `width` for every segment, or each point its own \
                     `start_width` and `end_width`, not both",
                ));
            }
            width
        }
        None => 0.0,
    };
    let level = points.first().map_or(0.0, |p: &Point3| p.z);
    if points.iter().any(|p| p.z != level) {
        let given = extras.iter().find_map(|e| e.first.clone()).or_else(|| {
            (width != 0.0).then(|| {
                (
                    fields.path_of("width"),
                    fields.get("width").cloned().expect("given"),
                )
            })
        });
        if let Some((key, at)) = given {
            return Err(bad_value(
                &at,
                &key,
                "a polyline whose points do not share one `z` is a 3D polyline, which has no \
                 arc segments and no widths, as in AutoCAD",
            ));
        }
    }
    let bulges = if extras.iter().any(|e| e.bulge.is_some_and(|b| b != 0.0)) {
        extras.iter().map(|e| e.bulge.unwrap_or(0.0)).collect()
    } else {
        Vec::new()
    };
    let widths = if extras.iter().any(|e| e.widths.is_some()) {
        extras
            .iter()
            .map(|e| e.widths.unwrap_or([0.0, 0.0]))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Polyline {
        points,
        bulges,
        widths,
        width,
        closed,
        layer,
        properties: Properties::default(),
    })
}

/// One point of a polyline: `[x, y, z]`, or a mapping that gives its
/// point, the bulge or arc angle of the segment that starts there, and
/// that segment's widths. `last` is whether it starts no segment: the last
/// point of an open polyline.
fn vertex(node: &Node, path: &str, units: Unit, last: bool) -> Result<(Point3, Extras), Error> {
    let Kind::Mapping(_) = &node.kind else {
        return Ok((point(node, path, units)?, Extras::default()));
    };
    let fields = Fields::of(node, path, &VERTEX_KEYS, "a polyline's point")?;
    let at = fields.require("point")?;
    let p = point(at, &fields.path_of("point"), units)?;
    let mut extra = Extras::default();
    let note = |extra: &mut Extras, key: &str, node: &Node| {
        if extra.first.is_none() {
            extra.first = Some((fields.path_of(key), node.clone()));
        }
    };
    if let (Some(_), Some(node)) = (fields.get("bulge"), fields.get("arc_angle")) {
        return Err(bad_value(
            node,
            &fields.path_of("arc_angle"),
            "a segment's arc is given by `bulge` or by `arc_angle`, not both",
        ));
    }
    if let Some(node) = fields.get("bulge") {
        let path = fields.path_of("bulge");
        let bulge = plain_number(node, &path, "a bulge, a number such as `0.5`")?;
        extra.bulge = Some(bulge);
        note(&mut extra, "bulge", node);
    }
    if let Some(node) = fields.get("arc_angle") {
        let path = fields.path_of("arc_angle");
        let degrees = plain_number(
            node,
            &path,
            "an arc's included angle in degrees, such as `90`",
        )?;
        if degrees.abs() >= 360.0 {
            return Err(bad_value(
                node,
                &path,
                "an arc's included angle is more than -360 and less than 360 degrees, positive \
                 anticlockwise; a whole circle is a `circle`",
            ));
        }
        extra.bulge = Some(bulge_of(degrees));
        note(&mut extra, "arc_angle", node);
    }
    let mut widths = [0.0, 0.0];
    let mut has_width = false;
    for (i, key) in ["start_width", "end_width"].into_iter().enumerate() {
        if let Some(node) = fields.get(key) {
            let path = fields.path_of(key);
            let width = length(node, &path, units)?;
            if width < 0.0 {
                return Err(bad_value(node, &path, "a width is 0 or more"));
            }
            widths[i] = width;
            has_width = true;
            note(&mut extra, key, node);
        }
    }
    if has_width {
        extra.widths = Some(widths);
    }
    if last {
        if let Some((key, node)) = &extra.first {
            return Err(bad_value(
                node,
                key,
                "the last point of an open polyline starts no segment, so it takes no arc and no \
                 width; give them on the point the segment starts at",
            ));
        }
    }
    Ok((p, extra))
}

pub(super) fn donut(fields: &Fields, units: Unit, layer: String) -> Result<Donut, Error> {
    let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
    let node = fields.require("outside_diameter")?;
    let path = fields.path_of("outside_diameter");
    let outside_diameter = length(node, &path, units)?;
    if outside_diameter <= 0.0 {
        return Err(bad_value(
            node,
            &path,
            "a donut's outside diameter is more than 0",
        ));
    }
    let inside_diameter = match fields.get("inside_diameter") {
        Some(node) => {
            let path = fields.path_of("inside_diameter");
            let inside = length(node, &path, units)?;
            if !(0.0..outside_diameter).contains(&inside) {
                return Err(bad_value(
                    node,
                    &path,
                    "a donut's inside diameter is 0 or more and less than its outside diameter; \
                     0 draws a filled dot",
                ));
            }
            inside
        }
        None => 0.0,
    };
    Ok(Donut {
        center,
        inside_diameter,
        outside_diameter,
        layer,
        properties: Properties::default(),
    })
}

pub(super) fn solid(fields: &Fields, units: Unit, layer: String) -> Result<Solid, Error> {
    let node = fields.require("corners")?;
    let path = fields.path_of("corners");
    let items = sequence(node, &path, "a list of 3 or 4 corners")?;
    if !(3..=4).contains(&items.len()) {
        return Err(wrong_type(
            node,
            &path,
            &format!(
                "a list of 3 or 4 corners, round the outline; this one has {}",
                items.len()
            ),
        ));
    }
    let corners = items
        .iter()
        .enumerate()
        .map(|(i, item)| point(item, &index_path(&path, i), units))
        .collect::<Result<Vec<_>, _>>()?;
    if corners.iter().any(|c| c.z != corners[0].z) {
        return Err(bad_value(
            node,
            &path,
            "a 2D solid's corners share one `z`: it lies flat",
        ));
    }
    Ok(Solid {
        corners,
        layer,
        properties: Properties::default(),
    })
}
