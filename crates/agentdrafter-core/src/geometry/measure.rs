//! An object's length, area, extents and angles, as AutoCAD's LIST, AREA
//! and MEASUREGEOM give them, from its curves exactly.
//!
//! A polyline's length and area are measured along its middle, whatever
//! its width, as AutoCAD's AREA measures a wide polyline, and its extents
//! take in its width, as AutoCAD's do; a donut is the polyline AutoCAD
//! draws it as, so its length and area are its middle circle's and its
//! extents its outside circle's. A hatch's area is what it fills: a loop
//! inside an odd number of others is a hole.

use serde::Serialize;

use std::f64::consts::TAU;

use super::curve::{length_of, sub, Curve, P3};
use super::{p3, Reader};
use crate::model::hatch::walk;
use crate::model::{HatchEdge, HatchLoop, Object, Polyline};

/// The box round an object's geometry, in space: its least and greatest
/// corner, `[x, y, z]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Box3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// What an object measures, each in the declared unit or in degrees; none
/// of what it has none of.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Measures {
    /// How long it is: a line's, a polyline's along its segments and arcs,
    /// an arc's, a circle's circumference, an ellipse's or an elliptical
    /// arc's and a spline's, in space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// The area it encloses: a circle's, an ellipse's, a closed polyline's
    /// with its arcs, a closed spline's, a hatch's, a solid's and a
    /// donut's, seen from above; or a 3D solid's total surface area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    /// Volume of a 3D solid (`3DSOLID`), in cubic declared units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    /// Total surface area of a 3D solid (`3DSOLID`), in square declared units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_area: Option<f64>,
    /// The box round its geometry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extents: Option<Box3>,
    /// A line's angle in the XY plane: degrees anticlockwise from the X
    /// axis, in `[0, 360)`, as AutoCAD's LIST gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    /// An arc's start angle, in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_angle: Option<f64>,
    /// An arc's end angle, in degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_angle: Option<f64>,
    /// The angle an arc sweeps, in degrees, AutoCAD's total angle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_angle: Option<f64>,
    /// What a dimension measures, as AutoCAD keeps it in the dimension: a
    /// linear dimension's distance along its angle, an aligned one's
    /// between its points, a radius, a diameter or an ordinate's X or Y
    /// from the origin, in the declared unit; an angular one's angle, in
    /// degrees, which AutoCAD keeps in radians; unrounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement: Option<f64>,
    /// What a dimension's text shows: its measured value as its style
    /// formats it, or its own text with `<>` standing for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Measures {
    /// The measures it has none of, in the order a reply gives them.
    pub(crate) fn without(&self, object: &Object) -> Vec<&'static str> {
        let mut none = Vec::new();
        if self.length.is_none() {
            none.push("length");
        }
        if self.area.is_none() {
            none.push("area");
        }
        if self.extents.is_none() {
            none.push("extents");
        }
        match object {
            Object::Line(_) => {}
            Object::Arc(_) => {}
            _ => none.push("angle"),
        }
        none
    }
}

/// The union of curves' boxes.
fn extents_of<'a>(curves: impl Iterator<Item = &'a Curve>) -> Option<Box3> {
    let mut b: Option<Box3> = None;
    for c in curves {
        let (min, max) = c.extents();
        b = Some(match b {
            None => Box3 { min, max },
            Some(b) => Box3 {
                min: [0, 1, 2].map(|i| b.min[i].min(min[i])),
                max: [0, 1, 2].map(|i| b.max[i].max(max[i])),
            },
        });
    }
    b
}

/// The box round a polyline along its middle: its points, exactly, and
/// where its arcs turn between them.
fn polyline_extents(polyline: &Polyline) -> Option<Box3> {
    let n = polyline.points.len();
    let boxes = (0..n).filter_map(|i| {
        let curve = super::polyline_segment(polyline, i)?;
        let (a, b) = (polyline.points[i], polyline.points[(i + 1) % n]);
        Some(curve.extents_from(p3(a), p3(b)))
    });
    let single = (n == 1).then(|| (p3(polyline.points[0]), p3(polyline.points[0])));
    boxes
        .chain(single)
        .fold(None, |b: Option<Box3>, (min, max)| {
            Some(match b {
                None => Box3 { min, max },
                Some(b) => Box3 {
                    min: [0, 1, 2].map(|i| b.min[i].min(min[i])),
                    max: [0, 1, 2].map(|i| b.max[i].max(max[i])),
                },
            })
        })
}

