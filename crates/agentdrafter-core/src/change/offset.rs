//! Geometric offset (`offset` / AutoCAD `OFFSET`) for lines, polylines
//! (including arc segments, with corners joined per AutoCAD `OFFSETGAPTYPE 0`),
//! arcs, circles, ellipses and splines.

use std::f64::consts::{PI, TAU};

use serde_json::{json, Map, Value};

use crate::draw::curves::{ellipse_parameter, spline_definition};
use crate::draw::polyline::arc_of;
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::{plan_distance, BSpline, Curve};
use crate::model::{
    Arc, Circle, Ellipse, Line, Object, Point3, Polyline, Properties, Spline, LAYER_ZERO,
};

const EPS: f64 = 1e-9;

/// Named side or reference point for `offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OffsetSide {
    Left,
    Right,
    Inside,
    Outside,
    Point([f64; 3]),
}

/// How an `offset` operation specifies distance and side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OffsetBy {
    Distance { distance: f64, side: OffsetSide },
    Through([f64; 3]),
}

/// Computes the parallel offset of `object` by `by` in the XY plane (keeping Z).
/// Returns `(new_object, effective_distance)`.
pub(crate) fn compute_offset(
    object: &Object,
    by: &OffsetBy,
    layer_override: Option<&str>,
) -> Result<(Object, f64), Error> {
    let apply_layer = |orig: &str| layer_override.unwrap_or(orig).to_owned();
    match object {
        Object::Line(line) => {
            let (out, dist) = offset_line(line, by, apply_layer(&line.layer))?;
            Ok((Object::Line(out), dist))
        }
        Object::Circle(circle) => {
            let (out, dist) = offset_circle(circle, by, apply_layer(&circle.layer))?;
            Ok((Object::Circle(out), dist))
        }
        Object::Arc(arc) => {
            let (out, dist) = offset_arc(arc, by, apply_layer(&arc.layer))?;
            Ok((Object::Arc(out), dist))
        }
        Object::Polyline(poly) => {
            let (out, dist) = offset_polyline(poly, by, apply_layer(&poly.layer))?;
            Ok((Object::Polyline(out), dist))
        }
        Object::Ellipse(ellipse) => offset_ellipse(ellipse, by, apply_layer(&ellipse.layer)),
        Object::Spline(spline) => {
            let (out, dist) = offset_spline(spline, by, apply_layer(&spline.layer))?;
            Ok((Object::Spline(out), dist))
        }
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`offset` works on a line, polyline, arc, circle, ellipse or spline, not a {}",
                object_kind_name(other)
            ),
        )),
    }
}

fn object_kind_name(object: &Object) -> &'static str {
    match object {
        Object::Line(_) => "line",
        Object::Polyline(_) => "polyline",
        Object::Arc(_) => "arc",
        Object::Circle(_) => "circle",
        Object::Ellipse(_) => "ellipse",
        Object::Spline(_) => "spline",
        Object::Hatch(_) => "hatch",
        Object::Donut(_) => "donut",
        Object::Solid(_) => "solid",
        Object::RevisionCloud(_) => "revision_cloud",
        Object::Wipeout(_) => "wipeout",
        Object::Text(_) => "text",
        Object::MText(_) => "mtext",
        Object::AttributeDefinition(_) => "attdef",
        Object::Insert(_) => "insert",
        Object::Table(_) => "table",
        Object::MLeader(_) => "mleader",
        Object::Leader(_) => "leader",
        Object::Dimension(_) => "dimension",
        Object::Viewport(_) => "viewport",
    }
}

// ---------------------------------------------------------------------------
// Line
// ---------------------------------------------------------------------------

fn offset_line(line: &Line, by: &OffsetBy, layer: String) -> Result<(Line, f64), Error> {
    let dx = line.to.x - line.from.x;
    let dy = line.to.y - line.from.y;
    let len = dx.hypot(dy);
    if len <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot offset a zero-length line in the XY plane".to_owned(),
        ));
    }
    let nx = -dy / len;
    let ny = dx / len;
    let signed = match *by {
        OffsetBy::Distance { distance, side } => {
            let sign = match side {
                OffsetSide::Left => 1.0,
                OffsetSide::Right => -1.0,
                OffsetSide::Inside | OffsetSide::Outside => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a line has no inside or outside; use `left`, `right`, or a point for `side`"
                            .to_owned(),
                    ));
                }
                OffsetSide::Point(p) => {
                    let cross = dx * (p[1] - line.from.y) - dy * (p[0] - line.from.x);
                    if cross.abs() <= EPS * len {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "the `side` point lies on the line; pick a point off the line"
                                .to_owned(),
                        ));
                    }
                    if cross > 0.0 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            sign * distance
        }
        OffsetBy::Through(p) => {
            let s = (p[0] - line.from.x) * nx + (p[1] - line.from.y) * ny;
            if s.abs() <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "the `through` point lies on the line; pick a point off the line".to_owned(),
                ));
            }
            s
        }
    };
    Ok((
        Line {
            from: Point3::new(
                line.from.x + signed * nx,
                line.from.y + signed * ny,
                line.from.z,
            ),
            to: Point3::new(line.to.x + signed * nx, line.to.y + signed * ny, line.to.z),
            layer,
            properties: line.properties.clone(),
        },
        signed.abs(),
    ))
}

// ---------------------------------------------------------------------------
// Circle & Arc
// ---------------------------------------------------------------------------

fn radial_offset(
    center: Point3,
    radius: f64,
    by: &OffsetBy,
    what: &str,
) -> Result<(f64, f64), Error> {
    let new_radius = match *by {
        OffsetBy::Distance { distance, side } => match side {
            OffsetSide::Outside | OffsetSide::Right => radius + distance,
            OffsetSide::Inside | OffsetSide::Left => radius - distance,
            OffsetSide::Point(p) => {
                let d = (p[0] - center.x).hypot(p[1] - center.y);
                if (d - radius).abs() <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("the `side` point lies on the {what}; pick a point inside or outside it"),
                    ));
                }
                if d > radius {
                    radius + distance
                } else {
                    radius - distance
                }
            }
        },
        OffsetBy::Through(p) => {
            let d = (p[0] - center.x).hypot(p[1] - center.y);
            if (d - radius).abs() <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("the `through` point lies on the {what}; pick a point off it"),
                ));
            }
            d
        }
    };
    if new_radius <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "offset leaves the {what} with non-positive radius ({new_radius:.6}); \
                 distance must be less than its radius ({radius})"
            ),
        ));
    }
    Ok((new_radius, (new_radius - radius).abs()))
}

