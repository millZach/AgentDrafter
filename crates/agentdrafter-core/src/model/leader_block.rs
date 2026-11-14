//! Where AutoCAD puts a block that is a multileader's content.
//!
//! AutoCAD 2026 made its own multileaders with block content, on both
//! sides and with both of its connections, scaled and turned, and placed
//! each block by one rule, which this module keeps:
//!
//! - The block's attachment point is the centre of its own extents, for
//!   the `extents` connection, AutoCAD's default, or its base point, for
//!   `insertion`, placed with the content's scale and rotation.
//! - The block sits with that point level with the landing, slid along
//!   the landing's line until the first place where the line through the
//!   point crosses the block's outline, from the landing's side, is at the
//!   landing's end. When the line crosses nothing, the point itself is at
//!   the landing's end.
//! - The outline is what the block draws, in its own coordinates and
//!   those of the blocks it inserts: lines, polylines along their middle,
//!   whatever their width, arcs, circles, ellipses, 2D solids and donuts.
//!   A line that runs along the landing's line does not cross it. A
//!   hatch counts towards the extents but is not crossed; an attribute
//!   definition counts towards neither.
//!
//! What AutoCAD records as the content's base point is the landing's end,
//! on the right, and on the left the far crossing: the landing's end less
//! the span between the first crossing and the last.
//!
//! Text, multiline text, a constant attribute and a nested block's
//! attributes draw letters, and where the landing meets letters depends on
//! the font's outline; AutoCAD's extents of a spline reach past its curve.
//! A block that holds any of them is not offered as content, since its
//! place could not be AutoCAD's exactly.

use std::f64::consts::TAU;

use super::{Block, HatchEdge, HatchLoop, Object, Point3, Polyline};

/// A piece of a block's outline, in the block's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    Segment([f64; 2], [f64; 2]),
    /// An arc from `start`, in radians, through `sweep`, anticlockwise
    /// when positive.
    Arc {
        center: [f64; 2],
        radius: f64,
        start: f64,
        sweep: f64,
    },
    /// An elliptical arc: the point at parameter `t` is `center + cos t ·
    /// major + sin t · minor`, from `start` through `sweep`.
    Ellipse {
        center: [f64; 2],
        major: [f64; 2],
        minor: [f64; 2],
        start: f64,
        sweep: f64,
    },
}

/// Where a nested block's points land in the outermost block's
/// coordinates: moved, scaled uniformly and turned.
#[derive(Clone, Copy, Debug)]
struct Frame {
    origin: [f64; 2],
    scale: f64,
    cos: f64,
    sin: f64,
    base: [f64; 2],
}

impl Frame {
    const OWN: Frame = Frame {
        origin: [0.0, 0.0],
        scale: 1.0,
        cos: 1.0,
        sin: 0.0,
        base: [0.0, 0.0],
    };

    fn point(&self, p: [f64; 2]) -> [f64; 2] {
        let x = (p[0] - self.base[0]) * self.scale;
        let y = (p[1] - self.base[1]) * self.scale;
        [
            self.origin[0] + x * self.cos - y * self.sin,
            self.origin[1] + x * self.sin + y * self.cos,
        ]
    }

    fn vector(&self, v: [f64; 2]) -> [f64; 2] {
        let (x, y) = (v[0] * self.scale, v[1] * self.scale);
        [x * self.cos - y * self.sin, x * self.sin + y * self.cos]
    }

    fn angle(&self) -> f64 {
        self.sin.atan2(self.cos)
    }

    /// The frame of an insert at `position`, `scale` and `degrees`, of a
    /// block whose base point is `base`, placed in this frame.
    fn within(&self, position: Point3, scale: f64, degrees: f64, base: Point3) -> Frame {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let origin = self.point([position.x, position.y]);
        let (s, c) = (
            self.sin * cos + self.cos * sin,
            self.cos * cos - self.sin * sin,
        );
        Frame {
            origin,
            scale: self.scale * scale,
            cos: c,
            sin: s,
            base: [base.x, base.y],
        }
    }

