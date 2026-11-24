//! Named (and unnamed) object groups (`GROUP` / `ACAD_GROUP`, #92).
//!
//! A group is a named set of objects on a sheet that operations can target as
//! one, by its name, and that AutoCAD selects as one without turning them into
//! a block. Unlike a block, grouped objects remain regular objects on the sheet
//! and an object may belong to more than one group.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// A group of objects on a sheet, matching AutoCAD's `GROUP` object in
/// `ACAD_GROUP`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Group name (case-insensitive unique per sheet, e.g. `FT-101`, or `*A1`
    /// for an unnamed AutoCAD group read from an existing DWG).
    pub name: String,
    /// Optional description (DXF code 300).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Whether AutoCAD selects the group as a unit (DXF code 71, default `true`).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub selectable: bool,
    /// Whether this is an unnamed AutoCAD group (`*A<n>`, DXF code 70 = 1).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unnamed: bool,
    /// 0-based indices into `Sheet::objects` (or `DrawingContents::objects`),
    /// in group order, without duplicates.
    #[serde(default)]
    pub objects: Vec<usize>,
}

impl Group {
    /// Creates a new named, selectable group with no description.
    pub fn new(name: impl Into<String>, objects: Vec<usize>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            selectable: true,
            unnamed: false,
            objects,
        }
    }
}
