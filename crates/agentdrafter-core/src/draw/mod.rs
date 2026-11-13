//! The drawing pipeline: a sheet becomes drawn geometry, a display list,
//! which the PNG draws from now and the PDF and the viewer draw from later
//! (ADR-0011). Text, linetypes and hatches are laid out here once, for
//! every output.
//!
//! What is drawn follows AutoCAD's plot of the same DWG, seen in AutoCAD
//! 2026's plots of made-up sheets:
//!
//! - Each object draws with its own colour, linetype and lineweight where
//!   it gives them, and its layer's where they are `ByLayer`. One drawn by
//!   an insert takes the insert's where they are `ByBlock`; one on the
//!   sheet draws `ByBlock` white, continuous and at the default
//!   lineweight, as AutoCAD draws it. A layer with no lineweight of its
//!   own draws with AutoCAD's default, 0.25 mm. A lineweight is a width on
//!   paper, so the display list keeps it in millimetres and each output
//!   turns it into its own units.
//! - A linetype's dashes and gaps are enlarged by the sheet's `ltscale`
//!   times the object's own linetype scale, AutoCAD's LTSCALE and
//!   CELTSCALE.
//! - A linetype repeats along each line, arc, circle, ellipse and spline,
//!   and along each segment of a flat polyline, laid out so that both ends
//!   draw alike ([`linetype`]). A 3D polyline draws continuous, as AutoCAD
//!   draws it.
//! - Text draws in its font: an SHX font's strokes with the layer's pen,
//!   and a TrueType font's outlines filled ([`text`]). Each line of text
//!   also gives a run of its characters along its baseline, which draws
//!   nothing, so an output can keep the text searchable where it draws.
//! - A hatch fills its area solid, or draws its pattern's lines inside its
//!   boundary ([`hatch`]).
//! - An insert draws its block's objects moved, scaled and turned. Hatch
//!   patterns and text are enlarged and turned with them, but a linetype
//!   keeps its own lengths, laid out along each object where the insert
//!   puts it. An object on layer `0` draws with the insert's layer. Each
//!   visible attribute draws as single-line text saying its value; an
//!   invisible one, and the block's attribute definitions, draw nothing.
//!   The extents take in the block's box, placed, as AutoCAD's do. Each
//!   shape an insert draws says which object of its block drew it, so the
//!   checker can measure a title block's texts and attributes apart.
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
mod pick;
mod polyline;
mod text;

use serde::{Deserialize, Serialize};

use crate::model::block::{definition_text, Placement};
use crate::model::{
    attribute_text, Block, Insert, Layer, Object, ObjectColor, ObjectLinetype, ObjectLineweight,
    Point3, Polyline, Properties, Sheet, CONTINUOUS, LAYER_ZERO, WHITE,
};
use crate::text::placement::SheetFonts;

pub use color::Color;
pub(crate) use hatch::{MOST_DASHES as MOST_HATCH_DASHES, MOST_LINES as MOST_HATCH_LINES};
pub use pick::{object_bounds, pick};
pub(crate) use polyline::bulge_of;

/// AutoCAD's default lineweight, which a layer with none of its own plots
/// with: 0.25 mm, AutoCAD's `LWDEFAULT` as installed.
pub const DEFAULT_LINEWEIGHT: f64 = 0.25;

/// A point in the drawing's own units, seen from above: `[x, y]`.
pub type Point2 = [f64; 2];

/// What a sheet draws: shapes in drawing order, each with the colour and
/// pen it draws with. Coordinates are in the drawing's units; lineweights
/// are millimetres on paper.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayList {
    pub items: Vec<Item>,
    /// Each line of text the items draw, as the characters it reads, laid
    /// along its baseline. A run draws nothing: the items draw the text.
    /// It lets an output keep the text searchable where it draws, as the
    /// PDF does.
    pub text: Vec<TextRun>,
    /// The box round what counts towards the extents but draws nothing of
    /// its own: the boundaries of patterned hatches, whose lines may stop
    /// short of them, and the boxes of inserts' blocks. AutoCAD's extents
    /// take in both.
    pub reach: Option<Bounds>,
    /// The box round each patterned hatch's boundary, which counts towards
    /// the extents though the hatch's lines may stop short of it, with the
    /// sheet's object that draws the hatch: its place in the sheet's
    /// objects, counted from 0. An insert whose block holds a hatch draws it.
    pub boundaries: Vec<(usize, Bounds)>,
    /// The hatches whose pattern needed more lines or dashes than one hatch
    /// draws, and so are not drawn in full.
    pub hatches_cut_short: Vec<HatchCutShort>,
}

