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

use crate::text::{MText, Text, TextStyle};
use crate::units::Unit;

pub(crate) mod block;
pub(crate) mod dimension;
pub(crate) mod hatch;
pub(crate) mod layout;
pub(crate) mod leader;
pub(crate) mod leader_block;
pub(crate) mod table;
pub use block::{
    attribute_text, Attribute, AttributeDefinition, Block, BlockLibrary, Cell, Insert,
};
pub use dimension::{
    AngularUnit, DecimalSeparator, Dimension, DimensionKind, DimensionPart, DimensionStyle,
    FractionFormat, LaidDimension, Lettered, Measured, OrdinateAxis, TextPiece, TextPlacement,
    UnitFormat, DEGREE_SIGN, DIAMETER_SIGN, ISO_DIMENSION_STYLE, MEASURED,
    STANDARD_DIMENSION_STYLE,
};
pub use hatch::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, SOLID};
pub use layout::{Clip, Layout, Viewport};
pub use leader::{
    Arrowhead, LaidContent, LaidMLeader, Leader, LeaderAnnotation, LeaderAttachment, MLeader,
    MLeaderBlock, MLeaderContent, MLeaderStyle, MLeaderText, MTextBox, STANDARD_MLEADER_STYLE,
};
pub use leader_block::{BlockConnection, Outline};
pub use table::{
    LaidCell, RowKind, RowStyle, Table, TableCell, TableStyle, LINE_BOX, LINE_STEP,
    STANDARD_TABLE_STYLE,
};

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

pub(crate) fn layer_zero() -> String {
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
    #[serde(flatten)]
    pub properties: Properties,
}

/// Segments through two or more points, joined end to end: straight, or
/// arcs where a point's bulge says so, and as wide as their widths.
///
/// When every point has the same `z` it is written as AutoCAD's lightweight
/// polyline at that elevation; otherwise as a 3D polyline, which has no
/// arcs and no widths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    pub points: Vec<Point3>,
    /// For each point, the bulge of the segment that starts there, as
    /// AutoCAD keeps it: the tangent of a quarter of the arc's included
    /// angle, positive anticlockwise, 0 for a straight segment. Empty when
    /// every segment is straight; otherwise one for each point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
    /// For each point, the width the segment that starts there has at its
    /// start and at its end, in the declared unit. Empty when no point
    /// gives one; otherwise one for each point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<[f64; 2]>,
    /// The width of every segment, when no point gives its own: AutoCAD's
    /// constant width. 0 draws a line.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f64,
    /// Whether a last segment joins the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
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
    #[serde(flatten)]
    pub properties: Properties,
}

/// A circle in the plane parallel to XY at its centre's `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    pub center: Point3,
    pub radius: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
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
    #[serde(flatten)]
    pub properties: Properties,
}

impl Polyline {
    /// A polyline of straight, thin segments through `points`.
    pub fn through(points: Vec<Point3>, closed: bool, layer: impl Into<String>) -> Self {
        Self {
            points,
            bulges: Vec::new(),
            widths: Vec::new(),
            width: 0.0,
            closed,
            layer: layer.into(),
            properties: Properties::default(),
        }
    }

    /// The bulge of the segment that starts at point `i`.
    pub fn bulge(&self, i: usize) -> f64 {
        self.bulges.get(i).copied().unwrap_or(0.0)
    }

    /// The widths at the start and the end of the segment that starts at
    /// point `i`.
    pub fn widths_at(&self, i: usize) -> [f64; 2] {
        if self.widths.is_empty() {
            [self.width, self.width]
        } else {
            self.widths.get(i).copied().unwrap_or([0.0, 0.0])
        }
    }

    /// Whether every point has the same `z`: AutoCAD's lightweight polyline.
    pub fn is_flat(&self) -> bool {
        let level = self.points.first().map_or(0.0, |p| p.z);
        self.points.iter().all(|p| p.z == level)
    }
}