    fn piece(&self, piece: Piece) -> Piece {
        match piece {
            Piece::Segment(a, b) => Piece::Segment(self.point(a), self.point(b)),
            Piece::Arc {
                center,
                radius,
                start,
                sweep,
            } => Piece::Arc {
                center: self.point(center),
                radius: radius * self.scale,
                start: start + self.angle(),
                sweep,
            },
            Piece::Ellipse {
                center,
                major,
                minor,
                start,
                sweep,
            } => Piece::Ellipse {
                center: self.point(center),
                major: self.vector(major),
                minor: self.vector(minor),
                start,
                sweep,
            },
        }
    }
}

/// A block's outline, as AutoCAD places block content by it: each piece,
/// and whether the landing's line crosses it or it counts towards the
/// extents alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pieces: Vec<(Piece, bool)>,
}

/// Why a block cannot be a multileader's content: what it holds that
/// AutoCAD places it by and the model cannot, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotContent {
    /// The block that holds it: the content's block or one nested in it.
    pub block: String,
    /// Its place in that block's objects.
    pub index: usize,
    /// What it is, in words, such as "a text".
    pub what: String,
}

impl std::fmt::Display for NotContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "block `{}` holds {} at `objects[{}]`",
            self.block, self.what, self.index
        )
    }
}

/// The parameter of an ellipse's point at `degrees` from its major axis.
fn parameter(degrees: f64, ratio: f64) -> f64 {
    let theta = degrees.to_radians();
    let t = (theta.sin()).atan2(ratio * theta.cos());
    // Keep it on the same turn as the angle.
    let turns = ((theta - t) / TAU).round();
    t + turns * TAU
}

/// The arc a polyline's segment from `a` to `b` with `bulge` draws, or the
/// straight segment.
fn bulged(a: [f64; 2], b: [f64; 2], bulge: f64) -> Piece {
    if bulge == 0.0 {
        return Piece::Segment(a, b);
    }
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let chord = dx.hypot(dy);
    let theta = 4.0 * bulge.atan();
    // The centre lies along the chord's left normal from its middle.
    let h = chord / 2.0 * (1.0 - bulge * bulge) / (2.0 * bulge);
    let (nx, ny) = (-dy / chord, dx / chord);
    let center = [(a[0] + b[0]) / 2.0 + nx * h, (a[1] + b[1]) / 2.0 + ny * h];
    let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
    Piece::Arc {
        center,
        radius,
        start: (a[1] - center[1]).atan2(a[0] - center[0]),
        sweep: theta,
    }
}

fn flat(p: Point3) -> [f64; 2] {
    [p.x, p.y]
}

fn polyline_pieces(polyline: &Polyline) -> Vec<Piece> {
    let n = polyline.points.len();
    let count = if polyline.closed {
        n
    } else {
        n.saturating_sub(1)
    };
    (0..count)
        .map(|i| {
            let a = flat(polyline.points[i]);
            let b = flat(polyline.points[(i + 1) % n]);
            bulged(a, b, polyline.bulge(i))
        })
        .collect()
}

fn degrees_arc(center: Point3, radius: f64, start: f64, end: f64) -> Piece {
    let sweep = (end - start).rem_euclid(360.0);
    Piece::Arc {
        center: flat(center),
        radius,
        start: start.to_radians(),
        sweep: if sweep == 0.0 {
            TAU
        } else {
            sweep.to_radians()
        },
    }
}

fn whole_circle(center: Point3, radius: f64) -> Piece {
    Piece::Arc {
        center: flat(center),
        radius,
        start: 0.0,
        sweep: TAU,
    }
}

impl Outline {
    /// The outline of `block`, whose nested blocks are among `blocks`, or
    /// what it holds that keeps it from being content.
    pub fn of(block: &Block, blocks: &[Block]) -> Result<Outline, NotContent> {
        let mut outline = Outline::default();
        outline.add(block, blocks, Frame::OWN, true, 0)?;
        Ok(outline)
    }