/// The box round a flat polyline that has a width: round the band it
/// fills, its arcs drawn as chords within a billionth of its size. None
/// for a polyline with no width.
fn band_extents(polyline: &Polyline) -> Option<Box3> {
    let n = polyline.points.len();
    if !polyline.is_flat() || !(0..n).any(|i| polyline.widths_at(i).iter().any(|&w| w > 0.0)) {
        return None;
    }
    let points: Vec<[f64; 2]> = polyline.points.iter().map(|p| [p.x, p.y]).collect();
    let size = points.iter().flatten().fold(1.0_f64, |m, v| m.max(v.abs()));
    let drawn = crate::draw::polyline::draw(
        &points,
        polyline.closed,
        |i| polyline.bulge(i),
        |i| polyline.widths_at(i),
        None,
        size * 1e-9,
    );
    let z = polyline.points[0].z;
    let mut all = drawn.fills.iter().chain(&drawn.thin).flatten().peekable();
    all.peek()?;
    let (mut min, mut max) = (
        [f64::INFINITY, f64::INFINITY, z],
        [f64::NEG_INFINITY, f64::NEG_INFINITY, z],
    );
    for p in all {
        for i in 0..2 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    Some(Box3 { min, max })
}

/// The area a closed run of curves encloses, seen from above.
fn enclosed(curves: &[Curve]) -> f64 {
    curves.iter().map(Curve::green).sum::<f64>().abs()
}

/// A hatch loop as the curves it runs along, in the order it walks them.
fn hatch_loop(l: &HatchLoop) -> Vec<Curve> {
    match l {
        HatchLoop::Polyline { points } => {
            let points: Vec<P3> = points.iter().map(|&p| p3(p)).collect();
            let n = points.len();
            (0..n)
                .map(|i| Curve::Segment {
                    a: points[i],
                    b: points[(i + 1) % n],
                })
                .collect()
        }
        HatchLoop::Edges(edges) => {
            let backwards = walk(edges).unwrap_or_else(|_| vec![false; edges.len()]);
            edges
                .iter()
                .zip(backwards)
                .map(|(edge, back)| match *edge {
                    HatchEdge::Line { from, to } => {
                        let (a, b) = if back { (to, from) } else { (from, to) };
                        Curve::Segment { a: p3(a), b: p3(b) }
                    }
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                    } => {
                        let sweep = (end_angle - start_angle).rem_euclid(360.0);
                        let sweep = if sweep == 0.0 { 360.0 } else { sweep }.to_radians();
                        let (start, sweep) = if back {
                            (end_angle.to_radians(), -sweep)
                        } else {
                            (start_angle.to_radians(), sweep)
                        };
                        Curve::Arc {
                            center: p3(center),
                            radius,
                            start,
                            sweep,
                        }
                    }
                })
                .collect()
        }
    }
}

/// The angle from `u` to `v`, both from one point, in `(-π, π]`.
fn turn(u: [f64; 2], v: [f64; 2]) -> f64 {
    (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1])
}

/// How far the curve turns about `p`, seen from above, in radians. A
/// segment's is exact. An arc is followed in pieces each so short that `p`
/// lies nowhere between a piece and its chord, so each piece turns as its
/// chord does: the region between them reaches no further from the circle
/// than the piece's sagitta, which is kept below `p`'s distance from it.
fn turn_about(c: &Curve, p: [f64; 2]) -> f64 {
    let from = |q: P3| [q[0] - p[0], q[1] - p[1]];
    let pieces = match c {
        Curve::Segment { .. } => 1,
        Curve::Arc {
            center,
            radius,
            sweep,
            ..
        } => {
            let off = ((p[0] - center[0]).hypot(p[1] - center[1]) - radius).abs();
            let sagitta = (off / 2.0).max(radius * 1e-15);
            let step = 2.0 * (1.0 - (sagitta / radius).min(1.0)).acos();
            (sweep.abs() / step).ceil().clamp(1.0, 1_048_576.0) as usize
        }
        _ => 4096,
    };
    let (a, b) = c.domain();
    let mut total = 0.0;
    let mut last = from(c.point(a));
    for i in 1..=pieces {
        let next = from(c.point(a + (b - a) * i as f64 / pieces as f64));
        total += turn(last, next);
        last = next;
    }
    total
}

/// Whether a closed loop of curves winds round `p`, seen from above, its
/// arcs followed exactly.
fn encloses(curves: &[Curve], p: [f64; 2]) -> bool {
    let total: f64 = curves.iter().map(|c| turn_about(c, p)).sum();
    (total / TAU).round() != 0.0
}

