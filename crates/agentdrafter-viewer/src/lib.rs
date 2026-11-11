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
//! - `web` (built for `wasm32` only) is the module the page loads: it draws
//!   the mesh with WebGL2 at the view the page asks for, on white paper.
//!   Pens are widths on paper, as in the PNG, never thinner than one CSS
//!   pixel; ends and corners are round. Ink only darkens the paper: each
//!   pixel takes the darkest ink drawn on it, so strokes that meet do not
//!   darken where they overlap. Panning and zooming change two numbers the
//!   GPU is given, so the mesh is built once per sheet.
//!
//! It needs WebGL2 and nothing newer: no WebGPU.

pub mod mesh;

#[cfg(target_arch = "wasm32")]
mod web;