/// A hatch whose pattern needed more lines or dashes than one hatch draws:
/// only the first of them are drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HatchCutShort {
    /// The place in the sheet's objects, counted from 0, of the hatch, or
    /// of the insert whose block holds it.
    pub object: usize,
    /// The block that holds the hatch, when an insert draws it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// The hatch's pattern.
    pub pattern: String,
    /// The hatch's own layer.
    pub layer: String,
    /// Why it was not drawn in full.
    pub why: String,
}

/// One line of text as it reads: its characters, laid along its baseline
/// as the font advances them.
///
/// The line's frame maps the font's own units to drawing units: a point
/// `(u, v)` of the font, `u` along the line in the font's advances and `v`
/// up in heights of its capitals, is at `origin + u * across + v * up`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    /// Which object of the sheet draws it: its place in the sheet's
    /// objects, counted from 0.
    pub object: usize,
    /// When an insert draws it, which object of the insert's block, as for
    /// an [`Item`]. None for an object of the sheet's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<usize>,
    /// The box round what its characters draw, in the line's frame: its
    /// lower left and upper right corners `(u, v)`. None when they draw
    /// nothing, as spaces do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drawn: Option<[Point2; 2]>,
    /// Where the first character's pen starts, on the baseline.
    pub origin: Point2,
    /// One unit of the font's advances along the baseline: the text's
    /// height times its width factor, turned with it.
    pub across: Point2,
    /// From the baseline to the top of the capitals: the text's height,
    /// turned with it.
    pub up: Point2,
    /// Each character the line draws, with how far the pen moves along the
    /// baseline after it, in units of `across`.
    pub characters: Vec<(char, f64)>,
}

/// One shape of one object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Which object of the sheet draws it: its place in the sheet's
    /// objects, counted from 0.
    pub object: usize,
    /// When an insert draws it, which object of the insert's block: its
    /// place in the block's objects, counted from 0, an attribute's being
    /// its attribute definition's. None for an object of the sheet's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<usize>,
    /// The AutoCAD colour index it draws in.
    pub color: Color,
    pub shape: Shape,
}

/// A shape: lines drawn with a pen, or an area filled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    /// Inside an odd number of contours: a hatch's islands.
    EvenOdd,
    /// Where the contours wind round a point other than zero times: a
    /// TrueType outline.
    NonZero,
}

/// The box around drawn geometry, in drawing units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: Point2,
    pub max: Point2,
}

impl Bounds {
    /// The box round `points`; none when there are none.
    pub(crate) fn of(points: impl Iterator<Item = Point2>) -> Option<Bounds> {
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

impl Shape {
    /// Every point the shape is drawn through.
    pub fn points(&self) -> impl Iterator<Item = Point2> + '_ {
        let lists = match self {
            Shape::Stroke { runs, .. } => runs,
            Shape::Fill { contours, .. } => contours,
        };
        lists.iter().flatten().copied()
    }
}

impl Item {
    /// Every point the shape is drawn through.
    pub fn points(&self) -> impl Iterator<Item = Point2> + '_ {
        self.shape.points()
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

/// How an object draws, resolved from its own properties, its layer's and
/// its insert's.
#[derive(Clone, Copy)]
struct Pen<'a> {
    color: Color,
    /// The linetype's pattern, as defined; none for a continuous line.
    pattern: Option<&'a [f64]>,
    /// What the pattern's lengths are enlarged by.
    scale: f64,
    /// Millimetres on paper.
    lineweight: f64,
}

impl Pen<'_> {
    /// How a `ByBlock` object on the sheet itself draws, outside any
    /// insert: white, continuous, at the default lineweight, as AutoCAD
    /// draws it.
    const OUTSIDE_A_BLOCK: Pen<'static> = Pen {
        color: Color(WHITE),
        pattern: None,
        scale: 1.0,
        lineweight: DEFAULT_LINEWEIGHT,
    };

    /// The pattern with its lengths enlarged by the scale.
    fn dashes(&self) -> Option<Vec<f64>> {
        self.pattern
            .map(|pattern| pattern.iter().map(|l| l * self.scale).collect())
    }
}

