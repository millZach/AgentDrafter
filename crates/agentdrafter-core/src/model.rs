//! The core's own model of a sheet.
//!
//! The model belongs to AgentDrafter, not to the DWG library: the library is
//! reached only through [`crate::dwg`]. Every coordinate is 3D, so that 3D
//! work later is not a rewrite.
//!
//! Lengths are in the sheet's declared unit. Angles are in degrees, counted
//! anticlockwise from the X axis, and lie in `[0, 360)`. A lineweight is a
//! pen width on paper and is in millimetres whatever the declared unit.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::units::Unit;

pub(crate) mod hatch;
pub use hatch::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, SOLID};

/// The layer every drawing has, and the one an object sits on unless it names another.
pub const LAYER_ZERO: &str = "0";

/// The solid linetype every drawing has. A layer draws with it unless it names another.
pub const CONTINUOUS: &str = "Continuous";

/// The linetypes every drawing has. They are AutoCAD's own, not a sheet's,
/// and a sheet cannot define them again.
pub(crate) const BUILT_IN_LINETYPES: [&str; 3] = ["ByLayer", "ByBlock", CONTINUOUS];

/// The degree of a spline through fit points: AutoCAD always fits a cubic.
pub(crate) const FIT_DEGREE: u8 = 3;

/// The colour a layer has unless it states one: 7, white on a dark
/// background and black on paper.
pub const WHITE: u8 = 7;

fn layer_zero() -> String {
    LAYER_ZERO.to_owned()
}

/// A point in the sheet's declared unit. Every coordinate in the core is 3D.
///
/// It is written as `[x, y, z]` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "[f64; 3]", into = "[f64; 3]")]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl From<[f64; 3]> for Point3 {
    fn from([x, y, z]: [f64; 3]) -> Self {
        Self { x, y, z }
    }
}

impl From<Point3> for [f64; 3] {
    fn from(p: Point3) -> Self {
        [p.x, p.y, p.z]
    }
}

/// A straight line between two points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub from: Point3,
    pub to: Point3,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

/// Straight segments through two or more points, joined end to end.
///
/// When every point has the same `z` it is written as AutoCAD's lightweight
/// polyline at that elevation; otherwise as a 3D polyline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub points: Vec<Point3>,
    /// Whether a last segment joins the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

/// Part of a circle, drawn anticlockwise from `start_angle` to `end_angle`.
/// It lies in the plane parallel to XY at its centre's `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arc {
    pub center: Point3,
    pub radius: f64,
    /// Degrees anticlockwise from the X axis, in `[0, 360)`.
    pub start_angle: f64,
    /// Degrees anticlockwise from the X axis, in `[0, 360)`.
    pub end_angle: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

/// A circle in the plane parallel to XY at its centre's `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    pub center: Point3,
    pub radius: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

/// An ellipse, or part of one, in the plane parallel to XY at its centre's `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ellipse {
    pub center: Point3,
    /// From the centre to one end of the major axis, as `[dx, dy, dz]`;
    /// `dz` is 0.
    pub major_axis: Point3,
    /// The minor axis over the major axis, more than 0 and at most 1.
    pub ratio: f64,
    /// Where the curve starts: degrees anticlockwise from the major axis.
    /// A whole ellipse runs from 0 to 360.
    pub start_angle: f64,
    /// Where the curve ends: degrees anticlockwise from the major axis.
    pub end_angle: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

impl Ellipse {
    /// Whether the ellipse is whole rather than an elliptical arc.
    pub fn is_whole(&self) -> bool {
        self.start_angle == 0.0 && self.end_angle == 360.0
    }
}

/// A smooth curve, stated either by points it passes through or by the
/// control points of a B-spline. Exactly one of the two lists holds points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spline {
    /// Points the curve passes through, in order. AutoCAD fits a cubic
    /// through them, as its SPLINE command does by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fit_points: Vec<Point3>,
    /// The control points of a clamped B-spline with uniform knots: the
    /// curve starts at the first and ends at the last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_points: Vec<Point3>,
    /// The curve's degree: 3 for a curve through fit points; 1 to 10, and
    /// less than the number of control points, for one by control points.
    pub degree: u8,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

/// A single thing on a sheet. Each sits on a layer and draws with its
/// layer's colour, linetype and lineweight.
///
/// It is written as `{"type": "line", ...}` in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Object {
    Line(Line),
    Polyline(Polyline),
    Arc(Arc),
    Circle(Circle),
    Ellipse(Ellipse),
    Spline(Spline),
    Hatch(Hatch),
}

impl Object {
    /// The object's type as a sheet description and JSON name it, such as `line`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Line(_) => "line",
            Object::Polyline(_) => "polyline",
            Object::Arc(_) => "arc",
            Object::Circle(_) => "circle",
            Object::Ellipse(_) => "ellipse",
            Object::Spline(_) => "spline",
            Object::Hatch(_) => "hatch",
        }
    }

    /// The name of the layer the object sits on.
    pub fn layer(&self) -> &str {
        match self {
            Object::Line(o) => &o.layer,
            Object::Polyline(o) => &o.layer,
            Object::Arc(o) => &o.layer,
            Object::Circle(o) => &o.layer,
            Object::Ellipse(o) => &o.layer,
            Object::Spline(o) => &o.layer,
            Object::Hatch(o) => &o.layer,
        }
    }
}

