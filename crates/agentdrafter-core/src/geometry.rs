//! An object's geometry, read exactly from its record: the curves it is
//! made of and the points AutoCAD's object snaps find on it, inserts read
//! into their blocks' own geometry, nested inserts too. [`snap`] answers
//! object snaps with it, and [`measure`] an object's length, area, extents
//! and angles.
//!
//! Every point is in the sheet's coordinates and declared unit. An insert's
//! block is placed by the one placement rule ([`Placement`]): moved,
//! enlarged evenly and turned about the Z axis, so its lines stay lines,
//! its arcs arcs and its ellipses ellipses.

pub(crate) mod curve;
pub mod measure;
pub mod snap;

use std::f64::consts::TAU;

use crate::draw::curves::{ellipse_parameter, spline_definition};
use crate::draw::polyline::arc_of;
use crate::draw::text::mtext_box;
use crate::model::block::{definition_text, Placement};
use crate::model::{
    attribute_text, Block, Dimension, Insert, LaidDimension, MLeaderStyle, Object, Point3,
    Polyline, Sheet,
};
use crate::text::placement::SheetFonts;
use crate::text::Justify;
use curve::{BSpline, Curve, P3};
pub use snap::{ObjectRef, SnapKind, SnapRef};

/// What an object's geometry is read with: its sheet, for its blocks and
/// styles, and the fonts its text is measured in.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    pub(crate) sheet: &'a Sheet,
    pub(crate) fonts: &'a SheetFonts,
}

/// How a point of a block's own coordinates lands on the sheet: placed by
/// the insert of the block, then by each insert that holds it, innermost
/// first. Empty for an object of the sheet's own.
#[derive(Clone, Default)]
pub(crate) struct Chain {
    placements: Vec<Placement>,
    /// The blocks the chain passes into, outermost first.
    pub(crate) blocks: Vec<String>,
}

impl Chain {
    /// The chain into `block`, which `insert`, placed by this chain, inserts.
    fn within(&self, insert: &Insert, block: &Block) -> Chain {
        let mut placements = vec![Placement::of(insert, block.base_point)];
        placements.extend(self.placements.iter().cloned());
        let mut blocks = self.blocks.clone();
        blocks.push(block.name.clone());
        Chain { placements, blocks }
    }

    pub(crate) fn point(&self, p: P3) -> P3 {
        let mut at = Point3::new(p[0], p[1], p[2]);
        for placement in &self.placements {
            at = placement.point(at);
        }
        [at.x, at.y, at.z]
    }

    fn curve(&self, c: &Curve) -> Curve {
        if self.placements.is_empty() {
            return c.clone();
        }
        let scale = self.placements.iter().map(Placement::scale).product();
        let mirrored = self.placements.iter().filter(|p| p.mirrored()).count() % 2 == 1;
        let angle = |radians: f64| self.placements.iter().fold(radians, |a, p| p.radians(a));
        c.placed(&|p| self.point(p), scale, &angle, mirrored)
    }
}

/// A clipped viewport's boundary, as the closed polyline or the circle it
/// is drawn as.
fn clip_boundary(viewport: &crate::model::Viewport) -> Object {
    let layer = viewport.layer.clone();
    let properties = viewport.properties.clone();
    match viewport.clip.as_ref() {
        Some(crate::model::Clip::Circle { center, radius }) => {
            Object::Circle(crate::model::Circle {
                center: *center,
                radius: *radius,
                layer,
                properties,
            })
        }
        Some(crate::model::Clip::Polyline { points, bulges }) => {
            Object::Polyline(crate::model::Polyline {
                points: points.clone(),
                bulges: bulges.clone(),
                widths: Vec::new(),
                width: 0.0,
                closed: true,
                layer,
                properties,
            })
        }
        None => Object::Viewport(viewport.clone()),
    }
}

pub(crate) fn p3(p: Point3) -> P3 {
    [p.x, p.y, p.z]
}

pub(crate) fn point3(p: P3) -> Point3 {
    Point3::new(p[0], p[1], p[2])
}

/// `path` and `leaf` joined, as `objects[3]` and `to` make `objects[3].to`.
fn join(path: &str, leaf: &str) -> String {
    match (path.is_empty(), leaf.is_empty()) {
        (true, _) => leaf.to_owned(),
        (_, true) => path.to_owned(),
        _ => format!("{path}.{leaf}"),
    }
}