fn offset_circle(circle: &Circle, by: &OffsetBy, layer: String) -> Result<(Circle, f64), Error> {
    let (new_radius, dist) = radial_offset(circle.center, circle.radius, by, "circle")?;
    Ok((
        Circle {
            center: circle.center,
            radius: new_radius,
            layer,
            properties: circle.properties.clone(),
        },
        dist,
    ))
}

fn offset_arc(arc: &Arc, by: &OffsetBy, layer: String) -> Result<(Arc, f64), Error> {
    let (new_radius, dist) = radial_offset(arc.center, arc.radius, by, "arc")?;
    Ok((
        Arc {
            center: arc.center,
            radius: new_radius,
            start_angle: arc.start_angle,
            end_angle: arc.end_angle,
            layer,
            properties: arc.properties.clone(),
        },
        dist,
    ))
}

// ---------------------------------------------------------------------------
// Polyline (OFFSETGAPTYPE = 0: extend adjacent segments to projected intersections)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum PolySeg {
    Line {
        a: [f64; 2],
        b: [f64; 2],
        dir: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

impl PolySeg {
    fn start_pt(&self) -> [f64; 2] {
        match *self {
            Self::Line { a, .. } => a,
            Self::Arc {
                center,
                radius,
                start,
                ..
            } => {
                let (s, c) = start.sin_cos();
                [center[0] + radius * c, center[1] + radius * s]
            }
        }
    }

    fn end_pt(&self) -> [f64; 2] {
        match *self {
            Self::Line { b, .. } => b,
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let (s, c) = (start + sweep).sin_cos();
                [center[0] + radius * c, center[1] + radius * s]
            }
        }
    }

    fn bulge(&self) -> f64 {
        match *self {
            Self::Line { .. } => 0.0,
            Self::Arc { sweep, .. } => (sweep / 4.0).tan(),
        }
    }

    /// Closest point on this segment to `p`, returning `(distance, left_signed_distance)`.
    /// `left_signed_distance` is positive when `p` lies to the left of the
    /// segment's forward traversal direction.
    fn closest_to(&self, p: [f64; 2]) -> (f64, f64) {
        match *self {
            Self::Line { a, b, dir } => {
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len = ab[0].hypot(ab[1]);
                let t = if len <= EPS {
                    0.0
                } else {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / (len * len)).clamp(0.0, 1.0)
                };
                let q = [a[0] + t * ab[0], a[1] + t * ab[1]];
                let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                let left_n = [-dir[1], dir[0]];
                let signed = (p[0] - q[0]) * left_n[0] + (p[1] - q[1]) * left_n[1];
                (d, signed)
            }
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let ang = (p[1] - center[1]).atan2(p[0] - center[0]);
                let on_arc = angle_in_sweep(ang, start, sweep);
                let dist_c = (p[0] - center[0]).hypot(p[1] - center[1]);
                if on_arc {
                    let radial_out = dist_c - radius;
                    // For CCW arc (sweep > 0), left is inward (-radial_out).
                    // For CW arc (sweep < 0), left is outward (+radial_out).
                    let signed = if sweep >= 0.0 {
                        -radial_out
                    } else {
                        radial_out
                    };
                    (radial_out.abs(), signed)
                } else {
                    let s_pt = self.start_pt();
                    let e_pt = self.end_pt();
                    let ds = (p[0] - s_pt[0]).hypot(p[1] - s_pt[1]);
                    let de = (p[0] - e_pt[0]).hypot(p[1] - e_pt[1]);
                    let (q, theta) = if ds <= de {
                        (s_pt, start)
                    } else {
                        (e_pt, start + sweep)
                    };
                    let d = ds.min(de);
                    // Tangent at theta: CCW (-sin, cos), CW (sin, -cos); left normal is (-ty, tx).
                    let (st, ct) = theta.sin_cos();
                    let left_n = if sweep >= 0.0 { [-ct, -st] } else { [ct, st] };
                    let signed = (p[0] - q[0]) * left_n[0] + (p[1] - q[1]) * left_n[1];
                    (d, signed)
                }
            }
        }
    }

    /// Signed area contribution of this segment (Green's theorem, positive CCW).
    fn signed_area(&self) -> f64 {
        let a = self.start_pt();
        let b = self.end_pt();
        let trap = 0.5 * (a[0] * b[1] - b[0] * a[1]);
        match *self {
            Self::Line { .. } => trap,
            Self::Arc { radius, sweep, .. } => {
                // Circular segment area between chord and arc: 0.5 * r^2 * (sweep - sin(sweep)).
                trap + 0.5 * radius * radius * (sweep - sweep.sin())
            }
        }
    }

    fn offset_by(&self, left_signed: f64) -> Option<Self> {
        match *self {
            Self::Line { a, b, dir } => {
                let n = [-dir[1], dir[0]];
                Some(Self::Line {
                    a: [a[0] + left_signed * n[0], a[1] + left_signed * n[1]],
                    b: [b[0] + left_signed * n[0], b[1] + left_signed * n[1]],
                    dir,
                })
            }
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                // For CCW (sweep > 0), left is inward (radius - s); for CW, left is outward (radius + s).
                let new_r = if sweep >= 0.0 {
                    radius - left_signed
                } else {
                    radius + left_signed
                };
                if new_r <= EPS {
                    return None;
                }
                Some(Self::Arc {
                    center,
                    radius: new_r,
                    start,
                    sweep,
                })
            }
        }
    }
}

fn angle_in_sweep(angle: f64, start: f64, sweep: f64) -> bool {
    if sweep >= 0.0 {
        let d = (angle - start).rem_euclid(TAU);
        d <= sweep + 1e-9
    } else {
        let d = (start - angle).rem_euclid(TAU);
        d <= -sweep + 1e-9
    }
}

fn build_poly_segs(poly: &Polyline) -> Vec<PolySeg> {
    let n = poly.points.len();
    let count = if poly.closed { n } else { n.saturating_sub(1) };
    let mut segs = Vec::with_capacity(count);
    for i in 0..count {
        let p0 = poly.points[i];
        let p1 = poly.points[(i + 1) % n];
        let a = [p0.x, p0.y];
        let b = [p1.x, p1.y];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = dx.hypot(dy);
        if len <= EPS {
            continue;
        }
        let bulge = poly.bulge(i);
        if let Some(arc) = arc_of(a, b, bulge) {
            segs.push(PolySeg::Arc {
                center: arc.center,
                radius: arc.radius,
                start: arc.start,
                sweep: arc.sweep,
            });
        } else {
            segs.push(PolySeg::Line {
                a,
                b,
                dir: [dx / len, dy / len],
            });
        }
    }
    segs
}