/// A ring, or a filled dot when its inside diameter is 0, as AutoCAD's
/// DONUT draws it: a closed polyline of two half circles, as wide as the
/// ring. It lies in the plane parallel to XY at its centre's `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Donut {
    pub center: Point3,
    /// At least 0 and less than the outside diameter.
    pub inside_diameter: f64,
    /// More than 0.
    pub outside_diameter: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl Donut {
    /// The polyline AutoCAD's DONUT writes: from the point on the ring's
    /// middle circle at 0 degrees, a half circle anticlockwise to the point
    /// at 180 and another back, as wide as the ring.
    pub fn polyline(&self) -> Polyline {
        let radius = (self.inside_diameter + self.outside_diameter) / 4.0;
        let c = self.center;
        Polyline {
            points: vec![
                Point3::new(c.x + radius, c.y, c.z),
                Point3::new(c.x - radius, c.y, c.z),
            ],
            bulges: vec![1.0, 1.0],
            widths: Vec::new(),
            width: (self.outside_diameter - self.inside_diameter) / 2.0,
            closed: true,
            layer: self.layer.clone(),
            properties: self.properties.clone(),
        }
    }
}

/// A filled triangle or quadrilateral, AutoCAD's 2D solid. Its corners are
/// listed round its outline, and it lies in the plane parallel to XY at
/// their `z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    /// 3 or 4 corners, in order round the outline, sharing one `z`.
    pub corners: Vec<Point3>,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
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
    #[serde(flatten)]
    pub properties: Properties,
}

/// A single thing on a sheet. Each sits on a layer and draws with its
/// layer's colour, linetype and lineweight unless its [`Properties`] give
/// its own.
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
    Donut(Donut),
    Solid(Solid),
    Text(Text),
    #[serde(rename = "mtext")]
    MText(MText),
    /// An attribute definition, which belongs in a block.
    #[serde(rename = "attdef")]
    AttributeDefinition(AttributeDefinition),
    /// An insert of a block, which belongs on a sheet.
    Insert(Insert),
    /// A table, which belongs on a sheet.
    Table(Table),
    /// A multileader, which belongs on a sheet.
    #[serde(rename = "mleader")]
    MLeader(MLeader),
    /// An older leader, which belongs on a sheet.
    Leader(Leader),
    /// A dimension, which belongs on a sheet.
    Dimension(Dimension),
    /// A viewport, which belongs in a sheet's paper space.
    Viewport(Viewport),
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
            Object::Donut(_) => "donut",
            Object::Solid(_) => "solid",
            Object::Text(_) => "text",
            Object::MText(_) => "mtext",
            Object::AttributeDefinition(_) => "attdef",
            Object::Insert(_) => "insert",
            Object::Table(_) => "table",
            Object::MLeader(_) => "mleader",
            Object::Leader(_) => "leader",
            Object::Dimension(_) => "dimension",
            Object::Viewport(_) => "viewport",
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
            Object::Donut(o) => &o.layer,
            Object::Solid(o) => &o.layer,
            Object::Text(o) => &o.layer,
            Object::MText(o) => &o.layer,
            Object::AttributeDefinition(o) => &o.layer,
            Object::Insert(o) => &o.layer,
            Object::Table(o) => &o.layer,
            Object::MLeader(o) => &o.layer,
            Object::Leader(o) => &o.layer,
            Object::Dimension(o) => &o.layer,
            Object::Viewport(o) => &o.layer,
        }
    }

    /// The object's own colour, linetype, lineweight and linetype scale.
    pub fn properties(&self) -> &Properties {
        match self {
            Object::Line(o) => &o.properties,
            Object::Polyline(o) => &o.properties,
            Object::Arc(o) => &o.properties,
            Object::Circle(o) => &o.properties,
            Object::Ellipse(o) => &o.properties,
            Object::Spline(o) => &o.properties,
            Object::Hatch(o) => &o.properties,
            Object::Donut(o) => &o.properties,
            Object::Solid(o) => &o.properties,
            Object::Text(o) => &o.properties,
            Object::MText(o) => &o.properties,
            Object::AttributeDefinition(o) => &o.properties,
            Object::Insert(o) => &o.properties,
            Object::Table(o) => &o.properties,
            Object::MLeader(o) => &o.properties,
            Object::Leader(o) => &o.properties,
            Object::Dimension(o) => &o.properties,
            Object::Viewport(o) => &o.properties,
        }
    }

    pub(crate) fn properties_mut(&mut self) -> &mut Properties {
        match self {
            Object::Line(o) => &mut o.properties,
            Object::Polyline(o) => &mut o.properties,
            Object::Arc(o) => &mut o.properties,
            Object::Circle(o) => &mut o.properties,
            Object::Ellipse(o) => &mut o.properties,
            Object::Spline(o) => &mut o.properties,
            Object::Hatch(o) => &mut o.properties,
            Object::Donut(o) => &mut o.properties,
            Object::Solid(o) => &mut o.properties,
            Object::Text(o) => &mut o.properties,
            Object::MText(o) => &mut o.properties,
            Object::AttributeDefinition(o) => &mut o.properties,
            Object::Insert(o) => &mut o.properties,
            Object::Table(o) => &mut o.properties,
            Object::MLeader(o) => &mut o.properties,
            Object::Leader(o) => &mut o.properties,
            Object::Dimension(o) => &mut o.properties,
            Object::Viewport(o) => &mut o.properties,
        }
    }
}

