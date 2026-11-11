//! The drawing pipeline: a sheet becomes drawn geometry, a display list,
//! which the PNG draws from now and the PDF and the viewer draw from later
//! (ADR-0011). Text, linetypes and hatches are laid out here once, for
//! every output.
//!
//! What is drawn follows AutoCAD's plot of the same DWG, seen in AutoCAD
//! 2026's plots of made-up sheets:
//!
//! - Each object draws with its layer's colour, linetype and lineweight. A
//!   layer with no lineweight of its own draws with AutoCAD's default,
//!   0.25 mm. A lineweight is a width on paper, so the display list keeps
//!   it in millimetres and each output turns it into its own units.
//! - A linetype repeats along each line, arc, circle, ellipse and spline,
//!   and along each segment of a flat polyline, laid out so that both ends
//!   draw alike ([`linetype`]). A 3D polyline draws continuous, as AutoCAD
//!   draws it.
//! - Text draws in its font: an SHX font's strokes with the layer's pen,
//!   and a TrueType font's outlines filled ([`text`]).
//! - A hatch fills its area solid, or draws its pattern's lines inside its
//!   boundary ([`hatch`]).
//! - The sheet is seen from above: `z` is dropped.
//!
//! Everything here is plain computation. It opens no window and uses no
//! GPU, file system, thread or clock, so it builds for
//! `wasm32-unknown-unknown` and the viewer can run it in the page. A test
//! keeps it that way.

pub(crate) mod color;
pub(crate) mod curves;
mod hatch;
mod linetype;
mod text;

use serde::Serialize;

use crate::model::{Layer, Object, Sheet, CONTINUOUS, LAYER_ZERO};
use crate::text::placement::SheetFonts;

pub use color::Color;

/// AutoCAD's default lineweight, which a layer with none of its own plots
/// with: 0.25 mm, AutoCAD's `LWDEFAULT` as installed.
pub const DEFAULT_LINEWEIGHT: f64 = 0.25;

/// A point in the drawing's own units, seen from above: `[x, y]`.
pub type Point2 = [f64; 2];

/// What a sheet draws: shapes in drawing order, each with the colour and
/// pen it draws with. Coordinates are in the drawing's units; lineweights
/// are millimetres on paper.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DisplayList {
    pub items: Vec<Item>,
    /// The box round what counts towards the extents but draws nothing of
    /// its own: the boundaries of patterned hatches, whose lines may stop
    /// short of them. AutoCAD's extents take in a hatch's boundary.
    pub reach: Option<Bounds>,
}

/// One shape of one object.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item {
    /// Which object of the sheet draws it: its place in the sheet's
    /// objects, counted from 0.
    pub object: usize,
    /// The AutoCAD colour index it draws in.
    pub color: Color,
    pub shape: Shape,
}

/// A shape: lines drawn with a pen, or an area filled.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// Lines drawn with a pen `lineweight` millimetres wide on paper, with
    /// round ends and corners. Each run is a polyline; a run of one point
    /// is a dot as wide as the pen.
    Stroke {
        lineweight: f64,
        runs: Vec<Vec<Point2>>,
    },
    /// An area filled solid. Each contour is closed, its last point joining
    /// its first.
    Fill {
        rule: FillRule,
        contours: Vec<Vec<Point2>>,
    },
}

/// Which points of a fill's contours are inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    /// Inside an odd number of contours: a hatch's islands.
    EvenOdd,
    /// Where the contours wind round a point other than zero times: a
    /// TrueType outline.
    NonZero,
}

/// The box around drawn geometry, in drawing units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bounds {
    pub min: Point2,
    pub max: Point2,
}

impl Bounds {
    fn of(points: impl Iterator<Item = Point2>) -> Option<Bounds> {
        points.fold(None, |bounds: Option<Bounds>, p| {
            Some(match bounds {
                None => Bounds { min: p, max: p },
                Some(b) => Bounds {
                    min: [b.min[0].min(p[0]), b.min[1].min(p[1])],
                    max: [b.max[0].max(p[0]), b.max[1].max(p[1])],
                },
            })
        })
    }

    pub fn width(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    pub fn height(&self) -> f64 {
        self.max[1] - self.min[1]
    }
}

impl Item {
    /// Every point the shape is drawn through.
    pub fn points(&self) -> impl Iterator<Item = Point2> + '_ {
        let lists = match &self.shape {
            Shape::Stroke { runs, .. } => runs,
            Shape::Fill { contours, .. } => contours,
        };
        lists.iter().flatten().copied()
    }
}

impl DisplayList {
    /// The box around everything drawn, as AutoCAD's extents are: the
    /// geometry itself, not the width of its pens. None when nothing draws.
    pub fn extents(&self) -> Option<Bounds> {
        let reach = self.reach.into_iter().flat_map(|b| [b.min, b.max]);
        Bounds::of(self.items.iter().flat_map(Item::points).chain(reach))
    }
}

