//! AgentDrafter's core: the library that knows drawings.
//!
//! Every interface is a thin layer over the entry points here: the command
//! line now, and the MCP server, the Python library and the HTTP API later.
//! The entry points take and return plain data, so an operation behaves the
//! same through each interface and none of them holds drawing logic
//! (ADR-0007).
//!
//! - [`build`] turns a sheet description into a DWG in the 2018 format.
//! - [`parse_sheet_description`] reads a sheet description into a [`Sheet`].
//! - [`sheet_description_schema`] states what a sheet description may hold,
//!   as a JSON Schema.
//! - [`read_dwg`] reads a DWG's linetypes, layers, text styles and
//!   model-space objects.
//! - [`measure_text`] measures how wide a line of text draws in a font.
//! - [`render`] draws a sheet description or a DWG to a PNG, stamped with
//!   the record it came from, and [`check_stale`] says whether such an
//!   output's record has moved on.
//!
//! [`Settings`] say where the machine keeps its fonts; [`build`],
//! [`measure_text`] and [`render`] read them from the environment, and
//! [`build_with`], [`measure_text_with`] and [`render_with`] take them as
//! given.
//!
//! DWG is read and written through acadrust, behind one seam in the `dwg`
//! module. Every coordinate in the core is 3D. A sheet is drawn by one
//! pipeline, the `draw` module, whose display list every output draws
//! from (ADR-0011).

mod build;
mod description;
mod draw;
mod dwg;
mod error;
mod model;
mod raster;
mod render;
mod reply;
mod settings;
mod stamp;
mod text;
mod units;

use std::path::Path;

pub use build::{build, build_with, BuildReport, BuildRequest};
pub use description::parse_sheet_description;
pub use error::{Error, ErrorCode};
pub use model::{
    Arc, Circle, DrawingContents, Ellipse, Layer, Line, Linetype, Lineweight, Object, Point3,
    Polyline, Sheet, Spline, CONTINUOUS, LAYER_ZERO,
};
pub use model::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, SOLID};
pub use render::{render, render_with, RenderReport, RenderRequest};
pub use reply::Reply;
pub use settings::{Settings, FONTS_VARIABLE};
pub use stamp::{check_stale, SourceKind, SourceStamp, StaleReport, StaleRequest, Stamp};
pub use text::fonts::{FontSubstitution, FontUsed};
pub use text::measure::{measure_text, measure_text_with, MeasureTextRequest, TextMeasurement};
pub use text::{Attachment, FontKind, Justify, MText, Text, TextStyle, STANDARD};
pub use units::Unit;

/// Reads the linetypes, the layers, the text styles and the model-space
/// objects of the DWG at `path`.
pub fn read_dwg(path: &Path) -> Result<DrawingContents, Error> {
    dwg::read(path)
}

/// The sheet description's schema: a JSON Schema (draft 2020-12) of every
/// key a sheet description may hold, with what each means.
pub fn sheet_description_schema() -> serde_json::Value {
    description::schema::sheet_description_schema()
}