/// An object's own colour, linetype, lineweight and linetype scale, as
/// AutoCAD's PROPERTIES sets them. Each is `ByLayer` unless the object
/// gives its own, and then it draws with its layer's.
///
/// Its keys sit beside the object's others in JSON, and are left out where
/// they are `ByLayer`, or 1 for the linetype scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Properties {
    #[serde(default, skip_serializing_if = "ObjectColor::is_by_layer")]
    pub color: ObjectColor,
    #[serde(default, skip_serializing_if = "ObjectLinetype::is_by_layer")]
    pub linetype: ObjectLinetype,
    #[serde(default, skip_serializing_if = "ObjectLineweight::is_by_layer")]
    pub lineweight: ObjectLineweight,
    /// What the object's linetype is enlarged by, more than 0, on top of
    /// the sheet's [`Sheet::ltscale`]: AutoCAD's CELTSCALE.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub linetype_scale: f64,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            color: ObjectColor::ByLayer,
            linetype: ObjectLinetype::ByLayer,
            lineweight: ObjectLineweight::ByLayer,
            linetype_scale: 1.0,
        }
    }
}

impl Properties {
    /// Whether the object takes everything from its layer.
    pub fn is_by_layer(&self) -> bool {
        *self == Self::default()
    }
}

pub(crate) fn one() -> f64 {
    1.0
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)]
pub(crate) fn is_one(n: &f64) -> bool {
    *n == 1.0
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)]
pub(crate) fn is_zero(n: &f64) -> bool {
    *n == 0.0
}

