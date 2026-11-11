//! AgentDrafter's core: the library that knows drawings.
//!
//! Every interface is a thin layer over the entry points here: the command
//! line now, and the MCP server, the Python library and the HTTP API later.
//! The entry points take and return plain data, so an operation behaves the
//! same through each interface and none of them holds drawing logic
//! (ADR-0007).
//!
//! - [`build`] turns a sheet description into a DWG in the 2018 format, and
//!   [`build_set`] builds every sheet of a drawing set, each a template
//!   plus one row of data.
//! - [`read_sheet_description`] reads a sheet description's file, with the
//!   block library it names, into a [`Sheet`]; [`parse_sheet_description`]
//!   reads the text of one that names no block library.
//! - [`parse_block_library`] reads a block library into a [`BlockLibrary`].
//! - [`sheet_description_schema`], [`block_library_schema`], [`template_schema`],
//!   [`set_schema`] and [`rows_schema`] state what a sheet description, a
//!   block library, a template and a drawing set's settings and rows may
//!   hold, as JSON Schemas.
//! - [`read_dwg`] reads a DWG's linetypes, layers, text styles, block
//!   definitions and model-space objects.
//! - [`measure_text`] measures how wide a line of text draws in a font.
//!
//! [`Settings`] say where the machine keeps its fonts; [`build`] and
//! [`measure_text`] read them from the environment, and [`build_with`] and
//! [`measure_text_with`] take them as given.
//!
//! DWG is read and written through acadrust, behind one seam in the `dwg`
//! module. Every coordinate in the core is 3D.

mod build;
mod description;
mod dwg;
mod error;
mod model;
mod reply;
mod set;
mod settings;
mod text;
mod units;

use std::path::Path;

pub use build::{build, build_with, BuildReport, BuildRequest};
pub use description::{parse_block_library, parse_sheet_description, read_sheet_description};
pub use error::{Error, ErrorCode};
pub use model::{attribute_text, Attribute, AttributeDefinition, Block, BlockLibrary, Insert};
pub use model::{
    Arc, Circle, DrawingContents, Ellipse, Layer, Line, Linetype, Lineweight, Object, Point3,
    Polyline, Sheet, Spline, CONTINUOUS, LAYER_ZERO,
};
pub use model::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, SOLID};
pub use reply::Reply;
pub use set::{
    build_set, build_set_with, SetBuildRequest, SetReport, SheetOutcome, SET_SETTINGS_FILE,
};
pub use settings::{Settings, FONTS_VARIABLE};
pub use text::fonts::{FontSubstitution, FontUsed};
pub use text::measure::{measure_text, measure_text_with, MeasureTextRequest, TextMeasurement};
pub use text::{Attachment, FontKind, Justify, MText, Text, TextStyle, STANDARD};
pub use units::Unit;

/// Reads the linetypes, the layers, the text styles, the block definitions
/// and the model-space objects of the DWG at `path`.
pub fn read_dwg(path: &Path) -> Result<DrawingContents, Error> {
    dwg::read(path)
}

/// The sheet description's schema: a JSON Schema (draft 2020-12) of every
/// key a sheet description may hold, with what each means.
pub fn sheet_description_schema() -> serde_json::Value {
    description::schema::sheet_description_schema()
}

/// The block library's schema: a JSON Schema (draft 2020-12) of every key a
/// block library may hold, with what each means.
pub fn block_library_schema() -> serde_json::Value {
    description::schema::block_library_schema()
}

/// The schema of a drawing set's settings, `set.yaml`: a JSON Schema
/// (draft 2020-12) of every key they may hold, with what each means.
pub fn set_schema() -> serde_json::Value {
    description::set_schema()
}

/// The schema of a drawing set's rows: a JSON Schema (draft 2020-12) of
/// every key the rows file may hold, with what each means.
pub fn rows_schema() -> serde_json::Value {
    description::rows_schema()
}

/// The template's schema: a JSON Schema (draft 2020-12) of every key a
/// template may hold, with what each means. It is the sheet description's,
/// with values a template takes from a row.
pub fn template_schema() -> serde_json::Value {
    description::template_schema()
}