fn point_in_closed_polyline(p: [f64; 2], poly: &Polyline) -> bool {
    // Tessellate closed polyline (respecting bulges) for accurate winding test.
    let n = poly.points.len();
    let mut outline: Vec<[f64; 2]> = Vec::new();
    for i in 0..n {
        let a = [poly.points[i].x, poly.points[i].y];
        let b = [poly.points[(i + 1) % n].x, poly.points[(i + 1) % n].y];
        let bulge = poly.bulge(i);
        if let Some(arc) = arc_of(a, b, bulge) {
            let steps = ((arc.sweep.abs() / (PI / 16.0)).ceil() as usize).clamp(4, 64);
            for s in 0..steps {
                let ang = arc.start + arc.sweep * (s as f64) / (steps as f64);
                let (sn, cs) = ang.sin_cos();
                outline.push([
                    arc.center[0] + arc.radius * cs,
                    arc.center[1] + arc.radius * sn,
                ]);
            }
        } else {
            outline.push(a);
        }
    }
    crate::draw::inside_outline(p, &outline)
}

fn line_line_intersection(
    a1: [f64; 2],
    d1: [f64; 2],
    a2: [f64; 2],
    d2: [f64; 2],
) -> Option<[f64; 2]> {
    let det = d1[0] * d2[1] - d1[1] * d2[0];
    if det.abs() <= 1e-10 {
        return None;
    }
    let rx = a2[0] - a1[0];
    let ry = a2[1] - a1[1];
    let t = (rx * d2[1] - ry * d2[0]) / det;
    Some([a1[0] + t * d1[0], a1[1] + t * d1[1]])
}

fn line_circle_intersections(
    a: [f64; 2],
    dir: [f64; 2],
    center: [f64; 2],
    radius: f64,
) -> Vec<[f64; 2]> {
    let fx = a[0] - center[0];
    let fy = a[1] - center[1];
    // dir is unit length.
    let b = fx * dir[0] + fy * dir[1];
    let c = fx * fx + fy * fy - radius * radius;
    let disc = b * b - c;
    if disc < -1e-7 * radius.max(1.0) {
        return Vec::new();
    }
    let root = disc.max(0.0).sqrt();
    let t1 = -b - root;
    let t2 = -b + root;
    if root <= 1e-9 {
        vec![[a[0] + t1 * dir[0], a[1] + t1 * dir[1]]]
    } else {
        vec![
            [a[0] + t1 * dir[0], a[1] + t1 * dir[1]],
            [a[0] + t2 * dir[0], a[1] + t2 * dir[1]],
        ]
    }
}

fn circle_circle_intersections(c1: [f64; 2], r1: f64, c2: [f64; 2], r2: f64) -> Vec<[f64; 2]> {
    let dx = c2[0] - c1[0];
    let dy = c2[1] - c1[1];
    let d = dx.hypot(dy);
    if d <= EPS || d > r1 + r2 + 1e-7 || d < (r1 - r2).abs() - 1e-7 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h_sq = r1 * r1 - a * a;
    if h_sq < -1e-7 * r1.max(1.0) {
        return Vec::new();
    }
    let h = h_sq.max(0.0).sqrt();
    let xm = c1[0] + a * dx / d;
    let ym = c1[1] + a * dy / d;
    if h <= 1e-9 {
        vec![[xm, ym]]
    } else {
        let rx = -dy * (h / d);
        let ry = dx * (h / d);
        vec![[xm + rx, ym + ry], [xm - rx, ym - ry]]
    }
}

fn update_arc_end(seg: &mut PolySeg, new_end: [f64; 2]) {
    if let PolySeg::Arc {
        center,
        start,
        sweep,
        ..
    } = seg
    {
        let end_ang = (new_end[1] - center[1]).atan2(new_end[0] - center[0]);
        let mut new_sweep = if *sweep >= 0.0 {
            (end_ang - *start).rem_euclid(TAU)
        } else {
            -((*start - end_ang).rem_euclid(TAU))
        };
        if new_sweep.abs() <= 1e-9 {
            new_sweep = 0.0;
        }
        *sweep = new_sweep;
    }
}

fn update_arc_start(seg: &mut PolySeg, new_start: [f64; 2]) {
    if let PolySeg::Arc {
        center,
        start,
        sweep,
        ..
    } = seg
    {
        let old_end = *start + *sweep;
        let start_ang = (new_start[1] - center[1]).atan2(new_start[0] - center[0]);
        let mut new_sweep = if *sweep >= 0.0 {
            (old_end - start_ang).rem_euclid(TAU)
        } else {
            -((start_ang - old_end).rem_euclid(TAU))
        };
        if new_sweep.abs() <= 1e-9 {
            new_sweep = 0.0;
        }
        *start = start_ang;
        *sweep = new_sweep;
    }
}

/// Joins two consecutive offset segments at their projected intersection
/// (AutoCAD `OFFSETGAPTYPE = 0`). Returns `true` when joined cleanly at a
/// shared vertex, or `false` if they have no intersection and require a bridge.
fn join_corner(prev: &mut PolySeg, next: &mut PolySeg) -> bool {
    let p1 = prev.end_pt();
    let q0 = next.start_pt();
    if (p1[0] - q0[0]).hypot(p1[1] - q0[1]) <= 1e-7 {
        let mid = [(p1[0] + q0[0]) * 0.5, (p1[1] + q0[1]) * 0.5];
        match prev {
            PolySeg::Line { b, .. } => *b = mid,
            PolySeg::Arc { .. } => update_arc_end(prev, mid),
        }
        match next {
            PolySeg::Line { a, .. } => *a = mid,
            PolySeg::Arc { .. } => update_arc_start(next, mid),
        }
        return true;
    }
    let pick_closest = |candidates: Vec<[f64; 2]>| -> Option<[f64; 2]> {
        candidates.into_iter().min_by(|u, v| {
            let du = (u[0] - p1[0]).hypot(u[1] - p1[1]) + (u[0] - q0[0]).hypot(u[1] - q0[1]);
            let dv = (v[0] - p1[0]).hypot(v[1] - p1[1]) + (v[0] - q0[0]).hypot(v[1] - q0[1]);
            du.total_cmp(&dv)
        })
    };
    let hit = match (*prev, *next) {
        (PolySeg::Line { a: a1, dir: d1, .. }, PolySeg::Line { a: a2, dir: d2, .. }) => {
            line_line_intersection(a1, d1, a2, d2)
        }
        (PolySeg::Line { a, dir, .. }, PolySeg::Arc { center, radius, .. })
        | (PolySeg::Arc { center, radius, .. }, PolySeg::Line { a, dir, .. }) => {
            pick_closest(line_circle_intersections(a, dir, center, radius))
        }
        (
            PolySeg::Arc {
                center: c1,
                radius: r1,
                ..
            },
            PolySeg::Arc {
                center: c2,
                radius: r2,
                ..
            },
        ) => pick_closest(circle_circle_intersections(c1, r1, c2, r2)),
    };
    let Some(m) = hit else {
        return false;
    };
    match prev {
        PolySeg::Line { b, .. } => *b = m,
        PolySeg::Arc { .. } => update_arc_end(prev, m),
    }
    match next {
        PolySeg::Line { a, .. } => *a = m,
        PolySeg::Arc { .. } => update_arc_start(next, m),
    }
    true
}