/// How a layer draws its objects, resolved.
struct Pen<'a> {
    color: Color,
    /// The linetype's pattern; none for a continuous line.
    pattern: Option<&'a [f64]>,
    /// Millimetres on paper.
    lineweight: f64,
}

/// Draws a sheet: every object, in drawing order, as AutoCAD plots it.
/// Curves are drawn as chords that stray at most `tolerance` drawing units
/// from them; text is drawn in the fonts `fonts` holds.
pub fn draw_sheet(sheet: &Sheet, fonts: &SheetFonts, tolerance: f64) -> DisplayList {
    let tolerance = if tolerance > 0.0 { tolerance } else { 1e-3 };
    let layer_zero = Layer::new(LAYER_ZERO);
    let pen_of = |name: &str| -> Pen<'_> {
        let layer = sheet
            .layers
            .iter()
            .find(|l| l.name == name)
            .or_else(|| {
                sheet
                    .layers
                    .iter()
                    .find(|l| l.name.eq_ignore_ascii_case(name))
            })
            .unwrap_or(&layer_zero);
        let pattern = if layer.linetype.eq_ignore_ascii_case(CONTINUOUS) {
            None
        } else {
            sheet
                .linetypes
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&layer.linetype))
                .map(|t| t.pattern.as_slice())
        };
        Pen {
            color: Color(layer.color),
            pattern,
            lineweight: layer
                .lineweight
                .map_or(DEFAULT_LINEWEIGHT, |w| w.millimetres()),
        }
    };
    let mut list = DisplayList::default();
    for (index, object) in sheet.objects.iter().enumerate() {
        let pen = pen_of(object.layer());
        let stroke = |runs: Vec<Vec<Point2>>| -> Vec<Shape> {
            if runs.is_empty() {
                Vec::new()
            } else {
                vec![Shape::Stroke {
                    lineweight: pen.lineweight,
                    runs,
                }]
            }
        };
        // Blocks arrive with ticket #6: an insert will draw its block's
        // objects, moved, scaled and turned, and its attributes as text,
        // through this same match.
        let shapes = match object {
            Object::Line(line) => stroke(linetype::along(
                &[flat(line.from), flat(line.to)],
                pen.pattern,
                false,
            )),
            Object::Polyline(polyline) => {
                let mut points: Vec<Point2> = polyline.points.iter().map(|&p| flat(p)).collect();
                if polyline.closed && !points.is_empty() {
                    points.push(points[0]);
                }
                let level = polyline.points.first().map_or(0.0, |p| p.z);
                if polyline.points.iter().all(|p| p.z == level) {
                    // A flat polyline starts its linetype again at each vertex.
                    let mut runs = Vec::new();
                    for segment in points.windows(2) {
                        runs.extend(linetype::along(segment, pen.pattern, false));
                    }
                    stroke(runs)
                } else {
                    // AutoCAD draws a 3D polyline continuous, whatever its linetype.
                    stroke(linetype::along(&points, None, false))
                }
            }
            Object::Arc(arc) => stroke(linetype::along(
                &curves::arc(
                    flat(arc.center),
                    arc.radius,
                    arc.start_angle,
                    arc.end_angle,
                    tolerance,
                ),
                pen.pattern,
                false,
            )),
            Object::Circle(circle) => stroke(linetype::along(
                &curves::circle(flat(circle.center), circle.radius, tolerance),
                pen.pattern,
                true,
            )),
            Object::Ellipse(ellipse) => stroke(linetype::along(
                &curves::ellipse(ellipse, tolerance),
                pen.pattern,
                ellipse.is_whole(),
            )),
            Object::Spline(spline) => stroke(linetype::along(
                &curves::spline(spline, tolerance),
                pen.pattern,
                false,
            )),
            Object::Hatch(h) => {
                let (shapes, boundary) = hatch::draw(h, pen.lineweight, tolerance);
                let reach = list.reach.into_iter().flat_map(|b| [b.min, b.max]);
                list.reach =
                    Bounds::of(reach.chain(boundary.into_iter().flat_map(|b| [b.min, b.max])));
                shapes
            }
            Object::Text(t) => text::draw_text(t, fonts, pen.lineweight, tolerance),
            Object::MText(t) => text::draw_mtext(t, fonts, pen.lineweight, tolerance),
        };
        list.items.extend(shapes.into_iter().map(|shape| Item {
            object: index,
            color: pen.color,
            shape,
        }));
    }
    list
}

/// A point seen from above.
fn flat(p: crate::model::Point3) -> Point2 {
    [p.x, p.y]
}

#[cfg(test)]
mod tests;
