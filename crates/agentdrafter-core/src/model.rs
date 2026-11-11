//! The core's own model of a sheet.
//!
//! The model belongs to AgentDrafter, not to the DWG library: the library is
//! reached only through [`crate::dwg`]. Every coordinate is 3D, so that 3D
//! work later is not a rewrite.

use serde::{Deserialize, Serialize};

use crate::units::Unit;

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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub from: Point3,
    pub to: Point3,
}

/// A single thing on a sheet. Only lines exist so far.
///
/// It is written as `{"type": "line", ...}` in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Object {
    Line(Line),
}

impl Object {
    /// The object's type as a sheet description and JSON name it, such as `line`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Line(_) => "line",
        }
    }
}

/// What a new sheet holds, read from its sheet description.
///
/// Every coordinate is in `units`, the sheet's declared unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub units: Unit,
    pub objects: Vec<Object>,
}

/// What the core read from a DWG's model space.
///
/// Coordinates are in the drawing's own units, unconverted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingContents {
    /// The DWG version the drawing is saved in, such as `AC1032` for 2018.
    pub dwg_version: String,
    /// The drawing's unit, or none when it is unitless or in a unit the core does not know.
    pub units: Option<Unit>,
    /// The model-space objects the core has a model for, in drawing order.
    pub objects: Vec<Object>,
    /// How many model-space objects the core has no model for yet.
    pub objects_not_read: usize,
}