/// What one object draws, in its own coordinates.
#[derive(Default)]
struct Drawn {
    shapes: Vec<Shape>,
    /// The lines of text it draws, as they read.
    runs: Vec<TextRun>,
    /// The points that count towards the extents without drawing: a
    /// patterned hatch's boundary.
    reach: Vec<Point2>,
    /// Whether it is a hatch whose pattern was cut short.
    cut_short: bool,
}

/// Where an insert puts its block's points, by the one placement rule.
struct Placed(Placement);

impl Placed {
    fn of(insert: &Insert, block: &Block) -> Placed {
        Placed(Placement::of(insert, block.base_point))
    }

    fn point(&self, p: Point2) -> Point2 {
        flat(self.0.point(Point3::new(p[0], p[1], 0.0)))
    }

    /// Where a placed point is in the block's own coordinates.
    fn back(&self, p: Point2) -> Point2 {
        self.0.back(p[0], p[1])
    }

    fn shape(&self, shape: Shape) -> Shape {
        let place = |lists: Vec<Vec<Point2>>| -> Vec<Vec<Point2>> {
            lists
                .into_iter()
                .map(|list| list.into_iter().map(|p| self.point(p)).collect())
                .collect()
        };
        match shape {
            Shape::Stroke { lineweight, runs } => Shape::Stroke {
                lineweight,
                runs: place(runs),
            },
            Shape::Fill { rule, contours } => Shape::Fill {
                rule,
                contours: place(contours),
            },
        }
    }

    /// A line of text placed: its frame moved, scaled and turned.
    fn run(&self, run: TextRun) -> TextRun {
        let origin = self.point(run.origin);
        let vector = |v: Point2| {
            let tip = self.point([run.origin[0] + v[0], run.origin[1] + v[1]]);
            [tip[0] - origin[0], tip[1] - origin[1]]
        };
        TextRun {
            origin,
            across: vector(run.across),
            up: vector(run.up),
            ..run
        }
    }
}

/// What draws a sheet's objects: its layers, linetypes, blocks and fonts.
struct Drawer<'a> {
    sheet: &'a Sheet,
    fonts: &'a SheetFonts,
    most_dashes: usize,
    layer_zero: Layer,
}

impl<'a> Drawer<'a> {
    /// How an object with `properties` on the layer named `layer` draws:
    /// with `by_block` where a property is `ByBlock`, the pen of the insert
    /// that draws it, if any.
    fn pen(&self, layer: &str, properties: &Properties, by_block: Option<Pen<'a>>) -> Pen<'a> {
        let by_layer = self.layer_pen(layer);
        let by_block = by_block.unwrap_or(Pen::OUTSIDE_A_BLOCK);
        let color = match properties.color {
            ObjectColor::ByLayer => by_layer.color,
            ObjectColor::ByBlock => by_block.color,
            ObjectColor::Index(index) => Color(index),
        };
        let pattern = match &properties.linetype {
            ObjectLinetype::ByLayer => by_layer.pattern,
            ObjectLinetype::ByBlock => by_block.pattern,
            ObjectLinetype::Named(name) => self.pattern(name),
        };
        let lineweight = match properties.lineweight {
            ObjectLineweight::ByLayer => by_layer.lineweight,
            ObjectLineweight::ByBlock => by_block.lineweight,
            ObjectLineweight::Default => DEFAULT_LINEWEIGHT,
            ObjectLineweight::Width(width) => width.millimetres(),
        };
        Pen {
            color,
            pattern,
            scale: self.sheet.ltscale * properties.linetype_scale,
            lineweight,
        }
    }

    /// The pattern of the linetype named `name`; none for `Continuous`, or
    /// for a linetype the sheet does not define.
    fn pattern(&self, name: &str) -> Option<&'a [f64]> {
        if name.eq_ignore_ascii_case(CONTINUOUS) {
            return None;
        }
        self.sheet
            .linetypes
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .map(|t| t.pattern.as_slice())
    }