fn seg_is_collapsed(seg: &PolySeg) -> bool {
    match *seg {
        PolySeg::Line { a, b, dir } => {
            let dot = (b[0] - a[0]) * dir[0] + (b[1] - a[1]) * dir[1];
            dot <= 1e-7
        }
        PolySeg::Arc { sweep, radius, .. } => sweep.abs() * radius <= 1e-7,
    }
}

fn offset_polyline(
    poly: &Polyline,
    by: &OffsetBy,
    layer: String,
) -> Result<(Polyline, f64), Error> {
    if !poly.is_flat() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`offset` requires a flat (2D) polyline whose points share one Z elevation".to_owned(),
        ));
    }
    let z = poly.points.first().map_or(0.0, |p| p.z);
    let orig_segs = build_poly_segs(poly);
    if orig_segs.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot offset a polyline with no non-zero segments".to_owned(),
        ));
    }
    let orig_area: f64 = if poly.closed {
        orig_segs.iter().map(PolySeg::signed_area).sum()
    } else {
        0.0
    };

    // Resolve signed offset (positive = left of traversal direction).
    let left_signed = match *by {
        OffsetBy::Distance { distance, side } => {
            let sign = match side {
                OffsetSide::Left => 1.0,
                OffsetSide::Right => -1.0,
                OffsetSide::Inside | OffsetSide::Outside => {
                    if !poly.closed {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "an open polyline has no inside or outside; use `left`, `right`, or a \
                             point for `side`"
                                .to_owned(),
                        ));
                    }
                    if orig_area.abs() <= EPS {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "cannot determine inside or outside of a zero-area polyline; use \
                             `left`, `right`, or a point"
                                .to_owned(),
                        ));
                    }
                    let ccw = orig_area > 0.0;
                    match (side, ccw) {
                        (OffsetSide::Inside, true) | (OffsetSide::Outside, false) => 1.0,
                        (OffsetSide::Inside, false) | (OffsetSide::Outside, true) => -1.0,
                        _ => unreachable!(),
                    }
                }
                OffsetSide::Point(p) => {
                    let pt = [p[0], p[1]];
                    let min_dist = orig_segs
                        .iter()
                        .map(|s| s.closest_to(pt))
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .unwrap();
                    if min_dist.0 <= EPS {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "the `side` point lies on the polyline; pick a point off it".to_owned(),
                        ));
                    }
                    if poly.closed && orig_area.abs() > EPS {
                        let inside = point_in_closed_polyline(pt, poly);
                        let ccw = orig_area > 0.0;
                        if inside == ccw {
                            1.0
                        } else {
                            -1.0
                        }
                    } else if min_dist.1 >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            sign * distance
        }
        OffsetBy::Through(p) => {
            let pt = [p[0], p[1]];
            let closest = orig_segs
                .iter()
                .map(|s| s.closest_to(pt))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap();
            if closest.0 <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "the `through` point lies on the polyline; pick a point off it".to_owned(),
                ));
            }
            let sign = if poly.closed && orig_area.abs() > EPS {
                let inside = point_in_closed_polyline(pt, poly);
                let ccw = orig_area > 0.0;
                if inside == ccw {
                    1.0
                } else {
                    -1.0
                }
            } else if closest.1 >= 0.0 {
                1.0
            } else {
                -1.0
            };
            sign * closest.0
        }
    };

    // Offset each segment independently, dropping arc segments whose radius shrinks <= 0.
    let mut active: Vec<PolySeg> = orig_segs
        .iter()
        .filter_map(|s| s.offset_by(left_signed))
        .collect();
    if active.is_empty() || (poly.closed && active.len() < 2) {
        return Err(Error::new(
            ErrorCode::BadValue,
            "offset distance is too large for this polyline, leaving no valid geometry".to_owned(),
        ));
    }

    // Iteratively join corners (OFFSETGAPTYPE 0) and drop collapsed segments.
    let max_iters = active.len() + 2;
    for _ in 0..max_iters {
        let mut joined = active.clone();
        let m = joined.len();
        let corners = if poly.closed { m } else { m.saturating_sub(1) };
        for i in 0..corners {
            let next_i = (i + 1) % m;
            if i == next_i {
                break;
            }
            if next_i > i {
                let (left_slice, right_slice) = joined.split_at_mut(next_i);
                let _ = join_corner(&mut left_slice[i], &mut right_slice[0]);
            } else {
                // Closing corner (last -> 0).
                let (first_slice, rest) = joined.split_at_mut(1);
                let _ = join_corner(&mut rest[i - 1], &mut first_slice[0]);
            }
        }
        let collapsed: Vec<usize> = joined
            .iter()
            .enumerate()
            .filter_map(|(i, s)| seg_is_collapsed(s).then_some(i))
            .collect();
        if collapsed.is_empty() {
            active = joined;
            break;
        }
        let remaining: Vec<PolySeg> = active
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (!collapsed.contains(&i)).then_some(*s))
            .collect();
        if remaining.is_empty() || (poly.closed && remaining.len() < 2) {
            return Err(Error::new(
                ErrorCode::BadValue,
                "offset distance is too large for this polyline; the offset collapses".to_owned(),
            ));
        }
        active = remaining;
    }

    // Verify a closed offset did not invert winding or collapse to zero area.
    if poly.closed && orig_area.abs() > EPS {
        let new_area: f64 = active.iter().map(PolySeg::signed_area).sum();
        if new_area.abs() <= EPS || new_area.signum() != orig_area.signum() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "offset distance is too large for this closed polyline; the interior collapses"
                    .to_owned(),
            ));
        }
    }

    // Reconstruct Polyline vertices and bulges.
    let mut points: Vec<Point3> = Vec::new();
    let mut bulges: Vec<f64> = Vec::new();
    let m = active.len();
    for (i, seg) in active.iter().enumerate() {
        let s = seg.start_pt();
        let e = seg.end_pt();
        if i == 0 {
            points.push(Point3::new(s[0], s[1], z));
            bulges.push(seg.bulge());
        } else {
            let prev_end = active[i - 1].end_pt();
            if (prev_end[0] - s[0]).hypot(prev_end[1] - s[1]) > 1e-6 {
                // Bridge gap when an arc and line could not intersect cleanly.
                points.push(Point3::new(prev_end[0], prev_end[1], z));
                bulges.push(0.0);
                points.push(Point3::new(s[0], s[1], z));
                bulges.push(seg.bulge());
            } else {
                points.push(Point3::new(s[0], s[1], z));
                bulges.push(seg.bulge());
            }
        }
        if !poly.closed && i + 1 == m {
            points.push(Point3::new(e[0], e[1], z));
            bulges.push(0.0);
        }
    }
    if poly.closed && m >= 1 {
        let last_end = active[m - 1].end_pt();
        let first_start = active[0].start_pt();
        if (last_end[0] - first_start[0]).hypot(last_end[1] - first_start[1]) > 1e-6 {
            points.push(Point3::new(last_end[0], last_end[1], z));
            bulges.push(0.0);
        }
    }
    if points.len() < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "offset distance leaves fewer than two vertices".to_owned(),
        ));
    }
    let has_bulge = bulges.iter().any(|&b| b.abs() > 1e-12);
    let widths = if !poly.widths.is_empty() && poly.widths.len() == points.len() {
        poly.widths.clone()
    } else {
        Vec::new()
    };
    Ok((
        Polyline {
            points,
            bulges: if has_bulge { bulges } else { Vec::new() },
            widths,
            width: poly.width,
            closed: poly.closed,
            layer,
            properties: poly.properties.clone(),
        },
        left_signed.abs(),
    ))
}

