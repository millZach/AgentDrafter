//! Reads a `wipeout` object (and sheet-level `wipeout_frame`) from a sheet
//! description (#78).

use super::strict_yaml::index_path;
use super::{bad_value, point, scalar, sequence, wrong_type, Fields};
use crate::error::Error;
use crate::model::revision_cloud::signed_area;
use crate::model::{Point3, Properties, Wipeout, WipeoutFrame};
use crate::units::Unit;

/// The keys a `wipeout` object takes in a sheet description.
pub(crate) const WIPEOUT_KEYS: [&str; 6] = [
    "points",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The frame visibility values `wipeout_frame` accepts.
pub(crate) const WIPEOUT_FRAME_NAMES: [&str; 3] = ["off", "on", "display"];

pub(super) fn wipeout_frame(
    node: &super::strict_yaml::Node,
    path: &str,
) -> Result<WipeoutFrame, Error> {
    let text = scalar(node, path, "off, on or display")?;
    match text {
        "off" => Ok(WipeoutFrame::Off),
        "on" => Ok(WipeoutFrame::On),
        "display" => Ok(WipeoutFrame::Display),
        _ => Err(bad_value(
            node,
            path,
            format!(
                "unknown wipeout_frame `{text}`; known values: {}",
                WIPEOUT_FRAME_NAMES.join(", ")
            ),
        )),
    }
}

pub(super) fn wipeout(fields: &Fields, units: Unit, layer: String) -> Result<Wipeout, Error> {
    let node = fields.require("points")?;
    let path = fields.path_of("points");
    let items = sequence(node, &path, "a list of at least 3 points")?;
    if items.len() < 3 {
        return Err(wrong_type(
            node,
            &path,
            &format!(
                "a list of at least 3 points, round the wipeout boundary; this one has {}",
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
                "a wipeout's points must share one z",
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
                "two consecutive wipeout points cannot be the same",
            ));
        }
    }
    if signed_area(&pts).abs() <= 1e-12 {
        return Err(bad_value(
            node,
            &path,
            "a wipeout's boundary must enclose an area more than 0",
        ));
    }
    Ok(Wipeout {
        points: pts,
        layer,
        properties: Properties::default(),
    })
}