/// A named pattern of dashes, gaps and dots that a layer draws its objects with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Linetype {
    pub name: String,
    /// What AutoCAD's linetype manager shows beside the name.
    #[serde(default)]
    pub description: String,
    /// The pattern, repeated along each object, in the declared unit: a
    /// positive length is a dash, a negative one a gap, and 0 a dot.
    pub pattern: Vec<f64>,
}

/// A named group of objects that share a colour, a linetype and a lineweight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    /// The AutoCAD colour index, 1 to 255.
    pub color: u8,
    /// `Continuous` or the name of one of the sheet's linetypes.
    pub linetype: String,
    /// The pen width objects on the layer plot with; none for AutoCAD's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineweight: Option<Lineweight>,
}

impl Layer {
    /// A layer with AutoCAD's defaults: white, continuous, default lineweight.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: WHITE,
            linetype: CONTINUOUS.to_owned(),
            lineweight: None,
        }
    }
}

/// A pen width on paper: one of AutoCAD's standard lineweights.
///
/// It is written in millimetres in JSON, such as `0.35`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Lineweight {
    hundredths_of_a_millimetre: u8,
}

impl Lineweight {
    /// Every standard lineweight, thinnest first.
    pub const ALL: [Lineweight; 24] = {
        const HUNDREDTHS: [u8; 24] = [
            0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140,
            158, 200, 211,
        ];
        let mut all = [Lineweight {
            hundredths_of_a_millimetre: 0,
        }; 24];
        let mut i = 0;
        while i < HUNDREDTHS.len() {
            all[i].hundredths_of_a_millimetre = HUNDREDTHS[i];
            i += 1;
        }
        all
    };

    /// The standard lineweight of exactly `millimetres`, if there is one.
    pub fn from_millimetres(millimetres: f64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|w| w.millimetres() == millimetres)
    }

    /// The standard lineweight of `hundredths` of a millimetre, if there is one.
    pub fn from_hundredths(hundredths: i16) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|w| i16::from(w.hundredths_of_a_millimetre) == hundredths)
    }

    pub fn millimetres(self) -> f64 {
        f64::from(self.hundredths_of_a_millimetre) / 100.0
    }

    pub fn hundredths(self) -> i16 {
        i16::from(self.hundredths_of_a_millimetre)
    }

    /// Every standard lineweight in millimetres, joined for an error message.
    pub(crate) fn choices() -> String {
        Self::ALL.map(|w| w.to_string()).join(", ")
    }
}

impl fmt::Display for Lineweight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.millimetres())
    }
}

impl TryFrom<f64> for Lineweight {
    type Error = String;

    fn try_from(millimetres: f64) -> Result<Self, String> {
        Self::from_millimetres(millimetres).ok_or_else(|| {
            format!(
                "{millimetres} mm is not a standard lineweight; one of {}",
                Self::choices()
            )
        })
    }
}

impl From<Lineweight> for f64 {
    fn from(w: Lineweight) -> Self {
        w.millimetres()
    }
}

/// What a new sheet holds, read from its sheet description.
///
/// Every length is in `units`, the sheet's declared unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub units: Unit,
    /// The linetypes the sheet defines, besides `Continuous`, which every drawing has.
    #[serde(default)]
    pub linetypes: Vec<Linetype>,
    /// The layers the sheet defines. Layer `0` is in every drawing; listing
    /// it here sets its colour, linetype and lineweight.
    #[serde(default)]
    pub layers: Vec<Layer>,
    pub objects: Vec<Object>,
}

/// What the core read from a DWG.
///
/// Lengths are in the drawing's own units, unconverted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingContents {
    /// The DWG version the drawing is saved in, such as `AC1032` for 2018.
    pub dwg_version: String,
    /// The drawing's unit, or none when it is unitless or in a unit the core does not know.
    pub units: Option<Unit>,
    /// The drawing's linetypes the core has a model for, in table order,
    /// leaving out `ByLayer`, `ByBlock` and `Continuous`, which every drawing has.
    pub linetypes: Vec<Linetype>,
    /// How many linetypes the core has no model for yet, such as those that
    /// draw text or shapes.
    pub linetypes_not_read: usize,
    /// The drawing's layers the core has a model for, in table order.
    pub layers: Vec<Layer>,
    /// How many layers the core has no model for yet, such as those with a
    /// true colour rather than a colour index.
    pub layers_not_read: usize,
    /// The model-space objects the core has a model for, in drawing order.
    pub objects: Vec<Object>,
    /// How many model-space objects the core has no model for yet, including
    /// those that set their own colour, linetype or lineweight rather than
    /// taking their layer's.
    pub objects_not_read: usize,
}
