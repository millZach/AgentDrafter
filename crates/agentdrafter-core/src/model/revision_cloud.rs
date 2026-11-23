//! A revision cloud: a closed chain of outward-bulging arcs round a
//! rectangle or a polygon, as AutoCAD's REVCLOUD draws it (#72).
//!
//! Measured against AutoCAD 2026's own `REVCLOUD` output:
//! - Written as a closed lightweight polyline (`LWPOLYLINE`) flat at the
//!   boundary's `z`, with vertices stepping round the boundary in
//!   anticlockwise order so that a positive bulge of `0.52056705` (a 110°
//!   arc, `tan(27.5°)`) bulges outward; `reverse: true` negates the bulge so
//!   the arcs bulge inward.
//! - Normal style keeps uniform zero polyline width (drawing at the object's
//!   or layer's lineweight). Calligraphy style tapers every arc from a start
//!   width of `0` to an end width of `0.1 * arc_length`.
//! - Written to DWG with AutoCAD's `RevcloudProps` extended data
//!   (`1070 = 0`, `1040 = arc_length`) so AutoCAD recognizes the object as a
//!   revision cloud.

use serde::{Deserialize, Serialize};

use super::{layer_zero, Point3, Polyline, Properties};

/// The bulge AutoCAD 2026 writes on every arc of a revision cloud:
/// `tan(110° / 4) = tan(27.5°)` rounded to eight decimal places.
pub const REVCLOUD_BULGE: f64 = 0.520_567_05;

/// The end width of each arc in calligraphy style, as a share of `arc_length`,
/// measured from AutoCAD 2026 (`0.05` at `arc_length = 0.5`).
pub const REVCLOUD_CALLIGRAPHY_WIDTH_FACTOR: f64 = 0.1;

/// The registered application name AutoCAD uses on a revision cloud's
/// extended data.
pub const REVCLOUD_APPLICATION: &str = "RevcloudProps";

/// The most arc segments one revision cloud may generate.
pub const MOST_REVCLOUD_ARCS: usize = 10_000;

/// How a revision cloud's arcs are drawn: AutoCAD's Normal or Calligraphy
/// style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionCloudStyle {
    /// Uniform lineweight along every arc.
    #[default]
    Normal,
    /// Tapered calligraphic strokes from `0` to `0.1 * arc_length` on each arc.
    Calligraphy,
}

impl RevisionCloudStyle {
    /// The name a sheet description and schema write for this style.
    pub fn as_str(self) -> &'static str {
        match self {
            RevisionCloudStyle::Normal => "normal",
            RevisionCloudStyle::Calligraphy => "calligraphy",
        }
    }

    pub fn is_normal(&self) -> bool {
        matches!(self, RevisionCloudStyle::Normal)
    }
}

/// The closed boundary a revision cloud steps around.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionCloudBoundary {
    /// An axis-aligned rectangle given by two opposite corners sharing one `z`.
    Rectangle { from: Point3, to: Point3 },
    /// A closed polygon of at least three corners sharing one `z`.
    Polygon(Vec<Point3>),
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A revision cloud drawn round a rectangle or a polygon, as AutoCAD's
/// `REVCLOUD` draws one (#72).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionCloud {
    pub boundary: RevisionCloudBoundary,
    /// The target chord length of each cloud arc, more than 0.
    pub arc_length: f64,
    #[serde(default, skip_serializing_if = "RevisionCloudStyle::is_normal")]
    pub style: RevisionCloudStyle,
    /// When true, arcs bulge inward instead of outward (AutoCAD's Reverse
    /// direction).
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl RevisionCloud {
    /// The boundary's corners in anticlockwise order (positive signed area)
    /// so that a positive bulge always bows outward.
    pub fn corners(&self) -> Vec<Point3> {
        match &self.boundary {
            RevisionCloudBoundary::Rectangle { from, to } => {
                let min_x = from.x.min(to.x);
                let max_x = from.x.max(to.x);
                let min_y = from.y.min(to.y);
                let max_y = from.y.max(to.y);
                let z = from.z;
                vec![
                    Point3::new(min_x, min_y, z),
                    Point3::new(max_x, min_y, z),
                    Point3::new(max_x, max_y, z),
                    Point3::new(min_x, max_y, z),
                ]
            }
            RevisionCloudBoundary::Polygon(pts) => {
                let mut corners = pts.clone();
                if signed_area(&corners) < 0.0 {
                    corners.reverse();
                }
                corners
            }
        }
    }

    /// How many arc segments this cloud generates when lowered to a polyline.
    pub fn arc_count(&self) -> usize {
        if !(self.arc_length.is_finite() && self.arc_length > 0.0) {
            return 0;
        }
        let corners = self.corners();
        if corners.len() < 3 {
            return 0;
        }
        let mut total: usize = 0;
        for i in 0..corners.len() {
            let a = corners[i];
            let b = corners[(i + 1) % corners.len()];
            let len = (b.x - a.x).hypot(b.y - a.y);
            let steps = edge_steps(len, self.arc_length);
            total = total.saturating_add(steps);
        }
        total
    }

    /// The closed lightweight polyline AutoCAD's `REVCLOUD` writes: vertices
    /// stepped evenly along each edge of the boundary, each arc bulging by
    /// [`REVCLOUD_BULGE`] (negated when `reverse` is true), with calligraphy
    /// widths `[0, 0.1 * arc_length]` when `style` is `Calligraphy`.
    pub fn polyline(&self) -> Polyline {
        let corners = self.corners();
        let mut points = Vec::new();
        for i in 0..corners.len() {
            let a = corners[i];
            let b = corners[(i + 1) % corners.len()];
            let len = (b.x - a.x).hypot(b.y - a.y);
            let steps = edge_steps(len, self.arc_length);
            for s in 0..steps {
                #[allow(clippy::cast_precision_loss)]
                let t = s as f64 / steps as f64;
                points.push(Point3::new(
                    a.x + (b.x - a.x) * t,
                    a.y + (b.y - a.y) * t,
                    a.z,
                ));
            }
        }
        let count = points.len();
        let bulge = if self.reverse {
            -REVCLOUD_BULGE
        } else {
            REVCLOUD_BULGE
        };
        let bulges = vec![bulge; count];
        let widths = match self.style {
            RevisionCloudStyle::Normal => Vec::new(),
            RevisionCloudStyle::Calligraphy => {
                vec![[0.0, self.arc_length * REVCLOUD_CALLIGRAPHY_WIDTH_FACTOR]; count]
            }
        };
        Polyline {
            points,
            bulges,
            widths,
            width: 0.0,
            closed: true,
            layer: self.layer.clone(),
            properties: self.properties.clone(),
        }
    }

    /// A clean default arc length for a cloud framing a box of `width` by
    /// `height`, used by `add_revision_cloud` with `around` when no
    /// `arc_length` is given.
    pub fn default_arc_length(width: f64, height: f64) -> f64 {
        let span = (width + height).max(0.0);
        let raw = if span > 0.0 { span / 12.0 } else { 0.5 };
        // Round to 4 decimal places so YAML stays readable and deterministic.
        let rounded = (raw * 10_000.0).round() / 10_000.0;
        if rounded > 0.0 {
            rounded
        } else {
            raw.max(1e-4)
        }
    }

    /// The default clearance around an object's extents when `margin` is not
    /// given on `add_revision_cloud` with `around`.
    pub fn default_margin(arc_length: f64) -> f64 {
        let raw = arc_length * 0.5;
        let rounded = (raw * 10_000.0).round() / 10_000.0;
        if rounded > 0.0 {
            rounded
        } else {
            raw
        }
    }
}

