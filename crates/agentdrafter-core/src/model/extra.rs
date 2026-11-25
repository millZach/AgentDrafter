//! Remaining 2D AutoCAD objects: points (`POINT`, `PDMODE`, `PDSIZE`),
//! construction lines (`XLINE`), rays (`RAY`), multilines and their styles
//! (`MLINE`, `MLSTYLE`), geometric tolerances (`TOLERANCE`), centre marks
//! (`CENTERMARK`), centre lines (`CENTERLINE`), and SHX shapes (`SHAPE`).

use serde::{Deserialize, Serialize};

use super::{
    is_false, is_one, is_true, is_zero, layer_zero, one, yes, ObjectColor, ObjectLinetype, Point3,
    Properties, STANDARD_DIMENSION_STYLE,
};

/// The default multiline style every drawing has.
pub const STANDARD_MLINE_STYLE: &str = "Standard";

/// Returns whether `mode` is one of AutoCAD's valid `PDMODE` values:
/// low style `0..=4` combined with `0`, `32` (circle), `64` (square), or `96` (both).
pub fn is_valid_pdmode(mode: i16) -> bool {
    let base = mode & 0x1F;
    let ring = mode & !0x1F;
    (0..=4).contains(&base) && matches!(ring, 0 | 32 | 64 | 96)
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_zero_i16(n: &i16) -> bool {
    *n == 0
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)]
pub(crate) fn is_right_angle(deg: &f64) -> bool {
    (*deg - 90.0).abs() < 1e-9
}

pub(crate) fn right_angle() -> f64 {
    90.0
}

pub(crate) fn standard_mline_style() -> String {
    STANDARD_MLINE_STYLE.to_owned()
}

pub(crate) fn standard_dim_style() -> String {
    STANDARD_DIMENSION_STYLE.to_owned()
}

/// A 2D/3D point (`POINT`), drawn according to the sheet's `pdmode` and `pdsize`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub position: Point3,
    /// Rotation angle of the point display glyph in degrees, anticlockwise from X.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub angle: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

/// An infinite construction line (`XLINE`) through `through` along `direction`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XLine {
    /// A point on the construction line (DXF 10/20/30).
    pub through: Point3,
    /// Unit direction vector (DXF 11/21/31).
    pub direction: Point3,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl XLine {
    /// Unit direction vector in 3D, falling back to `+X` if degenerate.
    pub fn unit_direction(&self) -> Point3 {
        normalize_point3(self.direction)
    }

    /// Angle of the XY projection in degrees `[0, 360)`.
    pub fn angle_degrees(&self) -> f64 {
        let d = self.unit_direction();
        d.y.atan2(d.x).to_degrees().rem_euclid(360.0)
    }
}

/// A semi-infinite ray (`RAY`) starting at `from` and extending along `direction`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    /// Start point of the ray (DXF 10/20/30).
    pub from: Point3,
    /// Unit direction vector (DXF 11/21/31).
    pub direction: Point3,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl Ray {
    /// Unit direction vector in 3D, falling back to `+X` if degenerate.
    pub fn unit_direction(&self) -> Point3 {
        normalize_point3(self.direction)
    }

    /// Angle of the XY projection in degrees `[0, 360)`.
    pub fn angle_degrees(&self) -> f64 {
        let d = self.unit_direction();
        d.y.atan2(d.x).to_degrees().rem_euclid(360.0)
    }
}

/// Normalizes a 3D vector, defaulting to `[1, 0, 0]` if near-zero.
pub fn normalize_point3(v: Point3) -> Point3 {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len > 1e-12 {
        Point3::new(v.x / len, v.y / len, v.z / len)
    } else {
        Point3::new(1.0, 0.0, 0.0)
    }
}

/// One parallel element line in an [`MLineStyle`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLineElement {
    /// Signed perpendicular offset from the multiline's zero axis.
    pub offset: f64,
    #[serde(default, skip_serializing_if = "ObjectColor::is_by_layer")]
    pub color: ObjectColor,
    #[serde(default, skip_serializing_if = "ObjectLinetype::is_by_layer")]
    pub linetype: ObjectLinetype,
}

impl MLineElement {
    pub fn new(offset: f64) -> Self {
        Self {
            offset,
            color: ObjectColor::ByLayer,
            linetype: ObjectLinetype::ByLayer,
        }
    }
}