/// The area a hatch fills: each loop's area, less where it is a hole, a
/// loop inside an odd number of others, as AutoCAD's normal island
/// detection fills it. A loop is inside another when a point of it is: the
/// middle of its first curve, tested against the other's own lines and
/// arcs, so a hole all but as large as its loop is still inside it.
fn hatch_area(loops: &[Vec<Curve>]) -> f64 {
    loops
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let Some(first) = l.first() else {
                return 0.0;
            };
            let p = first.point(first.middle());
            let depth = loops
                .iter()
                .enumerate()
                .filter(|&(j, other)| j != i && encloses(other, [p[0], p[1]]))
                .count();
            let area = enclosed(l);
            if depth % 2 == 0 {
                area
            } else {
                -area
            }
        })
        .sum()
}

/// Degrees in `[0, 360)`.
fn degrees(radians: f64) -> f64 {
    let d = radians.to_degrees().rem_euclid(360.0);
    if d >= 360.0 {
        0.0
    } else {
        d
    }
}

impl Reader<'_> {
    /// What an object measures.
    pub(crate) fn measures(&self, object: &Object) -> Measures {
        let curves: Vec<Curve> = self.curves(object).into_iter().map(|(_, c)| c).collect();
        let length = || Some(curves.iter().map(Curve::length).sum());
        let extents = || extents_of(curves.iter());
        let mut m = Measures::default();
        match object {
            Object::Line(line) => {
                m.length = length();
                m.extents = extents();
                let d = sub(p3(line.to), p3(line.from));
                m.angle = Some(if d[0] == 0.0 && d[1] == 0.0 {
                    0.0
                } else {
                    degrees(d[1].atan2(d[0]))
                });
            }
            Object::Polyline(polyline) => {
                m.length = length();
                m.extents = band_extents(polyline).or_else(|| polyline_extents(polyline));
                if polyline.closed {
                    m.area = Some(enclosed(&curves));
                }
            }
            Object::Donut(donut) => {
                m.length = length();
                m.extents = band_extents(&donut.polyline());
                m.area = Some(enclosed(&curves));
            }
            Object::RevisionCloud(cloud) => {
                let polyline = cloud.polyline();
                m.length = length();
                m.extents = band_extents(&polyline).or_else(|| polyline_extents(&polyline));
                m.area = Some(enclosed(&curves));
            }
            Object::Wipeout(_) | Object::RasterImage(_) | Object::PdfUnderlay(_) => {
                m.length = length();
                m.extents = extents();
                m.area = Some(enclosed(&curves));
            }
            Object::Arc(arc) => {
                m.length = length();
                m.extents = extents();
                m.start_angle = Some(arc.start_angle);
                m.end_angle = Some(arc.end_angle);
                let sweep = (arc.end_angle - arc.start_angle).rem_euclid(360.0);
                m.total_angle = Some(sweep);
            }
            Object::Circle(_) => {
                m.length = length();
                m.extents = extents();
                m.area = Some(enclosed(&curves));
            }
            Object::Ellipse(e) => {
                m.length = length();
                m.extents = extents();
                if e.is_whole() {
                    m.area = Some(enclosed(&curves));
                }
            }
            Object::Spline(_) => {
                m.length = length();
                m.extents = extents();
                if let Some(c) = curves.first() {
                    let size = super::curve::scale_of(c);
                    if length_of(sub(c.start(), c.end())) <= 1e-9 * size {
                        m.area = Some(enclosed(&curves));
                    }
                }
            }
            Object::Solid(_) => {
                m.extents = extents();
                m.area = Some(enclosed(&curves));
            }
            Object::Solid3D(solid) => {
                let metrics = super::solid3d::solid_metrics(&solid.body);
                m.volume = Some(metrics.volume);
                m.surface_area = Some(metrics.surface_area);
                m.area = Some(metrics.surface_area);
                m.extents = Some(Box3 {
                    min: metrics.min,
                    max: metrics.max,
                });
            }
            Object::Hatch(hatch) => {
                let loops: Vec<Vec<Curve>> = hatch.boundary.iter().map(hatch_loop).collect();
                m.extents = extents_of(loops.iter().flatten());
                m.area = Some(hatch_area(&loops));
            }
            Object::Dimension(dimension) => {
                m.extents = extents();
                m.measurement = Some(dimension.measurement());
                m.text = self
                    .sheet
                    .dimension_style(&dimension.dimension_style)
                    .map(|style| dimension.displayed_text(&style));
            }
            _ => {}
        }
        m
    }
}
