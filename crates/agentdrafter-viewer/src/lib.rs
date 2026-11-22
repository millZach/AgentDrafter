//! The viewer's renderer: a sheet's display list drawn in the page with
//! WebGL2, by Rust compiled to WebAssembly (ADR-0011).
//!
//! The engine lays the sheet out on the machine that holds the drawings and
//! sends its display list, the one the PNG and the PDF draw from (the
//! `display-list` command). This crate turns that list into pixels on the
//! screen, as `raster.rs` turns it into the PNG's:
//!
//! - [`mesh`] makes the list ready for the GPU, in plain Rust: every
//!   stroke's segments with its pen and its colour on paper, every fill cut
//!   into triangles by its rule.
//! - [`grid`] spaces the grid drawn behind the drawing, in plain Rust.
//! - [`snapping`] finds the object snaps in reach of the cursor, among the
//!   running ones, from those the display list gives; [`overlay`] lays out
//!   the crosshair and the snap markers drawn over the drawing.
//! - [`tools`] runs the drafter's drawing tools, Line, Circle, Rectangle,
//!   Text, Move, Copy, Array and Erase: their steps, prompts, rubber bands
//!   and the change set each gives, in plain Rust, with [`array`] for
//!   Array's values and where its copies go.
//! - [`props`] edits quick properties: the fields an object's quick
//!   properties edit, what each says of where its value is written, and the
//!   change set an edit gives, in plain Rust.
//! - [`preview`] lays out how a change set drawn before it lands is marked
//!   on the sheet as it would leave it: the rest faded, what it adds and
//!   changes highlighted, the old places faint, what it deletes struck, and
//!   a cloud round each check it would fail, in plain Rust.
//! - [`ghosts`] lays out the proposals waiting on the sheet as it is (#155):
//!   what each would add and change, dashed in the proposal's colour, what
//!   it would delete struck, and the box its card is put by, in plain Rust.
//! - `web` (built for `wasm32` only) is the module the page loads: it draws
//!   the mesh with WebGL2 at the view the page asks for, on dark model
//!   space in AutoCAD's screen colours, index 7 white, the paper a dashed
//!   outline; or, for a test, on white paper in the colours the PNG plots.
//!   Pens are widths on paper, as in the PNG, never thinner than one CSS
//!   pixel; ends and corners are round. Items paint in the display list's
//!   order, a later one over an earlier one, as the PNG paints them.
//!   Panning and zooming change two numbers the GPU is given, so the mesh
//!   is built once per sheet.
//!
//! It needs WebGL2 and nothing newer: no WebGPU.

pub mod array;
pub mod ghosts;
pub mod grid;
pub mod guide;
pub mod mesh;
pub mod overlay;
pub mod preview;
pub mod props;
pub mod snapping;
pub mod tools;

#[cfg(target_arch = "wasm32")]
mod web;
