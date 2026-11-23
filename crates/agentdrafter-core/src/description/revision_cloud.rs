//! Reads a `revision_cloud` object from a sheet description (#72): either a
//! rectangle (`rectangle: {from, to}`) or a closed polygon (`polygon: [...]`),
//! with an `arc_length` and a `style` (`normal` or `calligraphy`).

use super::strict_yaml::index_path;
use super::{bad_value, boolean, length, point, scalar, sequence, wrong_type, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::revision_cloud::signed_area;
use crate::model::{
    Point3, Properties, RevisionCloud, RevisionCloudBoundary, RevisionCloudStyle,
    MOST_REVCLOUD_ARCS,
};
use crate::units::Unit;

/// The keys a `revision_cloud` object takes in a sheet description.
pub(crate) const REVISION_CLOUD_KEYS: [&str; 10] = [
    "rectangle",
    "polygon",
    "arc_length",
    "style",
    "reverse",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The keys a revision cloud's `rectangle` mapping takes.
pub(crate) const RECTANGLE_KEYS: [&str; 2] = ["from", "to"];

/// The style names a revision cloud accepts.
pub(crate) const STYLE_NAMES: [&str; 2] = ["normal", "calligraphy"];

pub(super) fn revision_cloud(
    fields: &Fields,
    units: Unit,
    layer: String,
) -> Result<RevisionCloud, Error> {
    let rect_node = fields.get("rectangle");
    let poly_node = fields.get("polygon");
    let boundary = match (rect_node, poly_node) {
        (Some(_), Some(_)) => {
            return Err(bad_value(
                poly_node.unwrap(),
                &fields.path_of("polygon"),
                "give either `rectangle` or `polygon`, not both",
            ));
        }
        (None, None) => {
            return Err(super::at(
                Error::new(
                    ErrorCode::MissingKey,
                    "a revision cloud needs either `rectangle` or `polygon`",
                ),
                fields.path,
                fields.at,
            ));
        }
        (Some(node), None) => {
            let path = fields.path_of("rectangle");
            let rect = Fields::of(node, &path, &RECTANGLE_KEYS, "a rectangle")?;
            let from = point(rect.require("from")?, &rect.path_of("from"), units)?;
            let to = point(rect.require("to")?, &rect.path_of("to"), units)?;
            if from.z != to.z {
                return Err(bad_value(
                    rect.require("to")?,
                    &rect.path_of("to"),
                    "a revision cloud's rectangle corners must share one z",
                ));
            }
            if from.x == to.x || from.y == to.y {
                return Err(bad_value(
                    node,
                    &path,
                    "a revision cloud's rectangle must have width and height more than 0",
                ));
            }
            RevisionCloudBoundary::Rectangle { from, to }
        }
        (None, Some(node)) => {
            let path = fields.path_of("polygon");
            let items = sequence(node, &path, "a list of at least 3 points")?;
            if items.len() < 3 {
                return Err(wrong_type(
                    node,
                    &path,
                    &format!(
                        "a list of at least 3 points, round the boundary; this one has {}",
                        items.len()
                    ),
                ));
            }
            let mut pts: Vec<Point3> = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                pts.push(point(item, &index_path(&path, i), units)?);
            }
            let z = pts[0].z;
            for (i, p) in pts.iter().enumerate().skip(1) {
                if p.z != z {
                    return Err(bad_value(
                        &items[i],
                        &index_path(&path, i),
                        "a revision cloud's polygon points must share one z",
                    ));
                }
            }
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                if a.x == b.x && a.y == b.y {
                    let at_idx = (i + 1) % pts.len();
                    return Err(bad_value(
                        &items[at_idx],
                        &index_path(&path, at_idx),
                        "two consecutive polygon points cannot be the same",
                    ));
                }
            }
            if signed_area(&pts).abs() <= 1e-12 {
                return Err(bad_value(
                    node,
                    &path,
                    "a revision cloud's polygon must enclose an area more than 0",
                ));
            }
            RevisionCloudBoundary::Polygon(pts)
        }
    };

    let arc_node = fields.require("arc_length")?;
    let arc_path = fields.path_of("arc_length");
    let arc_length = length(arc_node, &arc_path, units)?;
    if !(arc_length.is_finite() && arc_length > 0.0) {
        return Err(bad_value(
            arc_node,
            &arc_path,
            "a revision cloud's arc_length is more than 0",
        ));
    }

    let style = match fields.get("style") {
        Some(node) => {
            let path = fields.path_of("style");
            let text = scalar(node, &path, "normal or calligraphy")?;
            match text {
                "normal" => RevisionCloudStyle::Normal,
                "calligraphy" => RevisionCloudStyle::Calligraphy,
                _ => {
                    return Err(bad_value(
                        node,
                        &path,
                        "a revision cloud's style is `normal` or `calligraphy`",
                    ));
                }
            }
        }
        None => RevisionCloudStyle::Normal,
    };

    let reverse = match fields.get("reverse") {
        Some(node) => boolean(node, &fields.path_of("reverse"))?,
        None => false,
    };

    let cloud = RevisionCloud {
        boundary,
        arc_length,
        style,
        reverse,
        layer,
        properties: Properties::default(),
    };
    let arcs = cloud.arc_count();
    if arcs > MOST_REVCLOUD_ARCS {
        return Err(bad_value(
            arc_node,
            &arc_path,
            format!(
                "arc_length makes {arcs} arcs, more than the limit of {MOST_REVCLOUD_ARCS}; \
                 give a larger arc_length"
            ),
        ));
    }
    Ok(cloud)
}