pub(crate) fn yes() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_true(b: &bool) -> bool {
    *b
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// An object property's value that takes its layer's: AutoCAD's ByLayer.
pub const BY_LAYER: &str = "ByLayer";

/// An object property's value that takes its block insert's: AutoCAD's ByBlock.
pub const BY_BLOCK: &str = "ByBlock";

/// The lineweight that is AutoCAD's default lineweight, LWDEFAULT.
pub const DEFAULT_LINEWEIGHT: &str = "Default";

/// An object's own colour: its layer's, its block insert's, or an AutoCAD
/// colour index from 1 to 255.
///
/// It is written as `"ByLayer"`, `"ByBlock"` or the index in JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ColorJson", into = "ColorJson")]
pub enum ObjectColor {
    #[default]
    ByLayer,
    ByBlock,
    Index(u8),
}

impl ObjectColor {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_by_layer(&self) -> bool {
        *self == Self::ByLayer
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ColorJson {
    Index(u8),
    Word(String),
}

impl TryFrom<ColorJson> for ObjectColor {
    type Error = String;

    fn try_from(json: ColorJson) -> Result<Self, String> {
        match json {
            ColorJson::Index(0) => Err("a colour index is 1 to 255".to_owned()),
            ColorJson::Index(index) => Ok(Self::Index(index)),
            ColorJson::Word(word) => match word.as_str() {
                BY_LAYER => Ok(Self::ByLayer),
                BY_BLOCK => Ok(Self::ByBlock),
                _ => Err(format!(
                    "`{word}` is not a colour; ByLayer, ByBlock or an index from 1 to 255"
                )),
            },
        }
    }
}

impl From<ObjectColor> for ColorJson {
    fn from(color: ObjectColor) -> Self {
        match color {
            ObjectColor::ByLayer => Self::Word(BY_LAYER.to_owned()),
            ObjectColor::ByBlock => Self::Word(BY_BLOCK.to_owned()),
            ObjectColor::Index(index) => Self::Index(index),
        }
    }
}

/// An object's own linetype: its layer's, its block insert's, or one by
/// name, `Continuous` or one the sheet defines.
///
/// It is written as `"ByLayer"`, `"ByBlock"` or the name in JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ObjectLinetype {
    #[default]
    ByLayer,
    ByBlock,
    Named(String),
}

impl ObjectLinetype {
    pub fn is_by_layer(&self) -> bool {
        *self == Self::ByLayer
    }
}

impl From<String> for ObjectLinetype {
    fn from(name: String) -> Self {
        match name.as_str() {
            BY_LAYER => Self::ByLayer,
            BY_BLOCK => Self::ByBlock,
            _ => Self::Named(name),
        }
    }
}

impl From<ObjectLinetype> for String {
    fn from(linetype: ObjectLinetype) -> Self {
        match linetype {
            ObjectLinetype::ByLayer => BY_LAYER.to_owned(),
            ObjectLinetype::ByBlock => BY_BLOCK.to_owned(),
            ObjectLinetype::Named(name) => name,
        }
    }
}

/// An object's own lineweight: its layer's, its block insert's, AutoCAD's
/// default lineweight, or a standard lineweight.
///
/// It is written as `"ByLayer"`, `"ByBlock"`, `"Default"` or millimetres in
/// JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LineweightJson", into = "LineweightJson")]
pub enum ObjectLineweight {
    #[default]
    ByLayer,
    ByBlock,
    Default,
    Width(Lineweight),
}

impl ObjectLineweight {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_by_layer(&self) -> bool {
        *self == Self::ByLayer
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum LineweightJson {
    Width(Lineweight),
    Word(String),
}

impl TryFrom<LineweightJson> for ObjectLineweight {
    type Error = String;

    fn try_from(json: LineweightJson) -> Result<Self, String> {
        match json {
            LineweightJson::Width(width) => Ok(Self::Width(width)),
            LineweightJson::Word(word) => match word.as_str() {
                BY_LAYER => Ok(Self::ByLayer),
                BY_BLOCK => Ok(Self::ByBlock),
                DEFAULT_LINEWEIGHT => Ok(Self::Default),
                _ => Err(format!(
                    "`{word}` is not a lineweight; ByLayer, ByBlock, Default or a standard lineweight in millimetres"
                )),
            },
        }
    }
}

impl From<ObjectLineweight> for LineweightJson {
    fn from(lineweight: ObjectLineweight) -> Self {
        match lineweight {
            ObjectLineweight::ByLayer => Self::Word(BY_LAYER.to_owned()),
            ObjectLineweight::ByBlock => Self::Word(BY_BLOCK.to_owned()),
            ObjectLineweight::Default => Self::Word(DEFAULT_LINEWEIGHT.to_owned()),
            ObjectLineweight::Width(width) => Self::Width(width),
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
    /// Whether the layer is on. An object on a layer that is off draws
    /// nothing, and still counts towards the extents, as in AutoCAD.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub on: bool,
    /// Whether the layer is frozen. An object on a frozen layer draws
    /// nothing and does not count towards the extents, as in AutoCAD.
    #[serde(default, skip_serializing_if = "is_false")]
    pub frozen: bool,
    /// Whether the layer is locked, as AutoCAD locks it against editing.
    /// It changes nothing a sheet draws.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Whether the layer plots. An object on a layer that does not plot is
    /// left out of a PDF, and drawn in a PNG and the viewer, as AutoCAD's
    /// screen shows it; it counts towards the extents.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub plot: bool,
    /// What AutoCAD's layer manager shows beside the name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Layer {
    /// A layer with AutoCAD's defaults: white, continuous, default lineweight.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: WHITE,
            linetype: CONTINUOUS.to_owned(),
            lineweight: None,
            on: true,
            frozen: false,
            locked: false,
            plot: true,
            description: String::new(),
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
    /// The block library the sheet draws on, as its sheet description
    /// writes its path: relative to the sheet description's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_library: Option<String>,
    /// The linetypes the sheet defines, besides `Continuous`, which every drawing has.
    #[serde(default)]
    pub linetypes: Vec<Linetype>,
    /// The layers the sheet defines. Layer `0` is in every drawing; listing
    /// it here sets its colour, linetype and lineweight.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The text styles the sheet defines. Style `Standard` is in every
    /// drawing; listing it here sets its font and width factor.
    #[serde(default)]
    pub text_styles: Vec<TextStyle>,
    /// The table styles the sheet defines. Style `Standard` is in every
    /// drawing; listing it here sets what it draws with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_styles: Vec<TableStyle>,
    /// The multileader styles the sheet defines. Style `Standard` is in
    /// every drawing; listing it here sets what it draws with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mleader_styles: Vec<MLeaderStyle>,
    /// The dimension styles the sheet defines. Its unit's own, `ISO-25` on
    /// a metric sheet and `Standard` on an imperial one, it has without
    /// listing it; listing it here sets what it draws with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimension_styles: Vec<DimensionStyle>,
    /// The blocks the sheet's inserts place, as the block library defines
    /// them. The layers, linetypes and text styles their objects use are
    /// among the sheet's.
    #[serde(default)]
    pub blocks: Vec<Block>,
    pub objects: Vec<Object>,
    /// The paper the sheet is drawn on, if its sheet description or
    /// template declares it. Only the checker reads it: a build, a render
    /// and a plot draw the sheet as they would without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// What every linetype on the sheet is enlarged by, more than 0:
    /// AutoCAD's LTSCALE. An object's own linetype scale multiplies it.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub ltscale: f64,
    /// The paper-space layout the sheet is drawn in, if it is drawn in
    /// one: its border and title block in paper space, at the paper's size,
    /// and its viewports onto model space. Without one, the sheet is drawn
    /// in model space alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// The sheet's other layouts, when it is drawn in more than one, in the
    /// order of their tabs after `layout`'s. They share its model space;
    /// each has its own paper, paper-space objects and viewports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_layouts: Vec<Layout>,
}

impl Sheet {
    /// Whether the sheet's object at `index` is in the paper space of the
    /// layout it draws, `layout`: never on a sheet without a layout.
    pub fn in_paper_space(&self, index: usize) -> bool {
        self.layout.as_ref().is_some_and(|l| l.holds(index))
    }

    /// Whether the sheet's object at `index` is in model space: in no
    /// layout's paper space.
    pub fn in_model_space(&self, index: usize) -> bool {
        self.layout_of(index).is_none()
    }

    /// The layout whose paper space holds the sheet's object at `index`;
    /// none for an object of model space.
    pub fn layout_of(&self, index: usize) -> Option<&Layout> {
        self.layouts().find(|l| l.holds(index))
    }

    /// Every layout the sheet is drawn in, in the order of their tabs.
    pub fn layouts(&self) -> impl Iterator<Item = &Layout> {
        self.layout.iter().chain(&self.other_layouts)
    }

    /// The sheet as each of its layouts prints it, in the order of their
    /// tabs: the sheet itself when it has one layout or none, and for each
    /// of several, the sheet with that layout first, as `layout`, and its
    /// paper as the sheet's. Each keeps every object in its place, so an
    /// object's place counts the same in all of them.
    pub fn pages(&self) -> Vec<std::borrow::Cow<'_, Sheet>> {
        if self.other_layouts.is_empty() {
            return vec![std::borrow::Cow::Borrowed(self)];
        }
        let all: Vec<&Layout> = self.layouts().collect();
        (0..all.len())
            .map(|k| {
                if k == 0 {
                    return std::borrow::Cow::Borrowed(self);
                }
                let mut page = self.clone();
                page.layout = Some(all[k].clone());
                page.other_layouts = all
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != k)
                    .map(|(_, l)| (*l).clone())
                    .collect();
                page.paper.clone_from(&all[k].paper);
                std::borrow::Cow::Owned(page)
            })
            .collect()
    }

    /// The sheet's viewports in the layout it draws, `layout`, each with
    /// its place in the sheet's objects.
    pub fn viewports(&self) -> impl Iterator<Item = (usize, &Viewport)> {
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| match o {
                Object::Viewport(v) if self.in_paper_space(i) => Some((i, v)),
                _ => None,
            })
    }

    /// The table style named `name` a table draws with: one the sheet
    /// lists, or, for `Standard` when the sheet does not list it, the
    /// Standard table style of the sheet's unit. None for a name the sheet
    /// does not have.
    pub fn table_style(&self, name: &str) -> Option<TableStyle> {
        self.table_styles
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .or_else(|| (name == STANDARD_TABLE_STYLE).then(|| TableStyle::standard(self.units)))
    }

    /// The insert that draws the block content of the multileader
    /// `mleader` on this sheet, placed where AutoCAD places it; none for
    /// text content.
    pub fn block_content(&self, mleader: &MLeader) -> Option<Insert> {
        mleader.content.block()?;
        let style = self
            .mleader_style(&mleader.mleader_style)
            .unwrap_or_else(|| MLeaderStyle::standard(self.units));
        let laid = mleader.laid_out(&style, &|_| MTextBox::default(), &self.blocks);
        match laid.content {
            LaidContent::Block { insert, .. } => Some(insert),
            LaidContent::Text { .. } => None,
        }
    }

    /// The multileader style named `name` a multileader draws with: one the
    /// sheet lists, or, for `Standard` when the sheet does not list it, the
    /// Standard multileader style of the sheet's unit. None for a name the
    /// sheet does not have.
    pub fn mleader_style(&self, name: &str) -> Option<MLeaderStyle> {
        self.mleader_styles
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .or_else(|| {
                (name == STANDARD_MLEADER_STYLE).then(|| MLeaderStyle::standard(self.units))
            })
    }

    /// The dimension style named `name` a dimension draws with: one the
    /// sheet lists, or its unit's own, `ISO-25` on a metric sheet and
    /// `Standard` on an imperial one, when the sheet does not list it.
    /// None for a name the sheet does not have.
    pub fn dimension_style(&self, name: &str) -> Option<DimensionStyle> {
        self.dimension_styles
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .or_else(|| DimensionStyle::built_in(name, self.units))
    }
}

/// The paper a sheet is drawn on, as its sheet description or template
/// declares it: its size in the declared unit, with its lower left corner
/// at the origin, `[0, 0]`, as a sheet drawn in model space at its paper
/// size draws its trim line from there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paper {
    /// The size's name, when it is declared by name, such as
    /// `ANSI B landscape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Across, along X.
    pub width: f64,
    /// Up, along Y.
    pub height: f64,
}

/// The ID and the name an object of a DWG carries in AgentDrafter's
/// extended data. An object AgentDrafter did not build carries neither;
/// one it built always carries its ID, such as `DEMO-001/border`, and its
/// name if it has one. An insert's name is its [`Insert`]'s name as well.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAndName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// The drawing's text styles the core has a model for, in table order,
    /// `Standard` among them, leaving out the entries that load shapes for
    /// linetypes rather than fonts for text.
    pub text_styles: Vec<TextStyle>,
    /// How many text styles the core has no model for yet, such as those
    /// that slant, mirror or fix the height of their text, or add a big font.
    pub text_styles_not_read: usize,
    /// The drawing's table styles the core has a model for, `Standard`
    /// among them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_styles: Vec<TableStyle>,
    /// How many table styles the core has no model for yet, such as those
    /// whose rows draw with different margins or borders, or fill their
    /// cells.
    #[serde(default)]
    pub table_styles_not_read: usize,
    /// The drawing's multileader styles the core has a model for,
    /// `Standard` among them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mleader_styles: Vec<MLeaderStyle>,
    /// How many multileader styles the core has no model for yet, such as
    /// those whose content is a block, whose leaders are splines, or that
    /// underline or frame their text.
    #[serde(default)]
    pub mleader_styles_not_read: usize,
    /// The drawing's dimension styles the core has a model for, in table
    /// order. Its own `Standard`, which every drawing has, is among them
    /// when the core can read it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimension_styles: Vec<DimensionStyle>,
    /// How many dimension styles the core has no model for yet, such as
    /// those that draw tolerances, alternate units or architectural ticks,
    /// and a style a dimension names that the drawing lacks. The drawing's
    /// own `Standard` that no dimension draws with is not counted.
    #[serde(default)]
    pub dimension_styles_not_read: usize,
    /// The drawing's block definitions the core has a model for, in table
    /// order, leaving out model space and the layouts.
    pub blocks: Vec<Block>,
    /// For each block in `blocks`, in the same order, the handle of each of
    /// its `objects`, in hexadecimal as AutoCAD lists it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_handles: Vec<Vec<String>>,
    /// For each block in `blocks`, in the same order, the ID and name each
    /// of its `objects` carries in AgentDrafter's extended data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_ids_and_names: Vec<Vec<IdAndName>>,
    /// How many block definitions the core has no model for yet, such as
    /// anonymous blocks, external references and blocks holding an object
    /// the core has no model for. A table's own anonymous block, which
    /// holds what it draws, is read with the table and not counted, and so
    /// is the block of a leader's arrowhead.
    pub blocks_not_read: usize,
    /// The model-space objects the core has a model for, in drawing order.
    pub objects: Vec<Object>,
    /// The ID and name each of `objects` carries in AgentDrafter's extended
    /// data, in the same order.
    #[serde(default)]
    pub ids_and_names: Vec<IdAndName>,
    /// How many model-space objects the core has no model for yet, such as
    /// those with a true colour.
    pub objects_not_read: usize,
    /// What every linetype in the drawing is enlarged by: its LTSCALE.
    #[serde(default = "one")]
    pub ltscale: f64,
    /// The drawing's first layout the core reads, in the order of their
    /// tabs, whatever tab the drawing was saved on: `objects` then holds
    /// its paper space's objects and viewports beside model space's, and
    /// the layout says which are which. None for a drawing of model space
    /// alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// The drawing's other layouts the core reads, after `layout`, in the
    /// order of their tabs: each with its paper, when the core can read
    /// its page setup, and its paper space's objects among `objects`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_layouts: Vec<Layout>,
    /// The layout the drawing opens on, by its name, as AutoCAD saved it
    /// current; none when it opens on the Model tab. It says which tab
    /// AutoCAD shows first, and changes nothing the core reads or draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_on: Option<String>,
    /// The paper the layout's page setup plots, when the core can read it:
    /// a standard paper, whole, at full size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// How many of the drawing's layouts the core reads no page setup of:
    /// the one the drawing opens on, and any other that holds anything,
    /// whose page setup is not a standard paper plotted whole at full size.
    /// Each is read, without a paper. AutoCAD's own empty layout is not
    /// counted.
    #[serde(default, skip_serializing_if = "is_none_of")]
    pub layouts_not_read: usize,
    /// The handle of each of `objects`, in the same order, in hexadecimal
    /// as AutoCAD lists it, such as `2F`. A handle is the DWG's own name for
    /// a record, and stays the same each time the same file is read.
    #[serde(default)]
    pub handles: Vec<String>,
    /// The objects of the sheet, model space's and each read layout's
    /// paper space's, the core has no model for, by kind, with the handle
    /// of each: what an output of the drawing does not draw. Their count is
    /// `objects_not_read`.
    #[serde(default)]
    pub not_read: Vec<RecordsOfKind>,
    /// Whether the DWG library decoded every record of the file, and the
    /// records it did not (ADR-0004).
    #[serde(default)]
    pub decode: DecodeReport,
}

/// Records of a DWG of one kind, counted, with the handle of each.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordsOfKind {
    /// The kind of record, as the DWG names it, such as `LINE`,
    /// `ACAD_PROXY_ENTITY` or a custom object's own name.
    pub kind: String,
    /// How many there are.
    pub count: usize,
    /// The handle of each, in hexadecimal as AutoCAD lists it, such as
    /// `2F`, in the order the file holds them. A record the reader could
    /// not place has none, so there may be fewer handles than `count`.
    pub handles: Vec<String>,
}