    fn add(
        &mut self,
        block: &Block,
        blocks: &[Block],
        frame: Frame,
        top: bool,
        depth: usize,
    ) -> Result<(), NotContent> {
        let refuse = |index: usize, what: &str| NotContent {
            block: block.name.clone(),
            index,
            what: what.to_owned(),
        };
        for (index, object) in block.objects.iter().enumerate() {
            let mut crossed = |pieces: Vec<Piece>, crosses: bool| {
                for piece in pieces {
                    self.pieces.push((frame.piece(piece), crosses));
                }
            };
            match object {
                Object::Line(l) => crossed(vec![Piece::Segment(flat(l.from), flat(l.to))], true),
                Object::Polyline(p) => crossed(polyline_pieces(p), true),
                Object::Donut(d) => crossed(
                    vec![whole_circle(
                        d.center,
                        (d.inside_diameter + d.outside_diameter) / 4.0,
                    )],
                    true,
                ),
                Object::Arc(a) => crossed(
                    vec![degrees_arc(a.center, a.radius, a.start_angle, a.end_angle)],
                    true,
                ),
                Object::Circle(c) => crossed(vec![whole_circle(c.center, c.radius)], true),
                Object::Ellipse(e) => {
                    let major = [e.major_axis.x, e.major_axis.y];
                    let minor = [-major[1] * e.ratio, major[0] * e.ratio];
                    let (start, sweep) = if e.is_whole() {
                        (0.0, TAU)
                    } else {
                        let start = parameter(e.start_angle, e.ratio);
                        let end = parameter(e.end_angle, e.ratio);
                        let sweep = (end - start).rem_euclid(TAU);
                        (start, if sweep == 0.0 { TAU } else { sweep })
                    };
                    crossed(
                        vec![Piece::Ellipse {
                            center: flat(e.center),
                            major,
                            minor,
                            start,
                            sweep,
                        }],
                        true,
                    );
                }
                Object::Solid(s) => {
                    let n = s.corners.len();
                    let edges = (0..n)
                        .map(|i| Piece::Segment(flat(s.corners[i]), flat(s.corners[(i + 1) % n])))
                        .collect();
                    crossed(edges, true);
                }
                Object::Hatch(h) => {
                    let mut edges = Vec::new();
                    for boundary in &h.boundary {
                        match boundary {
                            HatchLoop::Polyline { points } => {
                                let n = points.len();
                                edges.extend((0..n).map(|i| {
                                    Piece::Segment(flat(points[i]), flat(points[(i + 1) % n]))
                                }));
                            }
                            HatchLoop::Edges(list) => {
                                edges.extend(list.iter().map(|edge| match *edge {
                                    HatchEdge::Line { from, to } => {
                                        Piece::Segment(flat(from), flat(to))
                                    }
                                    HatchEdge::Arc {
                                        center,
                                        radius,
                                        start_angle,
                                        end_angle,
                                    } => degrees_arc(center, radius, start_angle, end_angle),
                                }));
                            }
                        }
                    }
                    crossed(edges, false);
                }
                // An attribute the content carries draws its value, which
                // AutoCAD neither crosses nor takes into the extents.
                Object::AttributeDefinition(d) if top && !d.constant => {}
                Object::AttributeDefinition(d) if d.constant => {
                    return Err(refuse(index, "a constant attribute, which draws letters"));
                }
                Object::AttributeDefinition(_) => {
                    return Err(refuse(
                        index,
                        "an attribute in a block nested in the content's, which draws letters",
                    ));
                }
                Object::Insert(inner) => {
                    let Some(nested) = blocks.iter().find(|b| b.name == inner.block) else {
                        return Err(refuse(
                            index,
                            "an insert of a block the sheet does not hold",
                        ));
                    };
                    if depth > blocks.len() {
                        return Err(refuse(index, "an insert of itself"));
                    }
                    let within = frame.within(
                        inner.position,
                        inner.scale,
                        inner.rotation,
                        nested.base_point,
                    );
                    self.add(nested, blocks, within, false, depth + 1)?;
                }
                Object::Text(_) => return Err(refuse(index, "a text, which draws letters")),
                Object::MText(_) => {
                    return Err(refuse(index, "a multiline text, which draws letters"))
                }
                Object::Spline(_) => {
                    return Err(refuse(
                        index,
                        "a spline, whose extents AutoCAD takes past its curve",
                    ))
                }
                Object::Table(_) | Object::MLeader(_) | Object::Leader(_) => {
                    return Err(refuse(index, "an object that belongs on a sheet"))
                }
            }
        }
        Ok(())
    }