/// One curve of an object, where it comes from in it, and where it lies.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    /// Its place in the object: `segments[1]` of a polyline, `objects[3]`
    /// of an insert's block, `objects[2].segments[0]`; empty for the
    /// object itself.
    pub(crate) path: String,
    /// The blocks the path passes into, outermost first.
    pub(crate) blocks: Vec<String>,
    pub(crate) curve: Curve,
}

/// A point one of AutoCAD's object snaps finds on an object by itself.
#[derive(Clone, Debug)]
pub(crate) struct OwnSnap {
    pub(crate) kind: SnapKind,
    /// Which of the object's points of that kind it is, named by its place:
    /// `to`, `points[2]`, `segments[1]`, `90`, `objects[3].to`; empty when
    /// the object has it once and it needs no name.
    pub(crate) which: String,
    pub(crate) blocks: Vec<String>,
    pub(crate) at: P3,
}

/// An arc's angles, in degrees, as a start and a sweep in radians.
fn arc_sweep(start_angle: f64, end_angle: f64) -> (f64, f64) {
    let sweep = (end_angle - start_angle).rem_euclid(360.0);
    let sweep = if sweep == 0.0 { 360.0 } else { sweep };
    (start_angle.to_radians(), sweep.to_radians())
}

/// The curve of a polyline's segment that starts at point `i`.
pub(crate) fn polyline_segment(polyline: &Polyline, i: usize) -> Option<Curve> {
    let n = polyline.points.len();
    let j = if i + 1 < n {
        i + 1
    } else if polyline.closed && n > 1 {
        0
    } else {
        return None;
    };
    let (a, b) = (polyline.points[i], polyline.points[j]);
    let bulge = if polyline.is_flat() {
        polyline.bulge(i)
    } else {
        0.0
    };
    Some(match arc_of([a.x, a.y], [b.x, b.y], bulge) {
        Some(arc) => Curve::Arc {
            center: [arc.center[0], arc.center[1], a.z],
            radius: arc.radius,
            start: arc.start,
            sweep: arc.sweep,
        },
        None => Curve::Segment { a: p3(a), b: p3(b) },
    })
}

/// The segments of a polyline, each with its place, `segments[i]`.
fn polyline_segments(polyline: &Polyline) -> Vec<(String, Curve)> {
    (0..polyline.points.len())
        .filter_map(|i| polyline_segment(polyline, i).map(|c| (format!("segments[{i}]"), c)))
        .collect()
}

/// The closed outline through `corners`, each edge with its place.
fn outline(corners: &[P3]) -> Vec<(String, Curve)> {
    let n = corners.len();
    (0..n)
        .filter(|&i| n > 2 || i + 1 < n)
        .map(|i| {
            (
                format!("segments[{i}]"),
                Curve::Segment {
                    a: corners[i],
                    b: corners[(i + 1) % n],
                },
            )
        })
        .collect()
}

/// The open path through `points`, each segment with its place.
fn path_through(points: &[P3]) -> Vec<(String, Curve)> {
    points
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            (
                format!("segments[{i}]"),
                Curve::Segment { a: w[0], b: w[1] },
            )
        })
        .collect()
}