// ---------------------------------------------------------------------------
// Ellipse -> concentric Ellipse (circular) or exact parallel cubic Spline (AutoCAD)
// ---------------------------------------------------------------------------

fn offset_ellipse(e: &Ellipse, by: &OffsetBy, layer: String) -> Result<(Object, f64), Error> {
    let major = [e.major_axis.x, e.major_axis.y];
    let a = major[0].hypot(major[1]);
    let b = a * e.ratio;
    if a <= EPS || b <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot offset a degenerate ellipse".to_owned(),
        ));
    }
    let minor = [-major[1] * e.ratio, major[0] * e.ratio];
    let (from, sweep) = if e.is_whole() {
        (0.0, TAU)
    } else {
        let start = ellipse_parameter(e.start_angle, e.ratio);
        let end = ellipse_parameter(e.end_angle, e.ratio);
        let sw = (end - start).rem_euclid(TAU);
        (start, if sw == 0.0 { TAU } else { sw })
    };
    let eval = |t: f64| -> ([f64; 2], [f64; 2]) {
        let (st, ct) = t.sin_cos();
        let pt = [
            e.center.x + ct * major[0] + st * minor[0],
            e.center.y + ct * major[1] + st * minor[1],
        ];
        let tan = [
            -st * major[0] + ct * minor[0],
            -st * major[1] + ct * minor[1],
        ];
        let tl = tan[0].hypot(tan[1]).max(EPS);
        // Outward normal (right of CCW tangent).
        let out_n = [tan[1] / tl, -tan[0] / tl];
        (pt, out_n)
    };

    // Positive outward_signed expands away from center; negative shrinks inward.
    let outward_signed = match *by {
        OffsetBy::Distance { distance, side } => match side {
            OffsetSide::Outside | OffsetSide::Right => distance,
            OffsetSide::Inside | OffsetSide::Left => -distance,
            OffsetSide::Point(p) => {
                let (dist, signed) = closest_ellipse_point(&eval, from, sweep, [p[0], p[1]]);
                if dist <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "the `side` point lies on the ellipse; pick a point inside or outside it"
                            .to_owned(),
                    ));
                }
                if signed >= 0.0 {
                    distance
                } else {
                    -distance
                }
            }
        },
        OffsetBy::Through(p) => {
            let (dist, signed) = closest_ellipse_point(&eval, from, sweep, [p[0], p[1]]);
            if dist <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "the `through` point lies on the ellipse; pick a point off it".to_owned(),
                ));
            }
            if signed >= 0.0 {
                dist
            } else {
                -dist
            }
        }
    };

    if outward_signed < 0.0 && outward_signed.abs() >= b - EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "inward offset distance ({:.6}) is at least the ellipse's minor semi-axis ({b:.6})",
                outward_signed.abs()
            ),
        ));
    }

    // Circular ellipse offsets to an exact concentric Ellipse.
    if (e.ratio - 1.0).abs() <= 1e-9 {
        let new_a = a + outward_signed;
        let scale = new_a / a;
        return Ok((
            Object::Ellipse(Ellipse {
                center: e.center,
                major_axis: Point3::new(e.major_axis.x * scale, e.major_axis.y * scale, 0.0),
                ratio: 1.0,
                start_angle: e.start_angle,
                end_angle: e.end_angle,
                layer,
                properties: e.properties.clone(),
            }),
            outward_signed.abs(),
        ));
    }

    // Non-circular ellipse: AutoCAD creates a cubic spline through exact offset points.
    let steps = ((sweep / TAU) * 48.0).ceil().clamp(16.0, 64.0) as usize;
    let mut fit_points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = from + sweep * (i as f64) / (steps as f64);
        let (pt, out_n) = eval(t);
        fit_points.push(Point3::new(
            pt[0] + outward_signed * out_n[0],
            pt[1] + outward_signed * out_n[1],
            e.center.z,
        ));
    }
    if e.is_whole() && fit_points.len() >= 2 {
        let first = fit_points[0];
        *fit_points.last_mut().unwrap() = first;
    }
    Ok((
        Object::Spline(Spline {
            fit_points,
            control_points: Vec::new(),
            degree: 3,
            layer,
            properties: e.properties.clone(),
        }),
        outward_signed.abs(),
    ))
}