/// Whether the DWG library decoded every record of a DWG it read: each
/// object, table entry and non-graphical object into a model of its own.
/// A record it did not decode it keeps as the bytes it read, which a save
/// in the version the file arrived in writes back as they were (ADR-0004,
/// ADR-0006), but nothing can draw, query or change it; a damaged record
/// skipped while reading past damage is left out altogether.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeReport {
    /// True when every record was decoded whole: none is under
    /// `not_decoded`, none was skipped, the reader recovered from no error,
    /// the record stream finished, and diagnostics were not cut off at the
    /// library's limit.
    pub decoded: bool,
    /// The records the library could not decode whole, by kind: opaque,
    /// proxy and custom records kept as raw bytes, records cut short such
    /// as a mesh over the face budget, and damaged records skipped while
    /// reading past damage.
    pub not_decoded: Vec<RecordsOfKind>,
    /// How many records the library could not read at all and left out.
    #[serde(default)]
    pub skipped: usize,
    /// How many errors the library recovered from while reading.
    #[serde(default)]
    pub recovered_errors: usize,
    /// How many non-fatal reader warnings the library reported while reading
    /// (such as inconsistent source references or unknown enum values),
    /// excluding records listed as skipped or errors.
    #[serde(default, skip_serializing_if = "is_none_of")]
    pub warnings: usize,
    /// True when the DWG library hit its internal diagnostic cap (999
    /// entries) or notification limit and omitted further diagnostics, so
    /// `not_decoded` or `warnings` may be incomplete.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diagnostics_truncated: bool,
    /// Why the library could not read the file whole, when it could not:
    /// the read then went on past the records it could not read, as
    /// AutoCAD's RECOVER reads a damaged drawing, though repairing nothing,
    /// and named each it passed under `not_decoded`. Only `read` reads past
    /// damage; every other command refuses such a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_past_damage: Option<String>,
}

impl Default for DecodeReport {
    fn default() -> Self {
        Self {
            decoded: true,
            not_decoded: Vec::new(),
            skipped: 0,
            recovered_errors: 0,
            warnings: 0,
            diagnostics_truncated: false,
            read_past_damage: None,
        }
    }
}

impl RecordsOfKind {
    /// Groups `(kind, handle)` pairs by kind, in the order each kind is
    /// first met, each with its handles in the order met.
    pub(crate) fn group<I: IntoIterator<Item = (String, Option<String>)>>(records: I) -> Vec<Self> {
        let mut kinds: Vec<RecordsOfKind> = Vec::new();
        for (kind, handle) in records {
            let at = match kinds.iter().position(|k| k.kind == kind) {
                Some(at) => at,
                None => {
                    kinds.push(RecordsOfKind {
                        kind,
                        count: 0,
                        handles: Vec::new(),
                    });
                    kinds.len() - 1
                }
            };
            kinds[at].count += 1;
            kinds[at].handles.extend(handle);
        }
        kinds
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_none_of(n: &usize) -> bool {
    *n == 0
}
