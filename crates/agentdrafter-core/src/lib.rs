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
//! - [`read_dwg`] reads the objects in a DWG's model space.
//!
//! DWG is read and written through acadrust, behind one seam in the `dwg`
//! module. Every coordinate in the core is 3D.

mod build;
mod description;
mod dwg;
mod error;
mod model;
mod reply;
mod units;

use std::path::Path;

pub use build::{build, BuildReport, BuildRequest};
pub use description::parse_sheet_description;
pub use error::{Error, ErrorCode};
pub use model::{DrawingContents, Line, Object, Point3, Sheet};
pub use reply::Reply;
pub use units::Unit;

/// Reads the objects in the model space of the DWG at `path`.
pub fn read_dwg(path: &Path) -> Result<DrawingContents, Error> {
    dwg::read(path)
}