fn closest_ellipse_point(
    eval: &impl Fn(f64) -> ([f64; 2], [f64; 2]),
    from: f64,
    sweep: f64,
    p: [f64; 2],
) -> (f64, f64) {
    let coarse = 128usize;
    let mut best_t = from;
    let mut best_d2 = f64::INFINITY;
    for i in 0..=coarse {
        let t = from + sweep * (i as f64) / (coarse as f64);
        let (q, _) = eval(t);
        let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
        if d2 < best_d2 {
            best_d2 = d2;
            best_t = t;
        }
    }
    // Golden-section local refinement around best_t.
    let step = sweep / (coarse as f64);
    let mut lo = (best_t - step).max(from);
    let mut hi = (best_t + step).min(from + sweep);
    for _ in 0..24 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        let (q1, _) = eval(m1);
        let (q2, _) = eval(m2);
        let d1 = (p[0] - q1[0]).powi(2) + (p[1] - q1[1]).powi(2);
        let d2 = (p[0] - q2[0]).powi(2) + (p[1] - q2[1]).powi(2);
        if d1 < d2 {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let t = 0.5 * (lo + hi);
    let (q, out_n) = eval(t);
    let dist = (p[0] - q[0]).hypot(p[1] - q[1]);
    let signed = (p[0] - q[0]) * out_n[0] + (p[1] - q[1]) * out_n[1];
    (dist, signed)
}

// ---------------------------------------------------------------------------
// Spline -> parallel cubic fit-point Spline
// ---------------------------------------------------------------------------

fn offset_spline(spline: &Spline, by: &OffsetBy, layer: String) -> Result<(Spline, f64), Error> {
    let Some((degree, control, knots)) = spline_definition(spline) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot offset a spline with too few points".to_owned(),
        ));
    };
    let bspline = Curve::Spline(BSpline::new(degree, control, knots));
    let (u0, u1) = bspline.domain();
    let span = u1 - u0;
    if span <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot offset a degenerate spline".to_owned(),
        ));
    }
    let eval = |u: f64| -> ([f64; 3], [f64; 2]) {
        let p = bspline.point(u);
        let mut t = bspline.tangent(u);
        let mut len = t[0].hypot(t[1]);
        if len <= 1e-12 {
            let eps = span * 1e-5;
            let u_nudge = if u + eps <= u1 { u + eps } else { u - eps };
            t = bspline.tangent(u_nudge);
            len = t[0].hypot(t[1]);
        }
        let left_n = if len <= 1e-12 {
            [0.0, 1.0]
        } else {
            [-t[1] / len, t[0] / len]
        };
        (p, left_n)
    };

    let p_start = bspline.point(u0);
    let p_end = bspline.point(u1);
    let is_closed = plan_distance(p_start, p_end) <= 1e-6;
    let signed_area = if is_closed {
        let samples = 64usize;
        let pts: Vec<[f64; 3]> = (0..=samples)
            .map(|i| bspline.point(u0 + span * (i as f64) / (samples as f64)))
            .collect();
        pts.windows(2)
            .map(|w| 0.5 * (w[0][0] * w[1][1] - w[1][0] * w[0][1]))
            .sum::<f64>()
    } else {
        0.0
    };

    let closest_on_spline = |p: [f64; 2]| -> (f64, f64) {
        let samples = 128usize;
        let mut best_u = u0;
        let mut best_d2 = f64::INFINITY;
        for i in 0..=samples {
            let u = u0 + span * (i as f64) / (samples as f64);
            let q = bspline.point(u);
            let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best_u = u;
            }
        }
        let step = span / (samples as f64);
        let mut lo = (best_u - step).max(u0);
        let mut hi = (best_u + step).min(u1);
        for _ in 0..24 {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            let q1 = bspline.point(m1);
            let q2 = bspline.point(m2);
            let d1 = (p[0] - q1[0]).powi(2) + (p[1] - q1[1]).powi(2);
            let d2 = (p[0] - q2[0]).powi(2) + (p[1] - q2[1]).powi(2);
            if d1 < d2 {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let (q, left_n) = eval(0.5 * (lo + hi));
        let dist = (p[0] - q[0]).hypot(p[1] - q[1]);
        let signed = (p[0] - q[0]) * left_n[0] + (p[1] - q[1]) * left_n[1];
        (dist, signed)
    };

    let left_signed = match *by {
        OffsetBy::Distance { distance, side } => match side {
            OffsetSide::Left => distance,
            OffsetSide::Right => -distance,
            OffsetSide::Inside | OffsetSide::Outside => {
                if !is_closed {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "an open spline has no inside or outside; use `left`, `right`, or a point \
                         for `side`"
                            .to_owned(),
                    ));
                }
                if signed_area.abs() <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "cannot determine inside or outside of a zero-area spline".to_owned(),
                    ));
                }
                let ccw = signed_area > 0.0;
                match (side, ccw) {
                    (OffsetSide::Inside, true) | (OffsetSide::Outside, false) => distance,
                    (OffsetSide::Inside, false) | (OffsetSide::Outside, true) => -distance,
                    _ => unreachable!(),
                }
            }
            OffsetSide::Point(p) => {
                let (dist, signed) = closest_on_spline([p[0], p[1]]);
                if dist <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "the `side` point lies on the spline; pick a point off it".to_owned(),
                    ));
                }
                if signed >= 0.0 {
                    distance
                } else {
                    -distance
                }
            }
        },
        OffsetBy::Through(p) => {
            let (dist, signed) = closest_on_spline([p[0], p[1]]);
            if dist <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "the `through` point lies on the spline; pick a point off it".to_owned(),
                ));
            }
            if signed >= 0.0 {
                dist
            } else {
                -dist
            }
        }
    };

    let base_count = spline.fit_points.len().max(spline.control_points.len());
    let steps = (base_count * 6).clamp(24, 64);
    let mut fit_points: Vec<Point3> = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let u = u0 + span * (i as f64) / (steps as f64);
        let (p, left_n) = eval(u);
        let q = Point3::new(
            p[0] + left_signed * left_n[0],
            p[1] + left_signed * left_n[1],
            p[2],
        );
        if fit_points
            .last()
            .is_none_or(|prev| (prev.x - q.x).hypot(prev.y - q.y) > 1e-7 || i == steps)
        {
            fit_points.push(q);
        }
    }
    if is_closed && fit_points.len() >= 2 {
        let first = fit_points[0];
        *fit_points.last_mut().unwrap() = first;
    }
    if fit_points.len() < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "offset leaves too few distinct points for a spline".to_owned(),
        ));
    }
    Ok((
        Spline {
            fit_points,
            control_points: Vec::new(),
            degree: 3,
            layer,
            properties: spline.properties.clone(),
        },
        left_signed.abs(),
    ))
}

// ---------------------------------------------------------------------------
// Serialize offset Object to sheet-description JSON keys for RecordFile::add
// ---------------------------------------------------------------------------