    /// How the layer named `name` draws, layer `0`'s defaults for a layer
    /// the sheet does not define.
    fn layer_pen(&self, name: &str) -> Pen<'a> {
        let sheet = self.sheet;
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
            .unwrap_or(&self.layer_zero);
        Pen {
            color: Color(layer.color),
            pattern: self.pattern(&layer.linetype),
            scale: 1.0,
            lineweight: layer
                .lineweight
                .map_or(DEFAULT_LINEWEIGHT, |w| w.millimetres()),
        }
    }

    /// The block an insert places, by its name.
    fn block(&self, name: &str) -> Option<&'a Block> {
        let blocks = &self.sheet.blocks;
        blocks
            .iter()
            .find(|b| b.name == name)
            .or_else(|| blocks.iter().find(|b| b.name.eq_ignore_ascii_case(name)))
    }

    /// Adds what the sheet's object at `index` draws to `list`.
    fn sheet_object(&self, index: usize, object: &Object, tolerance: f64, list: &mut DisplayList) {
        if let Object::Insert(insert) = object {
            self.insert(index, insert, tolerance, list);
            return;
        }
        let pen = self.pen(object.layer(), object.properties(), None);
        let drawn = self.object(object, pen, tolerance, None);
        if drawn.cut_short {
            list.hatches_cut_short
                .push(self.cut_short(index, object, None));
        }
        list.add(index, None, pen.color, drawn.shapes, drawn.reach);
        list.add_text(index, None, drawn.runs);
    }

    /// Adds what an insert draws to `list`: its block's objects moved,
    /// scaled and turned, those on layer `0` drawn with the insert's layer,
    /// and each visible attribute as text saying its value, as AutoCAD
    /// plots an insert. The block's attribute definitions themselves draw
    /// nothing on an insert.
    fn insert(&self, index: usize, insert: &Insert, tolerance: f64, list: &mut DisplayList) {
        let Some(block) = self.block(&insert.block) else {
            return;
        };
        let placed = Placed::of(insert, block);
        // What the block's `ByBlock` objects draw with.
        let insert_pen = self.pen(&insert.layer, &insert.properties, None);
        // Curves are made chords in the block's own coordinates, then
        // enlarged: to keep within the tolerance once enlarged, they are
        // made finer.
        let within = tolerance / insert.scale.abs().max(f64::MIN_POSITIVE);
        // The box round the block's objects in its own coordinates.
        let mut own: Option<Bounds> = None;
        for (part, object) in block.objects.iter().enumerate() {
            if matches!(object, Object::AttributeDefinition(_) | Object::Insert(_)) {
                continue;
            }
            let layer = match object.layer() {
                LAYER_ZERO => insert.layer.as_str(),
                layer => layer,
            };
            let pen = self.pen(layer, object.properties(), Some(insert_pen));
            let drawn = self.object(object, pen, within, Some(&placed));
            if drawn.cut_short {
                let cut = self.cut_short(index, object, Some(&block.name));
                list.hatches_cut_short.push(cut);
            }
            let drawn_points = drawn.shapes.iter().flat_map(Shape::points);
            let points = drawn_points.chain(drawn.reach.iter().copied());
            let had = own.into_iter().flat_map(|b| [b.min, b.max]);
            own = Bounds::of(had.chain(points.map(|p| placed.back(p))));
            list.add(index, Some(part), pen.color, drawn.shapes, drawn.reach);
            list.add_text(index, Some(part), drawn.runs);
        }
        // AutoCAD's extents take in an insert's block box: the box round the
        // block's objects in its own coordinates, placed whole. Turned, it
        // reaches past what the insert draws.
        if let Some(b) = own {
            let corners = [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]];
            list.take_in(corners.map(|c| placed.point(c)));
        }
        for (part, object) in block.objects.iter().enumerate() {
            let Object::AttributeDefinition(definition) = object else {
                continue;
            };
            if definition.invisible {
                continue;
            }
            let value = insert
                .attributes
                .iter()
                .find(|a| a.tag == definition.tag)
                .map_or(definition.default.as_str(), |a| a.value.as_str());
            let text = attribute_text(insert, block, definition, value);
            let pen = self.pen(&text.layer, &text.properties, Some(insert_pen));
            let lettering = text::draw_text(&text, self.fonts, pen.lineweight, tolerance);
            list.add(index, Some(part), pen.color, lettering.shapes, Vec::new());
            list.add_text(index, Some(part), lettering.runs);
        }
    }

    /// What one object draws with `pen`, put where `placed` puts it when an
    /// insert draws it. Its path is placed before its linetype is laid out
    /// along it, so that the linetype keeps its own lengths, as AutoCAD
    /// 2026 plotted a made-up probe of inserts scaled and turned; anything
    /// else is drawn and then placed whole. `tolerance` is in the object's
    /// own units. An insert draws nothing here: [`Drawer::insert`] draws it.
    fn object(
        &self,
        object: &Object,
        pen: Pen<'_>,
        tolerance: f64,
        placed: Option<&Placed>,
    ) -> Drawn {
        let dashes = pen.dashes();
        let pattern = dashes.as_deref();
        let path = |points: Vec<Point2>| -> Vec<Point2> {
            match placed {
                Some(placed) => points.into_iter().map(|p| placed.point(p)).collect(),
                None => points,
            }
        };
        let stroke = |runs: Vec<Vec<Point2>>| -> Drawn {
            let shapes = if runs.is_empty() {
                Vec::new()
            } else {
                vec![Shape::Stroke {
                    lineweight: pen.lineweight,
                    runs,
                }]
            };
            Drawn {
                shapes,
                ..Drawn::default()
            }
        };
        let shapes = |shapes: Vec<Shape>| Drawn {
            shapes: match placed {
                Some(placed) => shapes.into_iter().map(|s| placed.shape(s)).collect(),
                None => shapes,
            },
            ..Drawn::default()
        };
        let lettering = |lettering: text::Lettering| Drawn {
            runs: match placed {
                Some(placed) => lettering.runs.into_iter().map(|r| placed.run(r)).collect(),
                None => lettering.runs,
            },
            ..shapes(lettering.shapes)
        };
        // A flat polyline: its thin segments stroked, each starting its
        // linetype again, as AutoCAD draws a polyline with PLINEGEN off,
        // and its wide segments filled.
        let flat_polyline = |polyline: &Polyline| -> Drawn {
            let points: Vec<Point2> = polyline.points.iter().map(|&p| flat(p)).collect();
            let drawn = polyline::draw(
                &points,
                polyline.closed,
                |i| polyline.bulge(i),
                |i| polyline.widths_at(i),
                pattern,
                tolerance,
            );
            let mut runs = Vec::new();
            for segment in drawn.thin {
                runs.extend(linetype::along(&path(segment), pattern, false));
            }
            let mut out = stroke(runs);
            if !drawn.fills.is_empty() {
                out.shapes.extend(
                    shapes(vec![Shape::Fill {
                        rule: FillRule::NonZero,
                        contours: drawn.fills,
                    }])
                    .shapes,
                );
            }
            out
        };
        match object {
            Object::Line(line) => stroke(linetype::along(
                &path(vec![flat(line.from), flat(line.to)]),
                pattern,
                false,
            )),
            Object::Polyline(polyline) if polyline.is_flat() => flat_polyline(polyline),
            Object::Donut(donut) => flat_polyline(&donut.polyline()),
            Object::Solid(solid) => shapes(vec![Shape::Fill {
                rule: FillRule::NonZero,
                contours: vec![polyline::anticlockwise(
                    solid.corners.iter().map(|&p| flat(p)).collect(),
                )],
            }]),
            Object::Polyline(polyline) => {
                let mut points = path(polyline.points.iter().map(|&p| flat(p)).collect());
                if polyline.closed && !points.is_empty() {
                    points.push(points[0]);
                }
                {
                    // AutoCAD draws a 3D polyline continuous, whatever its linetype.
                    stroke(linetype::along(&points, None, false))
                }
            }
            Object::Arc(arc) => stroke(linetype::along(
                &path(curves::arc(
                    flat(arc.center),
                    arc.radius,
                    arc.start_angle,
                    arc.end_angle,
                    tolerance,
                )),
                pattern,
                false,
            )),
            Object::Circle(circle) => stroke(linetype::along(
                &path(curves::circle(
                    flat(circle.center),
                    circle.radius,
                    tolerance,
                )),
                pattern,
                true,
            )),
            Object::Ellipse(ellipse) => stroke(linetype::along(
                &path(curves::ellipse(ellipse, tolerance)),
                pattern,
                ellipse.is_whole(),
            )),
            Object::Spline(spline) => stroke(linetype::along(
                &path(curves::spline(spline, tolerance)),
                pattern,
                false,
            )),
            Object::Hatch(h) => {
                // A hatch's pattern is enlarged and turned with the insert.
                let drawn = hatch::draw(h, pen.lineweight, tolerance, self.most_dashes);
                Drawn {
                    reach: path(drawn.boundary),
                    cut_short: drawn.cut_short,
                    ..shapes(drawn.shapes)
                }
            }
            Object::Text(t) => lettering(text::draw_text(t, self.fonts, pen.lineweight, tolerance)),
            Object::MText(t) => {
                lettering(text::draw_mtext(t, self.fonts, pen.lineweight, tolerance))
            }
            // An attribute definition outside a block shows its tag, as
            // AutoCAD shows it.
            Object::AttributeDefinition(d) => lettering(text::draw_text(
                &definition_text(d),
                self.fonts,
                pen.lineweight,
                tolerance,
            )),
            Object::Insert(_) => Drawn::default(),
        }
    }

    /// The record of a hatch cut short: the sheet's object at `index`, or
    /// one of the objects of `block`, which that object inserts.
    fn cut_short(&self, index: usize, object: &Object, block: Option<&str>) -> HatchCutShort {
        let pattern = match object {
            Object::Hatch(h) => h.pattern.name.clone(),
            _ => String::new(),
        };
        HatchCutShort {
            object: index,
            block: block.map(str::to_owned),
            pattern,
            layer: object.layer().to_owned(),
            why: format!(
                "its pattern needs more than the {MOST_HATCH_LINES} lines or {} dashes one \
                 hatch draws; only the first are drawn",
                self.most_dashes
            ),
        }
    }
}