impl<'a> Reader<'a> {
    fn block(&self, name: &str) -> Option<&'a Block> {
        self.sheet.blocks.iter().find(|b| b.name == name)
    }

    /// The curves an object of the sheet's own, or of a block, is made of,
    /// in its own coordinates, with their places; none for an insert, whose
    /// block's curves [`Reader::parts`] reads. A hatch, text and a
    /// multileader have none an object snap finds a line in: AutoCAD snaps
    /// to no hatch by default (`OSNAPHATCH` 0), and finds no point along a
    /// multileader's lines but their ends.
    pub(crate) fn curves(&self, object: &Object) -> Vec<(String, Curve)> {
        let one = |c: Curve| vec![(String::new(), c)];
        match object {
            Object::Line(line) => one(Curve::Segment {
                a: p3(line.from),
                b: p3(line.to),
            }),
            Object::Polyline(polyline) => polyline_segments(polyline),
            Object::Arc(arc) => {
                let (start, sweep) = arc_sweep(arc.start_angle, arc.end_angle);
                one(Curve::Arc {
                    center: p3(arc.center),
                    radius: arc.radius,
                    start,
                    sweep,
                })
            }
            Object::Circle(circle) => one(Curve::Arc {
                center: p3(circle.center),
                radius: circle.radius,
                start: 0.0,
                sweep: TAU,
            }),
            Object::Ellipse(e) => {
                let major = [e.major_axis.x, e.major_axis.y];
                let minor = [-major[1] * e.ratio, major[0] * e.ratio];
                let (start, sweep) = if e.is_whole() {
                    (0.0, TAU)
                } else {
                    let start = ellipse_parameter(e.start_angle, e.ratio);
                    let end = ellipse_parameter(e.end_angle, e.ratio);
                    let sweep = (end - start).rem_euclid(TAU);
                    (start, if sweep == 0.0 { TAU } else { sweep })
                };
                one(Curve::Ellipse {
                    center: p3(e.center),
                    major,
                    minor,
                    start,
                    sweep,
                })
            }
            Object::Spline(spline) => match spline_definition(spline) {
                Some((degree, control, knots)) => {
                    one(Curve::Spline(BSpline::new(degree, control, knots)))
                }
                None => Vec::new(),
            },
            Object::Donut(donut) => polyline_segments(&donut.polyline()),
            Object::Solid(solid) => {
                let corners: Vec<P3> = solid.corners.iter().map(|&p| p3(p)).collect();
                outline(&corners)
            }
            // A viewport's frame, on the paper: its clipping boundary, when
            // it is clipped, as the page draws it.
            Object::Viewport(viewport) if viewport.clip.is_some() => {
                self.curves(&clip_boundary(viewport))
            }
            Object::Viewport(viewport) => {
                let corners: Vec<P3> = viewport.frame().iter().map(|&p| p3(p)).collect();
                outline(&corners)
            }
            Object::Table(table) => {
                let z = table.position.z;
                table
                    .borders()
                    .into_iter()
                    .enumerate()
                    .map(|(i, [a, b])| {
                        (
                            format!("borders[{i}]"),
                            Curve::Segment {
                                a: [a[0], a[1], z],
                                b: [b[0], b[1], z],
                            },
                        )
                    })
                    .collect()
            }
            Object::Leader(leader) => {
                let path: Vec<P3> = leader.path().into_iter().map(p3).collect();
                path_through(&path)
            }
            Object::Dimension(dimension) => match self.laid_dimension(dimension) {
                Some(laid) => {
                    let mut curves: Vec<(String, Curve)> = laid
                        .lines()
                        .enumerate()
                        .map(|(i, [a, b])| {
                            (format!("lines[{i}]"), Curve::Segment { a: p3(a), b: p3(b) })
                        })
                        .collect();
                    curves.extend(laid.arcs().enumerate().map(
                        |(i, (center, radius, start, end))| {
                            let curve = Curve::Arc {
                                center: p3(center),
                                radius,
                                start: start.to_radians(),
                                sweep: (end - start).to_radians(),
                            };
                            (format!("arcs[{i}]"), curve)
                        },
                    ));
                    curves
                }
                None => Vec::new(),
            },
            Object::Hatch(_)
            | Object::Text(_)
            | Object::MText(_)
            | Object::AttributeDefinition(_)
            | Object::Insert(_)
            | Object::MLeader(_) => Vec::new(),
        }
    }

    /// The curves of an object on the sheet, in the sheet's coordinates: an
    /// insert's are its block's, placed, and those of each insert nested in
    /// it, each with its path in the block, `objects[3]` and so on.
    pub(crate) fn parts(&self, object: &Object) -> Vec<Part> {
        let mut parts = Vec::new();
        self.parts_into(object, &Chain::default(), "", &mut parts);
        parts
    }

    fn parts_into(&self, object: &Object, chain: &Chain, path: &str, out: &mut Vec<Part>) {
        if let Object::Insert(insert) = object {
            let Some(block) = self.block(&insert.block) else {
                return;
            };
            let inner = chain.within(insert, block);
            for (k, object) in block.objects.iter().enumerate() {
                let here = join(path, &format!("objects[{k}]"));
                self.parts_into(object, &inner, &here, out);
            }
            return;
        }
        for (place, curve) in self.curves(object) {
            out.push(Part {
                path: join(path, &place),
                blocks: chain.blocks.clone(),
                curve: chain.curve(&curve),
            });
        }
    }

    /// The points each of AutoCAD's object snaps of one object finds on an
    /// object of the sheet: endpoint, midpoint, centre, quadrant, insertion
    /// and node, an insert's in its block too.
    pub(crate) fn own_snaps(&self, object: &Object) -> Vec<OwnSnap> {
        let mut out = Vec::new();
        self.own_snaps_into(object, &Chain::default(), "", &mut out);
        out
    }

    fn own_snaps_into(&self, object: &Object, chain: &Chain, path: &str, out: &mut Vec<OwnSnap>) {
        let mut add = |kind: SnapKind, leaf: &str, at: P3| {
            out.push(OwnSnap {
                kind,
                which: join(path, leaf),
                blocks: chain.blocks.clone(),
                at: chain.point(at),
            })
        };
        match object {
            Object::Insert(insert) => {
                // An insert nested in a block is snapped to as part of the
                // insert that holds it: AutoCAD 2026's insertion snap on it
                // found the outer insert's point.
                if chain.blocks.is_empty() {
                    add(SnapKind::Insertion, "position", p3(insert.position));
                }
                let Some(block) = self.block(&insert.block) else {
                    return;
                };
                let inner = chain.within(insert, block);
                for (k, object) in block.objects.iter().enumerate() {
                    let here = join(path, &format!("objects[{k}]"));
                    match object {
                        // An attribute definition draws nothing on an
                        // insert: its attribute, the insert's own text,
                        // does, where the insert puts it.
                        Object::AttributeDefinition(definition) => {
                            if definition.invisible {
                                continue;
                            }
                            let value = block
                                .attribute_value(insert, k)
                                .unwrap_or(&definition.default);
                            // A constant one on a mirrored insert is drawn
                            // with its block, mirrored, as AutoCAD draws it:
                            // its snaps are its text's in the block, placed.
                            let (snaps, placed) = if definition.constant && insert.mirrored {
                                let in_block = crate::text::Text {
                                    contents: value.to_owned(),
                                    ..crate::model::block::definition_text(definition)
                                };
                                (self.text_snaps(&in_block), &inner)
                            } else {
                                let text = attribute_text(insert, block, definition, value);
                                (self.text_snaps(&text), chain)
                            };
                            for (kind, at) in snaps {
                                out.push(OwnSnap {
                                    kind,
                                    which: here.clone(),
                                    blocks: inner.blocks.clone(),
                                    at: placed.point(at),
                                });
                            }
                        }
                        _ => self.own_snaps_into(object, &inner, &here, out),
                    }
                }
            }
            _ => {
                for (kind, leaf, at) in self.local_snaps(object) {
                    add(kind, &leaf, at);
                }
            }
        }
    }

    /// The insertion point and node of a single line of text: its position,
    /// and, when it is justified other than left, where its baseline
    /// starts, the point AutoCAD 2026's node snap found on justified text.
    fn text_snaps(&self, text: &crate::text::Text) -> Vec<(SnapKind, P3)> {
        let mut snaps = vec![(SnapKind::Insertion, p3(text.position))];
        if text.justify != Justify::Left {
            snaps.push((SnapKind::Node, p3(self.fonts.start(text))));
        }
        snaps
    }

    /// The snaps of an object other than an insert, in its own coordinates:
    /// each kind, the point's name in the object, and where it is.
    fn local_snaps(&self, object: &Object) -> Vec<(SnapKind, String, P3)> {
        use SnapKind::{Center, Endpoint, Insertion, Midpoint, Node, Quadrant};
        let mut out: Vec<(SnapKind, String, P3)> = Vec::new();
        let named = |key: &str, points: &[Point3]| -> Vec<(String, P3)> {
            points
                .iter()
                .enumerate()
                .map(|(i, &p)| (format!("{key}[{i}]"), p3(p)))
                .collect()
        };
        // The centre of an arc, and its quadrants that lie on it.
        let round = |out: &mut Vec<(SnapKind, String, P3)>, place: &str, curve: &Curve| {
            if let Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } = curve
            {
                out.push((Center, place.to_owned(), *center));
                for (degrees, [cos, sin]) in QUADRANTS {
                    let angle = f64::from(degrees).to_radians();
                    if on_arc(angle, *start, *sweep) {
                        out.push((
                            Quadrant,
                            join(place, &degrees.to_string()),
                            [
                                center[0] + radius * cos,
                                center[1] + radius * sin,
                                center[2],
                            ],
                        ));
                    }
                }
            }
        };
        match object {
            Object::Line(line) => {
                out.push((Endpoint, "from".into(), p3(line.from)));
                out.push((Endpoint, "to".into(), p3(line.to)));
                for (place, curve) in self.curves(object) {
                    out.push((Midpoint, place, curve.point(0.5)));
                }
            }
            Object::Polyline(polyline) => {
                for (name, p) in named("points", &polyline.points) {
                    out.push((Endpoint, name, p));
                }
                for (place, curve) in polyline_segments(polyline) {
                    out.push((Midpoint, place.clone(), curve.point(curve.middle())));
                    round(&mut out, &place, &curve);
                }
            }
            Object::Donut(donut) => {
                let polyline = donut.polyline();
                for (name, p) in named("points", &polyline.points) {
                    out.push((Endpoint, name, p));
                }
                let segments = polyline_segments(&polyline);
                for (place, curve) in &segments {
                    out.push((Midpoint, place.clone(), curve.point(0.5)));
                }
                // One centre and the middle circle's four quadrants.
                let c = p3(donut.center);
                let r = (donut.inside_diameter + donut.outside_diameter) / 4.0;
                out.push((Center, String::new(), c));
                for (degrees, [cos, sin]) in QUADRANTS {
                    out.push((
                        Quadrant,
                        degrees.to_string(),
                        [c[0] + r * cos, c[1] + r * sin, c[2]],
                    ));
                }
            }
            Object::Arc(_) => {
                for (_, curve) in self.curves(object) {
                    out.push((Endpoint, "start".into(), curve.start()));
                    out.push((Endpoint, "end".into(), curve.end()));
                    out.push((Midpoint, String::new(), curve.point(curve.middle())));
                    round(&mut out, "", &curve);
                }
            }
            Object::Circle(_) => {
                for (_, curve) in self.curves(object) {
                    round(&mut out, "", &curve);
                }
            }
            Object::Ellipse(e) => {
                for (_, curve) in self.curves(object) {
                    let Curve::Ellipse {
                        center,
                        major,
                        minor,
                        start,
                        sweep,
                    } = curve
                    else {
                        continue;
                    };
                    if !e.is_whole() {
                        out.push((Endpoint, "start".into(), curve.start()));
                        out.push((Endpoint, "end".into(), curve.end()));
                        out.push((Midpoint, String::new(), curve.point(curve.middle())));
                    }
                    out.push((Center, String::new(), center));
                    // The ends of its axes, by their angle from the major.
                    for (degrees, [cos, sin]) in QUADRANTS {
                        let s = f64::from(degrees).to_radians();
                        if e.is_whole() || on_arc(s, start, sweep) {
                            out.push((
                                Quadrant,
                                degrees.to_string(),
                                [
                                    center[0] + cos * major[0] + sin * minor[0],
                                    center[1] + cos * major[1] + sin * minor[1],
                                    center[2],
                                ],
                            ));
                        }
                    }
                }
            }
            Object::Spline(spline) => {
                let (key, points) = if spline.fit_points.is_empty() {
                    ("control_points", &spline.control_points)
                } else {
                    ("fit_points", &spline.fit_points)
                };
                if let (Some(first), Some(last)) = (points.first(), points.last()) {
                    out.push((Endpoint, format!("{key}[0]"), p3(*first)));
                    out.push((Endpoint, format!("{key}[{}]", points.len() - 1), p3(*last)));
                }
                for (place, curve) in self.curves(object) {
                    out.push((Midpoint, place, curve.point(curve.middle())));
                }
            }
            Object::Solid(solid) => {
                for (name, p) in named("corners", &solid.corners) {
                    out.push((Endpoint, name, p));
                }
                for (place, curve) in self.curves(object) {
                    out.push((Midpoint, place, curve.point(0.5)));
                }
            }
            // A clipped viewport's are its boundary's, as a closed
            // polyline's or a circle's.
            Object::Viewport(viewport) if viewport.clip.is_some() => {
                out.extend(self.local_snaps(&clip_boundary(viewport)));
            }
            // A viewport's frame's corners, from its lower left corner
            // anticlockwise, the middles of its sides and its middle.
            Object::Viewport(viewport) => {
                for (name, p) in named("corners", &viewport.frame()) {
                    out.push((Endpoint, name, p));
                }
                for (place, curve) in self.curves(object) {
                    out.push((Midpoint, place, curve.point(0.5)));
                }
                out.push((Center, String::new(), p3(viewport.center)));
            }
            Object::Text(text) => {
                for (kind, at) in self.text_snaps(text) {
                    out.push((kind, String::new(), at));
                }
            }
            Object::AttributeDefinition(definition) => {
                let text = definition_text(definition);
                for (kind, at) in self.text_snaps(&text) {
                    out.push((kind, String::new(), at));
                }
            }
            Object::MText(text) => {
                out.push((Insertion, String::new(), p3(text.position)));
            }
            Object::Table(_) => {
                if let Object::Table(table) = object {
                    out.push((Insertion, String::new(), p3(table.position)));
                }
                for (place, curve) in self.curves(object) {
                    out.push((Endpoint, join(&place, "from"), curve.start()));
                    out.push((Endpoint, join(&place, "to"), curve.end()));
                    out.push((Midpoint, place, curve.point(0.5)));
                }
            }
            Object::Leader(leader) => {
                for (name, p) in named("points", &leader.points) {
                    out.push((Endpoint, name, p));
                }
                if leader.hooked() {
                    let path = leader.path();
                    out.push((Endpoint, "hook".into(), p3(path[path.len() - 2])));
                }
                for (place, curve) in self.curves(object) {
                    out.push((Midpoint, place, curve.point(0.5)));
                }
                if let Some(text) = leader.annotation_text() {
                    out.push((Insertion, String::new(), p3(text.position)));
                }
            }
            Object::MLeader(mleader) => {
                let style = self
                    .sheet
                    .mleader_style(&mleader.mleader_style)
                    .unwrap_or_else(|| MLeaderStyle::standard(self.sheet.units));
                let fonts = self.fonts;
                let laid = mleader.laid_out(&style, &|t| mtext_box(t, fonts), &self.sheet.blocks);
                for (i, line) in laid.lines.iter().enumerate() {
                    for (j, &p) in line.iter().enumerate() {
                        out.push((Endpoint, format!("leaders[{i}][{j}]"), p3(p)));
                    }
                }
                if laid.landing > 0.0 {
                    out.push((Endpoint, "landing".into(), p3(laid.landing_end())));
                }
                let content = match &laid.content {
                    crate::model::LaidContent::Text { text, .. } => text.position,
                    crate::model::LaidContent::Block { insert, .. } => insert.position,
                };
                out.push((Insertion, String::new(), p3(content)));
            }
            Object::Dimension(dimension) => {
                // Its lines' and arcs' ends and middles, and an arc's centre
                // and quadrants, as AutoCAD snaps to the objects of its block.
                for (place, curve) in self.curves(object) {
                    out.push((Endpoint, join(&place, "from"), curve.start()));
                    out.push((Endpoint, join(&place, "to"), curve.end()));
                    out.push((Midpoint, place.clone(), curve.point(0.5)));
                    round(&mut out, &place, &curve);
                }
                if let Some(laid) = self.laid_dimension(dimension) {
                    // AutoCAD's node snap finds a dimension's definition
                    // points, and its insertion snap the middle of its text.
                    for (i, p) in laid.defpoints.iter().enumerate() {
                        out.push((Node, format!("defpoints[{i}]"), p3(*p)));
                    }
                    out.push((Insertion, String::new(), p3(laid.text.position)));
                }
            }
            Object::Hatch(_) | Object::Insert(_) => {}
        }
        out
    }

    /// A dimension laid out with its style and the sheet's fonts; none for
    /// a style the sheet does not have.
    pub(crate) fn laid_dimension(&self, dimension: &Dimension) -> Option<LaidDimension> {
        let style = self.sheet.dimension_style(&dimension.dimension_style)?;
        let fonts = self.fonts;
        Some(dimension.laid_out(&style, self.sheet.units, &|t| mtext_box(t, fonts)))
    }
}

/// The quadrants by their angle in degrees, with its cosine and sine.
const QUADRANTS: [(u16, [f64; 2]); 4] = [
    (0, [1.0, 0.0]),
    (90, [0.0, 1.0]),
    (180, [-1.0, 0.0]),
    (270, [0.0, -1.0]),
];

/// Whether `angle` lies on the arc from `start` sweeping `sweep`, within a
/// little.
fn on_arc(angle: f64, start: f64, sweep: f64) -> bool {
    let slack = 1e-12;
    let s = if sweep >= 0.0 {
        (angle - start).rem_euclid(TAU)
    } else {
        (start - angle).rem_euclid(TAU)
    };
    s <= sweep.abs() + slack || TAU - s <= slack
}