/// A named multiline style (`MLSTYLE` / `MLINESTYLE`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLineStyle {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Parallel elements, each with its offset, colour and linetype.
    pub elements: Vec<MLineElement>,
    /// Fill colour between the outermost elements when background fill is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<ObjectColor>,
    /// Whether miter lines are drawn at interior vertices.
    #[serde(default, skip_serializing_if = "is_false")]
    pub display_joints: bool,
    /// Start cap angle in degrees (`10..=170`, default `90`).
    #[serde(default = "right_angle", skip_serializing_if = "is_right_angle")]
    pub start_angle: f64,
    /// End cap angle in degrees (`10..=170`, default `90`).
    #[serde(default = "right_angle", skip_serializing_if = "is_right_angle")]
    pub end_angle: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_square_cap: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_round_cap: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_inner_arcs_cap: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub end_square_cap: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub end_round_cap: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub end_inner_arcs_cap: bool,
}

impl MLineStyle {
    /// AutoCAD's built-in `Standard` multiline style: two lines at `+0.5` and `-0.5`.
    pub fn standard() -> Self {
        Self {
            name: STANDARD_MLINE_STYLE.to_owned(),
            description: String::new(),
            elements: vec![MLineElement::new(0.5), MLineElement::new(-0.5)],
            fill_color: None,
            display_joints: false,
            start_angle: 90.0,
            end_angle: 90.0,
            start_square_cap: false,
            start_round_cap: false,
            start_inner_arcs_cap: false,
            end_square_cap: false,
            end_round_cap: false,
            end_inner_arcs_cap: false,
        }
    }

    /// Min and max element offsets, or `(0.0, 0.0)` when empty.
    pub fn offset_range(&self) -> (f64, f64) {
        if self.elements.is_empty() {
            return (0.0, 0.0);
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for e in &self.elements {
            lo = lo.min(e.offset);
            hi = hi.max(e.offset);
        }
        (lo, hi)
    }
}

/// How an [`MLine`] is aligned relative to its vertex path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MLineJustification {
    Top,
    #[default]
    Zero,
    Bottom,
}

impl MLineJustification {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_zero(&self) -> bool {
        *self == Self::Zero
    }
}

/// A multiline (`MLINE`): parallel offset segments controlled by an [`MLineStyle`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLine {
    /// Path vertices (at least 2).
    pub points: Vec<Point3>,
    /// Name of the multiline style in `mline_styles` (or `"Standard"`).
    #[serde(default = "standard_mline_style")]
    pub style: String,
    /// Width multiplier applied to the style's element offsets (default `1.0`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// Alignment of the style relative to `points` (`top`, `zero`, `bottom`).
    #[serde(default, skip_serializing_if = "MLineJustification::is_zero")]
    pub justification: MLineJustification,
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub suppress_start_caps: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub suppress_end_caps: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl MLine {
    /// Returns the effective signed perpendicular offset for style element `offset`
    /// after applying `justification` and `scale`.
    pub fn effective_offset(&self, element_offset: f64, style: &MLineStyle) -> f64 {
        let (min_off, max_off) = style.offset_range();
        let shift = match self.justification {
            MLineJustification::Top => max_off,
            MLineJustification::Zero => 0.0,
            MLineJustification::Bottom => min_off,
        };
        (element_offset - shift) * self.scale
    }
}

/// A geometric dimensioning & tolerancing (GD&T) feature control frame (`TOLERANCE`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    /// Insertion point (middle-left of the first frame row, DXF 10/20/30).
    pub position: Point3,
    /// Horizontal direction angle in degrees, anticlockwise from +X (DXF 11/21/31).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub direction: f64,
    /// Dimension style governing text height and frame gap (default `"Standard"`).
    #[serde(default = "standard_dim_style")]
    pub dimension_style: String,
    /// AutoCAD feature control frame string (`%%v` cell separators, `^J` / `\n` rows,
    /// `{\Fgdt;<letter>}` GDT symbols).
    pub text: String,
    /// Optional explicit text height override (defaults to the dimension style's text height).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_height: Option<f64>,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