impl DisplayList {
    /// Adds what the sheet's object at `index` draws in `color`: the
    /// object of its block at `part`, when it is an insert. `reach` is a
    /// patterned hatch's boundary, which draws nothing of its own.
    fn add(
        &mut self,
        index: usize,
        part: Option<usize>,
        color: Color,
        shapes: Vec<Shape>,
        reach: Vec<Point2>,
    ) {
        if let Some(boundary) = Bounds::of(reach.iter().copied()) {
            self.boundaries.push((index, boundary));
        }
        self.take_in(reach);
        self.items.extend(shapes.into_iter().map(|shape| Item {
            object: index,
            part,
            color,
            shape,
        }));
    }

    /// Adds the lines of text the sheet's object at `index` draws, as they
    /// read: the object of its block at `part`, when it is an insert.
    fn add_text(&mut self, index: usize, part: Option<usize>, runs: Vec<TextRun>) {
        self.text.extend(runs.into_iter().map(|run| TextRun {
            object: index,
            part,
            ..run
        }));
    }

    /// Takes `points` into the extents without drawing them.
    fn take_in(&mut self, points: impl IntoIterator<Item = Point2>) {
        let mut points = points.into_iter().peekable();
        if points.peek().is_some() {
            let had = self.reach.into_iter().flat_map(|b| [b.min, b.max]);
            self.reach = Bounds::of(had.chain(points));
        }
    }
}

/// Draws a sheet: every object, in drawing order, as AutoCAD plots it.
/// Curves are drawn as chords that stray at most `tolerance` drawing units
/// from them; text is drawn in the fonts `fonts` holds; each hatch draws
/// at most `most_dashes` dashes, [`MOST_HATCH_DASHES`] but in tests.
pub fn draw_sheet(
    sheet: &Sheet,
    fonts: &SheetFonts,
    tolerance: f64,
    most_dashes: usize,
) -> DisplayList {
    let tolerance = if tolerance > 0.0 { tolerance } else { 1e-3 };
    let drawer = Drawer {
        sheet,
        fonts,
        most_dashes,
        layer_zero: Layer::new(LAYER_ZERO),
    };
    let mut list = DisplayList::default();
    for (index, object) in sheet.objects.iter().enumerate() {
        drawer.sheet_object(index, object, tolerance, &mut list);
    }
    list
}

/// A point seen from above.
fn flat(p: crate::model::Point3) -> Point2 {
    [p.x, p.y]
}

#[cfg(test)]
mod tests;