    /// The box round every piece, `[min, max]`, or none for an empty
    /// outline.
    fn extents(&self) -> Option<[[f64; 2]; 2]> {
        let mut points = Vec::new();
        for (piece, _) in &self.pieces {
            match *piece {
                Piece::Segment(a, b) => points.extend([a, b]),
                Piece::Arc {
                    center,
                    radius,
                    start,
                    sweep,
                } => {
                    let at = |t: f64| [center[0] + radius * t.cos(), center[1] + radius * t.sin()];
                    points.extend([at(start), at(start + sweep)]);
                    for quarter in 0..4 {
                        let t = f64::from(quarter) * TAU / 4.0;
                        if within_sweep(t, start, sweep) {
                            points.push(at(t));
                        }
                    }
                }
                Piece::Ellipse {
                    center,
                    major,
                    minor,
                    start,
                    sweep,
                } => {
                    let at = |t: f64| {
                        [
                            center[0] + t.cos() * major[0] + t.sin() * minor[0],
                            center[1] + t.cos() * major[1] + t.sin() * minor[1],
                        ]
                    };
                    points.extend([at(start), at(start + sweep)]);
                    let x = minor[0].atan2(major[0]);
                    let y = minor[1].atan2(major[1]);
                    for t in [x, x + TAU / 2.0, y, y + TAU / 2.0] {
                        if within_sweep(t, start, sweep) {
                            points.push(at(t));
                        }
                    }
                }
            }
        }
        let first = *points.first()?;
        let mut b = [first, first];
        for p in points {
            b[0] = [b[0][0].min(p[0]), b[0][1].min(p[1])];
            b[1] = [b[1][0].max(p[0]), b[1][1].max(p[1])];
        }
        Some(b)
    }