fn pt_json(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn push_common_keys(
    keys: &mut Map<String, Value>,
    layer: &str,
    props: &Properties,
    id: &str,
    name: Option<&str>,
    space: Option<&str>,
) {
    if layer != LAYER_ZERO {
        keys.insert("layer".to_owned(), Value::String(layer.to_owned()));
    }
    if !props.color.is_by_layer() {
        keys.insert("color".to_owned(), json!(props.color));
    }
    if !props.linetype.is_by_layer() {
        keys.insert("linetype".to_owned(), json!(props.linetype));
    }
    if !props.lineweight.is_by_layer() {
        keys.insert("lineweight".to_owned(), json!(props.lineweight));
    }
    if (props.linetype_scale - 1.0).abs() > 1e-12 {
        keys.insert("linetype_scale".to_owned(), json!(props.linetype_scale));
    }
    keys.insert("id".to_owned(), Value::String(id.to_owned()));
    if let Some(name) = name {
        keys.insert("name".to_owned(), Value::String(name.to_owned()));
    }
    if let Some(space) = space {
        keys.insert("space".to_owned(), Value::String(space.to_owned()));
    }
}

/// Converts a newly computed offset [`Object`] into `(kind, keys)` ready for
/// `RecordFile::add`.
pub(crate) fn object_to_record_keys(
    object: &Object,
    id: &str,
    name: Option<&str>,
    space: Option<&str>,
) -> (&'static str, Map<String, Value>) {
    let mut keys = Map::new();
    match object {
        Object::Line(l) => {
            keys.insert("from".to_owned(), pt_json(l.from));
            keys.insert("to".to_owned(), pt_json(l.to));
            push_common_keys(&mut keys, &l.layer, &l.properties, id, name, space);
            ("line", keys)
        }
        Object::Circle(c) => {
            keys.insert("center".to_owned(), pt_json(c.center));
            keys.insert("radius".to_owned(), json!(c.radius));
            push_common_keys(&mut keys, &c.layer, &c.properties, id, name, space);
            ("circle", keys)
        }
        Object::Arc(a) => {
            keys.insert("center".to_owned(), pt_json(a.center));
            keys.insert("radius".to_owned(), json!(a.radius));
            keys.insert("start_angle".to_owned(), json!(a.start_angle));
            keys.insert("end_angle".to_owned(), json!(a.end_angle));
            push_common_keys(&mut keys, &a.layer, &a.properties, id, name, space);
            ("arc", keys)
        }
        Object::Polyline(p) => {
            let has_bulges = !p.bulges.is_empty();
            let has_widths = !p.widths.is_empty();
            let pts: Vec<Value> = p
                .points
                .iter()
                .enumerate()
                .map(|(i, &pt)| {
                    let b = if has_bulges {
                        p.bulges.get(i).copied().unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    let w = if has_widths {
                        p.widths.get(i).copied()
                    } else {
                        None
                    };
                    if b.abs() <= 1e-12 && w.is_none() {
                        pt_json(pt)
                    } else {
                        let mut v = Map::new();
                        v.insert("point".to_owned(), pt_json(pt));
                        if b.abs() > 1e-12 {
                            v.insert("bulge".to_owned(), json!(b));
                        }
                        if let Some([sw, ew]) = w {
                            v.insert("start_width".to_owned(), json!(sw));
                            v.insert("end_width".to_owned(), json!(ew));
                        }
                        Value::Object(v)
                    }
                })
                .collect();
            keys.insert("points".to_owned(), Value::Array(pts));
            if p.width != 0.0 {
                keys.insert("width".to_owned(), json!(p.width));
            }
            if p.closed {
                keys.insert("closed".to_owned(), Value::Bool(true));
            }
            push_common_keys(&mut keys, &p.layer, &p.properties, id, name, space);
            ("polyline", keys)
        }
        Object::Ellipse(e) => {
            keys.insert("center".to_owned(), pt_json(e.center));
            keys.insert("major_axis".to_owned(), pt_json(e.major_axis));
            keys.insert("ratio".to_owned(), json!(e.ratio));
            if !e.is_whole() {
                keys.insert("start_angle".to_owned(), json!(e.start_angle));
                keys.insert("end_angle".to_owned(), json!(e.end_angle));
            }
            push_common_keys(&mut keys, &e.layer, &e.properties, id, name, space);
            ("ellipse", keys)
        }
        Object::Spline(s) => {
            if !s.fit_points.is_empty() {
                keys.insert(
                    "fit_points".to_owned(),
                    Value::Array(s.fit_points.iter().map(|&p| pt_json(p)).collect()),
                );
            } else {
                keys.insert(
                    "control_points".to_owned(),
                    Value::Array(s.control_points.iter().map(|&p| pt_json(p)).collect()),
                );
                keys.insert("degree".to_owned(), json!(s.degree));
            }
            push_common_keys(&mut keys, &s.layer, &s.properties, id, name, space);
            ("spline", keys)
        }
        Object::Solid(s) => {
            keys.insert(
                "corners".to_owned(),
                Value::Array(s.corners.iter().map(|&p| pt_json(p)).collect()),
            );
            push_common_keys(&mut keys, &s.layer, &s.properties, id, name, space);
            ("solid", keys)
        }
        Object::Wipeout(w) => {
            keys.insert(
                "points".to_owned(),
                Value::Array(w.points.iter().map(|&p| pt_json(p)).collect()),
            );
            push_common_keys(&mut keys, &w.layer, &w.properties, id, name, space);
            ("wipeout", keys)
        }
        Object::RevisionCloud(rc) => {
            match &rc.boundary {
                crate::model::RevisionCloudBoundary::Rectangle { from, to } => {
                    keys.insert(
                        "rectangle".to_owned(),
                        json!({ "from": pt_json(*from), "to": pt_json(*to) }),
                    );
                }
                crate::model::RevisionCloudBoundary::Polygon(points) => {
                    keys.insert(
                        "polygon".to_owned(),
                        Value::Array(points.iter().map(|&p| pt_json(p)).collect()),
                    );
                }
            }
            keys.insert("arc_length".to_owned(), json!(rc.arc_length));
            if rc.style != crate::model::RevisionCloudStyle::Normal {
                keys.insert("style".to_owned(), json!("calligraphy"));
            }
            if rc.reverse {
                keys.insert("reverse".to_owned(), Value::Bool(true));
            }
            push_common_keys(&mut keys, &rc.layer, &rc.properties, id, name, space);
            ("revision_cloud", keys)
        }
        _ => unreachable!(
            "object_to_record_keys only produces curve, solid, wipeout or revision_cloud objects"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn line_offset_left_right_point_and_through() {
        let line = Object::Line(Line {
            from: Point3::new(0.0, 0.0, 2.0),
            to: Point3::new(10.0, 0.0, 2.0),
            layer: "walls".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Line(left), d) = compute_offset(
            &line,
            &OffsetBy::Distance {
                distance: 5.0,
                side: OffsetSide::Left,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(d, 5.0);
        close(left.from.y, 5.0);
        close(left.to.y, 5.0);
        close(left.from.z, 2.0);
        assert_eq!(left.layer, "walls");

        let (Object::Line(through), d2) =
            compute_offset(&line, &OffsetBy::Through([3.0, -7.5, 0.0]), Some("aux")).unwrap()
        else {
            panic!()
        };
        close(d2, 7.5);
        close(through.from.y, -7.5);
        assert_eq!(through.layer, "aux");

        assert!(compute_offset(
            &line,
            &OffsetBy::Distance {
                distance: 1.0,
                side: OffsetSide::Inside
            },
            None
        )
        .is_err());
    }

    #[test]
    fn circle_and_arc_offset_inside_outside_and_reject_collapse() {
        let circle = Object::Circle(Circle {
            center: Point3::new(10.0, 20.0, 0.0),
            radius: 15.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Circle(out), _) = compute_offset(
            &circle,
            &OffsetBy::Distance {
                distance: 5.0,
                side: OffsetSide::Outside,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(out.radius, 20.0);

        let (Object::Circle(inn), _) = compute_offset(
            &circle,
            &OffsetBy::Distance {
                distance: 5.0,
                side: OffsetSide::Inside,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(inn.radius, 10.0);

        assert!(compute_offset(
            &circle,
            &OffsetBy::Distance {
                distance: 15.0,
                side: OffsetSide::Inside
            },
            None
        )
        .is_err());

        let arc = Object::Arc(Arc {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 10.0,
            start_angle: 0.0,
            end_angle: 90.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Arc(ao), _) =
            compute_offset(&arc, &OffsetBy::Through([12.0, 5.0, 0.0]), None).unwrap()
        else {
            panic!()
        };
        close(ao.radius, 13.0);
        close(ao.start_angle, 0.0);
        close(ao.end_angle, 90.0);
    }

    #[test]
    fn polyline_rectangle_and_open_corner_join_with_offsetgaptype_0() {
        // Closed CCW 100x40 rectangle offset inward by 5 -> 90x30 rectangle.
        let rect = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(100.0, 0.0, 0.0),
                Point3::new(100.0, 40.0, 0.0),
                Point3::new(0.0, 40.0, 0.0),
            ],
            true,
            "enclosure",
        ));
        let (Object::Polyline(inner), d) = compute_offset(
            &rect,
            &OffsetBy::Distance {
                distance: 5.0,
                side: OffsetSide::Inside,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(d, 5.0);
        assert!(inner.closed);
        assert_eq!(inner.points.len(), 4);
        close(inner.points[0].x, 5.0);
        close(inner.points[0].y, 5.0);
        close(inner.points[2].x, 95.0);
        close(inner.points[2].y, 35.0);

        // Inward by 20 collapses the 40-tall rectangle -> refused cleanly.
        assert!(compute_offset(
            &rect,
            &OffsetBy::Distance {
                distance: 20.0,
                side: OffsetSide::Inside
            },
            None
        )
        .is_err());

        // Open L-shape offset to the right (outer corner): extended to exact miter (10, -5) -> (15, -5) -> (15, 10).
        let lshape = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(10.0, 10.0, 0.0),
            ],
            false,
            "0",
        ));
        let (Object::Polyline(outer), _) = compute_offset(
            &lshape,
            &OffsetBy::Distance {
                distance: 5.0,
                side: OffsetSide::Right,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(outer.points.len(), 3);
        close(outer.points[0].x, 0.0);
        close(outer.points[0].y, -5.0);
        close(outer.points[1].x, 15.0);
        close(outer.points[1].y, -5.0);
        close(outer.points[2].x, 15.0);
        close(outer.points[2].y, 10.0);
    }

    #[test]
    fn polyline_with_arc_bulge_offsets_concentrically() {
        // Rounded stadium (straight + semicircular bulge + straight + semicircular bulge).
        let stadium = Object::Polyline(Polyline {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(20.0, 0.0, 0.0),
                Point3::new(20.0, 10.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
            ],
            bulges: vec![0.0, 1.0, 0.0, 1.0],
            widths: Vec::new(),
            width: 0.0,
            closed: true,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Polyline(out), _) = compute_offset(
            &stadium,
            &OffsetBy::Distance {
                distance: 2.0,
                side: OffsetSide::Outside,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        assert!(out.closed);
        assert_eq!(out.points.len(), 4);
        close(out.points[0].x, 0.0);
        close(out.points[0].y, -2.0);
        close(out.points[1].x, 20.0);
        close(out.points[1].y, -2.0);
        close(out.points[2].x, 20.0);
        close(out.points[2].y, 12.0);
        close(out.bulge(1), 1.0);
        close(out.bulge(3), 1.0);
    }

    #[test]
    fn ellipse_and_spline_offset_maintain_exact_distance() {
        let ellipse = Object::Ellipse(Ellipse {
            center: Point3::new(0.0, 0.0, 0.0),
            major_axis: Point3::new(20.0, 0.0, 0.0),
            ratio: 0.5,
            start_angle: 0.0,
            end_angle: 360.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Spline(sp), d) = compute_offset(
            &ellipse,
            &OffsetBy::Distance {
                distance: 3.0,
                side: OffsetSide::Outside,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(d, 3.0);
        // At t = 0 (major axis tip +20, 0) -> (+23, 0); at t = pi/2 (minor axis tip 0, +10) -> (0, +13).
        close(sp.fit_points[0].x, 23.0);
        close(sp.fit_points[0].y, 0.0);
        let quarter = sp.fit_points.len() / 4;
        close(sp.fit_points[quarter].x, 0.0);
        close(sp.fit_points[quarter].y, 13.0);

        let spline = Object::Spline(Spline {
            fit_points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 5.0, 0.0),
                Point3::new(20.0, 0.0, 0.0),
            ],
            control_points: Vec::new(),
            degree: 3,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let (Object::Spline(off_sp), d2) = compute_offset(
            &spline,
            &OffsetBy::Distance {
                distance: 2.0,
                side: OffsetSide::Left,
            },
            None,
        )
        .unwrap() else {
            panic!()
        };
        close(d2, 2.0);
        assert!(off_sp.fit_points.len() >= 20);
        // Midpoint apex (10, 5) has horizontal tangent -> offset straight up by +2 -> y ~= 7.
        let mid = off_sp.fit_points[off_sp.fit_points.len() / 2];
        assert!((mid.y - 7.0).abs() < 0.25, "mid={mid:?}");
    }
}