/// Twice the signed area of a closed 2D polygon (positive anticlockwise).
pub(crate) fn signed_area(points: &[Point3]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum * 0.5
}

fn edge_steps(edge_len: f64, arc_length: f64) -> usize {
    if !(edge_len.is_finite() && edge_len > 0.0 && arc_length.is_finite() && arc_length > 0.0) {
        return 1;
    }
    let count = (edge_len / arc_length).round();
    if !count.is_finite() || count <= 1.0 {
        1
    } else if count >= MOST_REVCLOUD_ARCS as f64 {
        MOST_REVCLOUD_ARCS.saturating_add(1)
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            count as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_matches_autocad_2026_vertex_count_bulge_and_calligraphy() {
        let cloud = RevisionCloud {
            boundary: RevisionCloudBoundary::Rectangle {
                from: Point3::new(0.0, 0.0, 0.0),
                to: Point3::new(4.0, 2.0, 0.0),
            },
            arc_length: 0.5,
            style: RevisionCloudStyle::Calligraphy,
            reverse: false,
            layer: "0".into(),
            properties: Properties::default(),
        };
        let poly = cloud.polyline();
        assert!(poly.closed);
        assert_eq!(poly.points.len(), 24);
        assert_eq!(poly.points[0], Point3::new(0.0, 0.0, 0.0));
        assert_eq!(poly.points[8], Point3::new(4.0, 0.0, 0.0));
        assert_eq!(poly.points[12], Point3::new(4.0, 2.0, 0.0));
        assert_eq!(poly.points[20], Point3::new(0.0, 2.0, 0.0));
        assert!(poly.bulges.iter().all(|&b| b == REVCLOUD_BULGE));
        assert!(poly.widths.iter().all(|&w| w == [0.0, 0.05]));
        assert_eq!(poly.width, 0.0);
    }

    #[test]
    fn clockwise_polygon_normalizes_ccw_so_bulges_face_outward() {
        let cw = RevisionCloud {
            boundary: RevisionCloudBoundary::Polygon(vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
            ]),
            arc_length: 1.0,
            style: RevisionCloudStyle::Normal,
            reverse: false,
            layer: "0".into(),
            properties: Properties::default(),
        };
        assert!(signed_area(&cw.corners()) > 0.0);
        let poly = cw.polyline();
        assert!(poly.bulges.iter().all(|&b| b == REVCLOUD_BULGE));
        assert!(poly.widths.is_empty());
    }

    #[test]
    fn reverse_negates_bulge() {
        let cloud = RevisionCloud {
            boundary: RevisionCloudBoundary::Rectangle {
                from: Point3::new(0.0, 0.0, 0.0),
                to: Point3::new(2.0, 2.0, 0.0),
            },
            arc_length: 1.0,
            style: RevisionCloudStyle::Normal,
            reverse: true,
            layer: "0".into(),
            properties: Properties::default(),
        };
        let poly = cloud.polyline();
        assert!(poly.bulges.iter().all(|&b| b == -REVCLOUD_BULGE));
    }
}