/// A circular centre mark (`CENTERMARK`): central crosshair and optional extension lines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CenterMark {
    pub center: Point3,
    /// Radius of the reference circle or arc.
    pub radius: f64,
    /// Width/height of the central cross; defaults to `0.2 * radius` (10% of diameter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_size: Option<f64>,
    /// Gap between the central cross and the extension lines; defaults to `0.1 * radius`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_gap: Option<f64>,
    /// Length that extension lines overshoot beyond `radius`; defaults to `0.24 * radius`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<f64>,
    /// Whether the four radial extension lines are drawn (default `true`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub show_extensions: bool,
    /// Rotation angle in degrees, anticlockwise from +X.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl CenterMark {
    pub fn effective_cross_size(&self) -> f64 {
        self.cross_size
            .unwrap_or_else(|| (self.radius * 0.2).max(1e-6))
    }

    pub fn effective_cross_gap(&self) -> f64 {
        self.cross_gap
            .unwrap_or_else(|| (self.radius * 0.1).max(0.0))
    }

    pub fn effective_extension(&self) -> f64 {
        self.extension
            .unwrap_or_else(|| (self.radius * 0.24).max(0.0))
    }

    /// The line segments that draw this centre mark in world coordinates:
    /// two central crosshair lines plus four optional extension lines.
    pub fn segments(&self) -> Vec<[Point3; 2]> {
        let rad = self.rotation.to_radians();
        let (sin, cos) = rad.sin_cos();
        let rot = |lx: f64, ly: f64| {
            Point3::new(
                self.center.x + lx * cos - ly * sin,
                self.center.y + lx * sin + ly * cos,
                self.center.z,
            )
        };
        let half = self.effective_cross_size() * 0.5;
        let mut segs = vec![
            [rot(-half, 0.0), rot(half, 0.0)],
            [rot(0.0, -half), rot(0.0, half)],
        ];
        if self.show_extensions {
            let start = half + self.effective_cross_gap();
            let end = self.radius + self.effective_extension();
            if end > start + 1e-9 {
                segs.push([rot(start, 0.0), rot(end, 0.0)]);
                segs.push([rot(-start, 0.0), rot(-end, 0.0)]);
                segs.push([rot(0.0, start), rot(0.0, end)]);
                segs.push([rot(0.0, -start), rot(0.0, -end)]);
            }
        }
        segs
    }
}

/// A centre line (`CENTERLINE`): straight centreline segment with optional end extensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CenterLine {
    pub from: Point3,
    pub to: Point3,
    /// Overshoot before `from` along the line's direction.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start_extension: f64,
    /// Overshoot past `to` along the line's direction.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub end_extension: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl CenterLine {
    /// Returns the drawn start and end points after applying `start_extension`
    /// and `end_extension`.
    pub fn extended_endpoints(&self) -> (Point3, Point3) {
        let dx = self.to.x - self.from.x;
        let dy = self.to.y - self.from.y;
        let dz = self.to.z - self.from.z;
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        if len <= 1e-12 {
            return (self.from, self.to);
        }
        let ux = dx / len;
        let uy = dy / len;
        let uz = dz / len;
        let p0 = Point3::new(
            self.from.x - ux * self.start_extension,
            self.from.y - uy * self.start_extension,
            self.from.z - uz * self.start_extension,
        );
        let p1 = Point3::new(
            self.to.x + ux * self.end_extension,
            self.to.y + uy * self.end_extension,
            self.to.z + uz * self.end_extension,
        );
        (p0, p1)
    }
}

/// A shape placed from an SHX shape file (`SHAPE`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShxShape {
    /// Shape file name (such as `myshapes.shx`).
    pub file: String,
    /// Shape name inside `file` (DXF code 2); may be empty when read by number alone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Shape number inside `file` (DWG shape index); resolved from `file` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i32>,
    /// Insertion point in WCS (DXF 10/20/30).
    pub position: Point3,
    /// Size scale factor applied to the shape's definition units (DXF 40).
    pub size: f64,
    /// Rotation angle in degrees, anticlockwise from +X (DXF 50).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    /// Relative X scale factor (DXF 41, default `1.0`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub width_factor: f64,
    /// Oblique slant angle in degrees (DXF 51, default `0.0`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub oblique: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

/// Runtime-resolved strokes of a shape loaded from an `.shx` shape file.
/// Never serialized into sheet descriptions.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedShape {
    /// Shape file name as referenced on [`ShxShape::file`].
    pub file: String,
    /// Shape name inside the `.shx` file (empty if unnamed).
    pub name: String,
    /// Shape number inside the `.shx` file.
    pub number: i32,
    /// Unit-scale polylines (in the `.shx` file's own shape units; scaled by `ShxShape::size`).
    pub strokes: Vec<Vec<[f64; 2]>>,
}