    /// Where the line through `from` along `along`, a unit vector, crosses
    /// the outline: how far along it each crossing is.
    fn crossings(&self, from: [f64; 2], along: [f64; 2]) -> Vec<f64> {
        let mut found = Vec::new();
        for (piece, crosses) in &self.pieces {
            if !crosses {
                continue;
            }
            match *piece {
                Piece::Segment(a, b) => {
                    let e = [b[0] - a[0], b[1] - a[1]];
                    let length = e[0].hypot(e[1]);
                    let cross = along[0] * e[1] - along[1] * e[0];
                    // Along the line, or a point: no crossing.
                    if length == 0.0 || cross.abs() <= 1e-12 * length {
                        continue;
                    }
                    let w = [a[0] - from[0], a[1] - from[1]];
                    let t = (w[0] * e[1] - w[1] * e[0]) / cross;
                    let u = (w[0] * along[1] - w[1] * along[0]) / cross;
                    if (-1e-12..=1.0 + 1e-12).contains(&u) {
                        found.push(t);
                    }
                }
                Piece::Arc {
                    center,
                    radius,
                    start,
                    sweep,
                } => {
                    let w = [from[0] - center[0], from[1] - center[1]];
                    let half_b = along[0] * w[0] + along[1] * w[1];
                    let c = w[0] * w[0] + w[1] * w[1] - radius * radius;
                    let disc = half_b * half_b - c;
                    if disc < 0.0 {
                        continue;
                    }
                    let root = disc.sqrt();
                    for t in [-half_b - root, -half_b + root] {
                        let p = [w[0] + t * along[0], w[1] + t * along[1]];
                        if within_sweep(p[1].atan2(p[0]), start, sweep) {
                            found.push(t);
                        }
                    }
                }
                Piece::Ellipse {
                    center,
                    major,
                    minor,
                    start,
                    sweep,
                } => {
                    // In the ellipse's own frame it is the unit circle.
                    let m2 = major[0] * major[0] + major[1] * major[1];
                    let n2 = minor[0] * minor[0] + minor[1] * minor[1];
                    let coords = |v: [f64; 2]| {
                        [
                            (v[0] * major[0] + v[1] * major[1]) / m2,
                            (v[0] * minor[0] + v[1] * minor[1]) / n2,
                        ]
                    };
                    let w = coords([from[0] - center[0], from[1] - center[1]]);
                    let d = coords(along);
                    let a = d[0] * d[0] + d[1] * d[1];
                    let half_b = d[0] * w[0] + d[1] * w[1];
                    let c = w[0] * w[0] + w[1] * w[1] - 1.0;
                    let disc = half_b * half_b - a * c;
                    if a == 0.0 || disc < 0.0 {
                        continue;
                    }
                    let root = disc.sqrt();
                    for t in [(-half_b - root) / a, (-half_b + root) / a] {
                        let p = [w[0] + t * d[0], w[1] + t * d[1]];
                        if within_sweep(p[1].atan2(p[0]), start, sweep) {
                            found.push(t);
                        }
                    }
                }
            }
        }
        found
    }
}

/// Whether the angle `t` lies on the arc from `start` through `sweep`.
fn within_sweep(t: f64, start: f64, sweep: f64) -> bool {
    if sweep.abs() >= TAU {
        return true;
    }
    let tolerance = 1e-12;
    let along = if sweep >= 0.0 {
        (t - start).rem_euclid(TAU)
    } else {
        (start - t).rem_euclid(TAU)
    };
    along <= sweep.abs() + tolerance || along >= TAU - tolerance
}

/// How AutoCAD connects block content to the landing: by the centre of the
/// block's extents, its default, or by its base point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockConnection {
    #[default]
    Extents,
    Insertion,
}

impl BlockConnection {
    pub const ALL: [BlockConnection; 2] = [BlockConnection::Extents, BlockConnection::Insertion];

    /// The name a sheet description writes.
    pub const fn name(self) -> &'static str {
        match self {
            BlockConnection::Extents => "extents",
            BlockConnection::Insertion => "insertion",
        }
    }
}

/// Where block content sits: its block's base point, and the point
/// AutoCAD records as the content's base.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: Point3,
    pub base: Point3,
}

/// Places `block`, with `outline`, scaled by `scale` and turned by
/// `degrees`, connected by `connection` at `landing_end`, the landing
/// running to `side`: 1 for the right, -1 for the left.
pub fn place(
    block: &Block,
    outline: &Outline,
    connection: BlockConnection,
    scale: f64,
    degrees: f64,
    landing_end: Point3,
    side: f64,
) -> Placed {
    let base = [block.base_point.x, block.base_point.y];
    let attach = match (connection, outline.extents()) {
        (BlockConnection::Extents, Some([min, max])) => {
            [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]
        }
        _ => base,
    };
    let (sin, cos) = degrees.to_radians().sin_cos();
    // The landing's line, in the block's own coordinates.
    let along = [cos, -sin];
    let crossings = outline.crossings(attach, along);
    let first = crossings.iter().copied().fold(f64::INFINITY, f64::min);
    let last = crossings.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (first, last) = if crossings.is_empty() {
        (0.0, 0.0)
    } else {
        (first, last)
    };
    let near = if side > 0.0 { first } else { last };
    let attach_at = [landing_end.x - scale * near, landing_end.y];
    let v = [(attach[0] - base[0]) * scale, (attach[1] - base[1]) * scale];
    let position = Point3 {
        x: attach_at[0] - (v[0] * cos - v[1] * sin),
        y: attach_at[1] - (v[0] * sin + v[1] * cos),
        z: landing_end.z,
    };
    let content_base = if side > 0.0 {
        landing_end
    } else {
        Point3 {
            x: landing_end.x - scale * (last - first),
            ..landing_end
        }
    };
    Placed {
        position,
        base: content_base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AttributeDefinition, Circle, Insert, Line, Properties};
    use crate::text::{Justify, STANDARD};

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    fn block(name: &str, objects: Vec<Object>) -> Block {
        Block {
            name: name.to_owned(),
            base_point: p(0.0, 0.0),
            objects,
            cells: Vec::new(),
        }
    }

    fn rectangle() -> Object {
        Object::Polyline(Polyline::through(
            vec![p(5.0, -3.0), p(25.0, -3.0), p(25.0, 9.0), p(5.0, 9.0)],
            true,
            "0",
        ))
    }

    fn circle(x: f64, y: f64, r: f64) -> Object {
        Object::Circle(Circle {
            center: p(x, y),
            radius: r,
            layer: "0".into(),
            properties: Properties::default(),
        })
    }

    fn attdef(tag: &str, at: Point3, constant: bool) -> Object {
        Object::AttributeDefinition(AttributeDefinition {
            tag: tag.into(),
            prompt: String::new(),
            default: String::new(),
            position: at,
            height: 1.5,
            style: STANDARD.into(),
            width_factor: 1.0,
            rotation: 0.0,
            justify: Justify::Left,
            invisible: false,
            constant,
            verify: false,
            preset: false,
            lock_position: false,
            layer: "0".into(),
            properties: Properties::default(),
        })
    }

    fn close(a: Point3, x: f64, y: f64) {
        assert!(
            (a.x - x).abs() < 1e-3 && (a.y - y).abs() < 1e-3,
            "{a:?} against AutoCAD's ({x}, {y})"
        );
    }

    /// AutoCAD 2026's own multileaders, made with MLEADER on made-up
    /// blocks, their landings 8 long: where it put each block.
    #[test]
    fn block_content_sits_where_autocad_put_it() {
        let boxa = block(
            "BOXA",
            vec![rectangle(), attdef("TAG", p(15.0, 3.0), false)],
        );
        let circ = block("CIRC", vec![circle(30.0, 20.0, 6.0)]);
        let blocks = [boxa.clone(), circ.clone()];
        // The block, its connection, scale and rotation, the landing's end
        // and side, and where AutoCAD put the block's base point and the
        // content's base.
        type Case<'a> = (
            &'a Block,
            BlockConnection,
            f64,
            f64,
            Point3,
            f64,
            [f64; 2],
            f64,
        );
        let cases: [Case; 12] = [
            (
                &boxa,
                BlockConnection::Extents,
                1.0,
                0.0,
                p(138.0, 20.0),
                1.0,
                [133.0, 17.0],
                138.0,
            ),
            (
                &boxa,
                BlockConnection::Extents,
                1.0,
                0.0,
                p(62.0, -40.0),
                -1.0,
                [37.0, -43.0],
                42.0,
            ),
            (
                &circ,
                BlockConnection::Extents,
                1.0,
                0.0,
                p(338.0, 20.0),
                1.0,
                [314.0, 0.0],
                338.0,
            ),
            (
                &circ,
                BlockConnection::Extents,
                1.0,
                0.0,
                p(262.0, -40.0),
                -1.0,
                [226.0, -60.0],
                250.0,
            ),
            (
                &boxa,
                BlockConnection::Insertion,
                1.0,
                0.0,
                p(138.0, -180.0),
                1.0,
                [133.0, -180.0],
                138.0,
            ),
            (
                &boxa,
                BlockConnection::Insertion,
                1.0,
                0.0,
                p(62.0, -240.0),
                -1.0,
                [37.0, -240.0],
                42.0,
            ),
            (
                &circ,
                BlockConnection::Insertion,
                1.0,
                0.0,
                p(338.0, -180.0),
                1.0,
                [338.0, -180.0],
                338.0,
            ),
            (
                &circ,
                BlockConnection::Insertion,
                1.0,
                0.0,
                p(262.0, -240.0),
                -1.0,
                [262.0, -240.0],
                262.0,
            ),
            (
                &boxa,
                BlockConnection::Extents,
                2.0,
                30.0,
                p(138.0, -380.0),
                1.0,
                [138.113, -400.196],
                138.0,
            ),
            (
                &boxa,
                BlockConnection::Extents,
                2.0,
                30.0,
                p(62.0, -440.0),
                -1.0,
                [15.9252, -460.196],
                15.812,
            ),
            (
                &boxa,
                BlockConnection::Insertion,
                2.0,
                30.0,
                p(138.0, -580.0),
                1.0,
                [126.453, -580.0],
                138.0,
            ),
            (
                &boxa,
                BlockConnection::Insertion,
                2.0,
                30.0,
                p(62.0, -640.0),
                -1.0,
                [50.0, -640.0],
                61.547,
            ),
        ];
        for (b, connection, scale, degrees, end, side, at, base) in cases {
            let outline = Outline::of(b, &blocks).unwrap();
            let placed = place(b, &outline, connection, scale, degrees, end, side);
            close(placed.position, at[0], at[1]);
            close(placed.base, base, end.y);
        }
    }

    /// AutoCAD counted a constant attribute in a block's extents and not
    /// the others: here a block with no constant one, so the line alone.
    #[test]
    fn an_attribute_counts_towards_nothing_and_a_line_along_the_landing_crosses_nothing() {
        let line = Object::Line(Line {
            from: p(0.0, 0.0),
            to: p(20.0, 0.0),
            layer: "0".into(),
            properties: Properties::default(),
        });
        let b = block("COLL", vec![line, attdef("A", p(0.0, 6.0), false)]);
        let outline = Outline::of(&b, &[]).unwrap();
        // As AutoCAD placed it: the extents' centre at the landing's end.
        let placed = place(
            &b,
            &outline,
            BlockConnection::Extents,
            1.0,
            0.0,
            p(450.36, 10.0),
            1.0,
        );
        close(placed.position, 440.36, 10.0);
        let constant = block("K", vec![attdef("C", p(0.0, 4.0), true)]);
        let refused = Outline::of(&constant, &[]).unwrap_err();
        assert_eq!(refused.index, 0);
    }

    #[test]
    fn a_nested_insert_counts_where_it_is_placed() {
        let bar = Object::Line(Line {
            from: p(0.0, -4.0),
            to: p(0.0, 4.0),
            layer: "0".into(),
            properties: Properties::default(),
        });
        let vbar = block("VBAR", vec![bar]);
        let diagonal = Object::Line(Line {
            from: p(0.0, -10.0),
            to: p(20.0, 10.0),
            layer: "0".into(),
            properties: Properties::default(),
        });
        let insert = Object::Insert(Insert {
            block: "VBAR".into(),
            position: p(32.0, 14.0),
            scale: 1.0,
            rotation: 0.0,
            attributes: Vec::new(),
            name: None,
            layer: "0".into(),
            properties: Properties::default(),
        });
        let b = block("E_INS", vec![diagonal, insert]);
        let blocks = [vbar, b.clone()];
        let outline = Outline::of(&b, &blocks).unwrap();
        // AutoCAD 2026: its extents reach 18 up, so the line through their
        // centre crosses the diagonal at 14.
        let placed = place(
            &b,
            &outline,
            BlockConnection::Extents,
            1.0,
            0.0,
            p(250.36, 10.0),
            1.0,
        );
        close(placed.position, 236.36, 6.0);
    }
}
