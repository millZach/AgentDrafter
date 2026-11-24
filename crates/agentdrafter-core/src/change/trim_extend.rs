//! Geometric trim (`trim` / `TRIM` / `TR`) and extend (`extend` / `EXTEND` /
//! `EX`) against one or more boundary objects' curves.
//!
//! All intersections reuse `geometry::curve::crossings` (from #74), so any
//! boundary object whose geometry has curves—lines, polylines, arcs, circles,
//! ellipses, splines, solids, wipeouts, tables, and block inserts—can act as a
//! cutting or boundary edge.

use std::f64::consts::TAU;

use crate::draw::curves::{ellipse_parameter, spline_definition};
use crate::draw::polyline::arc_of;
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::{crossings, length_of, sub, BSpline, Curve, P3};
use crate::model::{Arc, Circle, Ellipse, Line, Object, Point3, Polyline, Spline};

const EPS: f64 = 1e-7;

/// Which part of `target` a `trim` operation keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrimKeep {
    /// Keep from the start of the object up to the first crossing (or on a
    /// closed curve, the span from the first crossing forward to the second).
    Start,
    /// Keep from the last crossing to the end of the object (or on a closed
    /// curve, the complementary span from the second crossing around to the
    /// first).
    End,
    /// Keep the span between the first and last crossings (requires ≥ 2
    /// crossings).
    Between,
    /// Remove the middle span between the first and last crossings, splitting
    /// an open curve into two pieces (the start piece keeps `target`'s ID; the
    /// end piece gets a new ID). On a closed curve, keeps the complementary
    /// span from the second crossing around to the first.
    Ends,
}

impl TrimKeep {
    pub(crate) const ALL: [&'static str; 4] = ["start", "end", "between", "ends"];

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            "between" => Some(Self::Between),
            "ends" => Some(Self::Ends),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Between => "between",
            Self::Ends => "ends",
        }
    }
}

/// Which end(s) of `target` an `extend` operation carries on to a boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExtendEnd {
    Start,
    End,
    Both,
}

impl ExtendEnd {
    pub(crate) const ALL: [&'static str; 3] = ["start", "end", "both"];

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Both => "both",
        }
    }
}

/// Output of [`compute_trim`]: the kept object (retaining `target`'s ID) and
/// an optional second piece created when `keep: "ends"` splits an open object.
#[derive(Clone, Debug)]
pub(crate) struct TrimResult {
    pub(crate) kept: Object,
    pub(crate) split: Option<Object>,
}

fn p3_of(p: Point3) -> P3 {
    [p.x, p.y, p.z]
}

fn point3_of(p: P3) -> Point3 {
    Point3::new(p[0], p[1], p[2])
}

/// Inverse of [`ellipse_parameter`]: turns an ellipse parameter (radians) into
/// the angle in degrees `[0, 360)` anticlockwise from the major axis.
fn ellipse_degrees(param: f64, ratio: f64) -> f64 {
    let (s, c) = param.sin_cos();
    (ratio * s).atan2(c).to_degrees().rem_euclid(360.0)
}

fn dedup_params(params: &mut Vec<f64>, tol: f64) {
    params.sort_by(|a, b| a.total_cmp(b));
    let mut out: Vec<f64> = Vec::with_capacity(params.len());
    for &p in params.iter() {
        if out.last().is_none_or(|prev| (p - prev).abs() > tol) {
            out.push(p);
        }
    }
    *params = out;
}

fn curve_crossings_with(curve: &Curve, boundaries: &[Curve]) -> Vec<f64> {
    let mut ts = Vec::new();
    for b in boundaries {
        for (t, _) in crossings(curve, b) {
            ts.push(t);
        }
    }
    ts
}

fn interior_crossings(curve: &Curve, boundaries: &[Curve]) -> Vec<f64> {
    let (d0, d1) = curve.domain();
    let tol = (d1 - d0) * 1e-6;
    let mut ts: Vec<f64> = curve_crossings_with(curve, boundaries)
        .into_iter()
        .filter(|&t| t > d0 + tol && t < d1 - tol)
        .collect();
    dedup_params(&mut ts, tol);
    ts
}

fn closed_crossings(curve: &Curve, boundaries: &[Curve]) -> Vec<f64> {
    let mut ts: Vec<f64> = curve_crossings_with(curve, boundaries)
        .into_iter()
        .map(|t| t.rem_euclid(1.0))
        .collect();
    dedup_params(&mut ts, 1e-6);
    if ts.len() >= 2 && (1.0 - (*ts.last().unwrap() - ts[0])).abs() <= 1e-6 {
        ts.pop();
    }
    ts
}

// ---------------------------------------------------------------------------
// TRIM
// ---------------------------------------------------------------------------

pub(crate) fn compute_trim(
    target: &Object,
    boundaries: &[Curve],
    keep: TrimKeep,
) -> Result<TrimResult, Error> {
    if boundaries.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the boundary object has no geometry to trim against".to_owned(),
        ));
    }
    match target {
        Object::Line(line) => trim_line(line, boundaries, keep),
        Object::Arc(arc) => trim_arc(arc, boundaries, keep),
        Object::Circle(circle) => trim_circle(circle, boundaries, keep),
        Object::Polyline(poly) => trim_polyline(poly, boundaries, keep),
        Object::Ellipse(ellipse) => trim_ellipse(ellipse, boundaries, keep),
        Object::Spline(spline) => trim_spline(spline, boundaries, keep),
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`trim` works on a line, polyline, arc, circle, ellipse or spline, not a {}",
                other.type_name()
            ),
        )),
    }
}

fn need_crossings(what: &str, have: usize, need: usize) -> Error {
    if have == 0 {
        Error::new(
            ErrorCode::BadValue,
            format!("the boundary does not cross the {what}; nothing to trim"),
        )
    } else {
        Error::new(
            ErrorCode::BadValue,
            format!(
                "found only {have} crossing on the {what}, but this trim needs at least {need} \
                 crossings"
            ),
        )
    }
}

fn trim_line(line: &Line, boundaries: &[Curve], keep: TrimKeep) -> Result<TrimResult, Error> {
    let seg = Curve::Segment {
        a: p3_of(line.from),
        b: p3_of(line.to),
    };
    let ts = interior_crossings(&seg, boundaries);
    let mk = |t0: f64, t1: f64| {
        Object::Line(Line {
            from: point3_of(seg.point(t0)),
            to: point3_of(seg.point(t1)),
            layer: line.layer.clone(),
            properties: line.properties.clone(),
        })
    };
    match keep {
        TrimKeep::Start => {
            let Some(&t) = ts.first() else {
                return Err(need_crossings("line", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(0.0, t),
                split: None,
            })
        }
        TrimKeep::End => {
            let Some(&t) = ts.last() else {
                return Err(need_crossings("line", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(t, 1.0),
                split: None,
            })
        }
        TrimKeep::Between => {
            if ts.len() < 2 {
                return Err(need_crossings("line", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(ts[0], *ts.last().unwrap()),
                split: None,
            })
        }
        TrimKeep::Ends => {
            if ts.len() < 2 {
                return Err(need_crossings("line", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(0.0, ts[0]),
                split: Some(mk(*ts.last().unwrap(), 1.0)),
            })
        }
    }
}

fn arc_sweep_rad(arc: &Arc) -> (f64, f64) {
    let start = arc.start_angle.to_radians();
    let end = arc.end_angle.to_radians();
    let sweep = (end - start).rem_euclid(TAU);
    (start, if sweep == 0.0 { TAU } else { sweep })
}

fn trim_arc(arc: &Arc, boundaries: &[Curve], keep: TrimKeep) -> Result<TrimResult, Error> {
    let (start, sweep) = arc_sweep_rad(arc);
    let curve = Curve::Arc {
        center: p3_of(arc.center),
        radius: arc.radius,
        start,
        sweep,
    };
    let ts = interior_crossings(&curve, boundaries);
    let mk = |t0: f64, t1: f64| {
        Object::Arc(Arc {
            center: arc.center,
            radius: arc.radius,
            start_angle: (start + sweep * t0).to_degrees().rem_euclid(360.0),
            end_angle: (start + sweep * t1).to_degrees().rem_euclid(360.0),
            layer: arc.layer.clone(),
            properties: arc.properties.clone(),
        })
    };
    match keep {
        TrimKeep::Start => {
            let Some(&t) = ts.first() else {
                return Err(need_crossings("arc", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(0.0, t),
                split: None,
            })
        }
        TrimKeep::End => {
            let Some(&t) = ts.last() else {
                return Err(need_crossings("arc", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(t, 1.0),
                split: None,
            })
        }
        TrimKeep::Between => {
            if ts.len() < 2 {
                return Err(need_crossings("arc", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(ts[0], *ts.last().unwrap()),
                split: None,
            })
        }
        TrimKeep::Ends => {
            if ts.len() < 2 {
                return Err(need_crossings("arc", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(0.0, ts[0]),
                split: Some(mk(*ts.last().unwrap(), 1.0)),
            })
        }
    }
}

fn trim_circle(circle: &Circle, boundaries: &[Curve], keep: TrimKeep) -> Result<TrimResult, Error> {
    let curve = Curve::Arc {
        center: p3_of(circle.center),
        radius: circle.radius,
        start: 0.0,
        sweep: TAU,
    };
    let ts = closed_crossings(&curve, boundaries);
    if ts.len() < 2 {
        return Err(need_crossings("circle", ts.len(), 2));
    }
    let (a, b) = match keep {
        TrimKeep::Start | TrimKeep::Between => (ts[0], ts[1]),
        TrimKeep::End | TrimKeep::Ends => (ts[1], ts[0]),
    };
    Ok(TrimResult {
        kept: Object::Arc(Arc {
            center: circle.center,
            radius: circle.radius,
            start_angle: (a * 360.0).rem_euclid(360.0),
            end_angle: (b * 360.0).rem_euclid(360.0),
            layer: circle.layer.clone(),
            properties: circle.properties.clone(),
        }),
        split: None,
    })
}

fn poly_segment_curve(poly: &Polyline, i: usize) -> Option<Curve> {
    let n = poly.points.len();
    let p0 = poly.points[i];
    let p1 = poly.points[(i + 1) % n];
    let bulge = poly.bulge(i);
    if poly.is_flat() {
        if let Some(arc) = arc_of([p0.x, p0.y], [p1.x, p1.y], bulge) {
            return Some(Curve::Arc {
                center: [arc.center[0], arc.center[1], p0.z],
                radius: arc.radius,
                start: arc.start,
                sweep: arc.sweep,
            });
        }
    }
    let a = p3_of(p0);
    let b = p3_of(p1);
    if length_of(sub(b, a)) <= EPS {
        return None;
    }
    Some(Curve::Segment { a, b })
}

/// Slices a polyline's segment curves from global parameter `u_start` to `u_end`
/// (with `u_start < u_end`, where `u_end` may wrap past `n_segs` for closed
/// polylines).
fn slice_polyline(poly: &Polyline, curves: &[Option<Curve>], u_start: f64, u_end: f64) -> Polyline {
    let m = curves.len();
    let mut points: Vec<Point3> = Vec::new();
    let mut bulges: Vec<f64> = Vec::new();
    let mut cur = u_start;
    while cur < u_end - 1e-7 {
        let seg_total = cur.floor() as usize;
        let seg_idx = seg_total % m;
        let seg_start_u = seg_total as f64;
        let seg_end_u = (seg_total + 1) as f64;
        let t0 = (cur - seg_start_u).clamp(0.0, 1.0);
        let t1 = (u_end.min(seg_end_u) - seg_start_u).clamp(0.0, 1.0);
        if t1 > t0 + 1e-7 {
            if let Some(c) = &curves[seg_idx] {
                let p_a = point3_of(c.point(t0));
                let p_b = point3_of(c.point(t1));
                let b = match *c {
                    Curve::Arc { sweep, .. } => ((sweep * (t1 - t0)) / 4.0).tan(),
                    _ => 0.0,
                };
                if points.is_empty() {
                    points.push(p_a);
                    bulges.push(b);
                } else {
                    *bulges.last_mut().unwrap() = b;
                }
                points.push(p_b);
                bulges.push(0.0);
            }
        }
        cur = seg_end_u;
    }
    let has_bulge = bulges.iter().any(|&b| b.abs() > 1e-12);
    Polyline {
        points,
        bulges: if has_bulge { bulges } else { Vec::new() },
        widths: Vec::new(),
        width: poly.width,
        closed: false,
        layer: poly.layer.clone(),
        properties: poly.properties.clone(),
    }
}

fn trim_polyline(
    poly: &Polyline,
    boundaries: &[Curve],
    keep: TrimKeep,
) -> Result<TrimResult, Error> {
    let n = poly.points.len();
    let m = if poly.closed { n } else { n.saturating_sub(1) };
    if m == 0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "polyline has no segments to trim".to_owned(),
        ));
    }
    let curves: Vec<Option<Curve>> = (0..m).map(|i| poly_segment_curve(poly, i)).collect();
    let mut us: Vec<f64> = Vec::new();
    for (i, c) in curves.iter().enumerate() {
        let Some(c) = c else {
            continue;
        };
        for t in curve_crossings_with(c, boundaries) {
            let u = i as f64 + t.clamp(0.0, 1.0);
            if poly.closed {
                us.push(u.rem_euclid(m as f64));
            } else if u > 1e-6 && u < (m as f64) - 1e-6 {
                us.push(u);
            }
        }
    }
    dedup_params(&mut us, 1e-5);
    if poly.closed && us.len() >= 2 && ((m as f64) - (*us.last().unwrap() - us[0])).abs() <= 1e-5 {
        us.pop();
    }

    if poly.closed {
        if us.len() < 2 {
            return Err(need_crossings("closed polyline", us.len(), 2));
        }
        let (a, b) = match keep {
            TrimKeep::Start | TrimKeep::Between => (us[0], us[1]),
            TrimKeep::End | TrimKeep::Ends => (us[1], us[0] + m as f64),
        };
        return Ok(TrimResult {
            kept: Object::Polyline(slice_polyline(poly, &curves, a, b)),
            split: None,
        });
    }

    match keep {
        TrimKeep::Start => {
            let Some(&u) = us.first() else {
                return Err(need_crossings("polyline", 0, 1));
            };
            Ok(TrimResult {
                kept: Object::Polyline(slice_polyline(poly, &curves, 0.0, u)),
                split: None,
            })
        }
        TrimKeep::End => {
            let Some(&u) = us.last() else {
                return Err(need_crossings("polyline", 0, 1));
            };
            Ok(TrimResult {
                kept: Object::Polyline(slice_polyline(poly, &curves, u, m as f64)),
                split: None,
            })
        }
        TrimKeep::Between => {
            if us.len() < 2 {
                return Err(need_crossings("polyline", us.len(), 2));
            }
            Ok(TrimResult {
                kept: Object::Polyline(slice_polyline(poly, &curves, us[0], *us.last().unwrap())),
                split: None,
            })
        }
        TrimKeep::Ends => {
            if us.len() < 2 {
                return Err(need_crossings("polyline", us.len(), 2));
            }
            Ok(TrimResult {
                kept: Object::Polyline(slice_polyline(poly, &curves, 0.0, us[0])),
                split: Some(Object::Polyline(slice_polyline(
                    poly,
                    &curves,
                    *us.last().unwrap(),
                    m as f64,
                ))),
            })
        }
    }
}

fn ellipse_curve(e: &Ellipse) -> (Curve, f64, f64) {
    let major = [e.major_axis.x, e.major_axis.y];
    let minor = [-major[1] * e.ratio, major[0] * e.ratio];
    let (start, sweep) = if e.is_whole() {
        (0.0, TAU)
    } else {
        let s = ellipse_parameter(e.start_angle, e.ratio);
        let en = ellipse_parameter(e.end_angle, e.ratio);
        let sw = (en - s).rem_euclid(TAU);
        (s, if sw == 0.0 { TAU } else { sw })
    };
    (
        Curve::Ellipse {
            center: p3_of(e.center),
            major,
            minor,
            start,
            sweep,
        },
        start,
        sweep,
    )
}

fn trim_ellipse(e: &Ellipse, boundaries: &[Curve], keep: TrimKeep) -> Result<TrimResult, Error> {
    let (curve, start, sweep) = ellipse_curve(e);
    let mk = |p0: f64, p1: f64| {
        Object::Ellipse(Ellipse {
            center: e.center,
            major_axis: e.major_axis,
            ratio: e.ratio,
            start_angle: ellipse_degrees(p0, e.ratio),
            end_angle: ellipse_degrees(p1, e.ratio),
            layer: e.layer.clone(),
            properties: e.properties.clone(),
        })
    };
    if e.is_whole() {
        let ts = closed_crossings(&curve, boundaries);
        if ts.len() < 2 {
            return Err(need_crossings("ellipse", ts.len(), 2));
        }
        let (a, b) = match keep {
            TrimKeep::Start | TrimKeep::Between => (ts[0] * TAU, ts[1] * TAU),
            TrimKeep::End | TrimKeep::Ends => (ts[1] * TAU, ts[0] * TAU),
        };
        return Ok(TrimResult {
            kept: mk(a, b),
            split: None,
        });
    }
    let ts = interior_crossings(&curve, boundaries);
    match keep {
        TrimKeep::Start => {
            let Some(&t) = ts.first() else {
                return Err(need_crossings("ellipse", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(start, start + sweep * t),
                split: None,
            })
        }
        TrimKeep::End => {
            let Some(&t) = ts.last() else {
                return Err(need_crossings("ellipse", 0, 1));
            };
            Ok(TrimResult {
                kept: mk(start + sweep * t, start + sweep),
                split: None,
            })
        }
        TrimKeep::Between => {
            if ts.len() < 2 {
                return Err(need_crossings("ellipse", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(start + sweep * ts[0], start + sweep * *ts.last().unwrap()),
                split: None,
            })
        }
        TrimKeep::Ends => {
            if ts.len() < 2 {
                return Err(need_crossings("ellipse", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: mk(start, start + sweep * ts[0]),
                split: Some(mk(start + sweep * *ts.last().unwrap(), start + sweep)),
            })
        }
    }
}

fn slice_spline(spline: &Spline, curve: &Curve, u0: f64, u1: f64) -> Object {
    let steps = 32usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let u = u0 + (u1 - u0) * (i as f64) / (steps as f64);
        let p = point3_of(curve.point(u));
        if pts
            .last()
            .is_none_or(|prev: &Point3| (prev.x - p.x).hypot(prev.y - p.y) > 1e-7 || i == steps)
        {
            pts.push(p);
        }
    }
    Object::Spline(Spline {
        fit_points: pts,
        control_points: Vec::new(),
        degree: 3,
        layer: spline.layer.clone(),
        properties: spline.properties.clone(),
    })
}

fn trim_spline(spline: &Spline, boundaries: &[Curve], keep: TrimKeep) -> Result<TrimResult, Error> {
    let Some((degree, control, knots)) = spline_definition(spline) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "spline has too few points to trim".to_owned(),
        ));
    };
    let curve = Curve::Spline(BSpline::new(degree, control, knots));
    let (d0, d1) = curve.domain();
    let ts = interior_crossings(&curve, boundaries);
    match keep {
        TrimKeep::Start => {
            let Some(&u) = ts.first() else {
                return Err(need_crossings("spline", 0, 1));
            };
            Ok(TrimResult {
                kept: slice_spline(spline, &curve, d0, u),
                split: None,
            })
        }
        TrimKeep::End => {
            let Some(&u) = ts.last() else {
                return Err(need_crossings("spline", 0, 1));
            };
            Ok(TrimResult {
                kept: slice_spline(spline, &curve, u, d1),
                split: None,
            })
        }
        TrimKeep::Between => {
            if ts.len() < 2 {
                return Err(need_crossings("spline", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: slice_spline(spline, &curve, ts[0], *ts.last().unwrap()),
                split: None,
            })
        }
        TrimKeep::Ends => {
            if ts.len() < 2 {
                return Err(need_crossings("spline", ts.len(), 2));
            }
            Ok(TrimResult {
                kept: slice_spline(spline, &curve, d0, ts[0]),
                split: Some(slice_spline(spline, &curve, *ts.last().unwrap(), d1)),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// EXTEND
// ---------------------------------------------------------------------------

pub(crate) fn compute_extend(
    target: &Object,
    boundaries: &[Curve],
    end: ExtendEnd,
) -> Result<Object, Error> {
    if boundaries.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the boundary object has no geometry to extend to".to_owned(),
        ));
    }
    let ray_len = ray_reach(target, boundaries);
    match target {
        Object::Line(line) => extend_line(line, boundaries, end, ray_len),
        Object::Arc(arc) => extend_arc(arc, boundaries, end),
        Object::Polyline(poly) => extend_polyline(poly, boundaries, end, ray_len),
        Object::Ellipse(ellipse) => extend_ellipse(ellipse, boundaries, end),
        Object::Spline(spline) => extend_spline(spline, boundaries, end, ray_len),
        Object::Circle(_) => Err(Error::new(
            ErrorCode::BadValue,
            "a circle is closed and cannot be extended".to_owned(),
        )),
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`extend` works on an open line, polyline, arc, ellipse or spline, not a {}",
                other.type_name()
            ),
        )),
    }
}

/// Computes a safe finite ray length far larger than the combined extents of
/// `target` and `boundaries`.
fn ray_reach(target: &Object, boundaries: &[Curve]) -> f64 {
    let mut max_coord = 1000.0_f64;
    for b in boundaries {
        let (mn, mx) = b.extents();
        for v in mn.into_iter().chain(mx) {
            max_coord = max_coord.max(v.abs());
        }
    }
    match target {
        Object::Line(l) => {
            for v in [l.from.x, l.from.y, l.from.z, l.to.x, l.to.y, l.to.z] {
                max_coord = max_coord.max(v.abs());
            }
        }
        Object::Polyline(p) => {
            for pt in &p.points {
                for v in [pt.x, pt.y, pt.z] {
                    max_coord = max_coord.max(v.abs());
                }
            }
        }
        Object::Spline(s) => {
            for pt in s.fit_points.iter().chain(&s.control_points) {
                for v in [pt.x, pt.y, pt.z] {
                    max_coord = max_coord.max(v.abs());
                }
            }
        }
        _ => {}
    }
    max_coord * 100.0
}

fn nearest_ray_hit(origin: P3, unit_dir: P3, ray_len: f64, boundaries: &[Curve]) -> Option<P3> {
    let end = [
        origin[0] + unit_dir[0] * ray_len,
        origin[1] + unit_dir[1] * ray_len,
        origin[2] + unit_dir[2] * ray_len,
    ];
    let ray = Curve::Segment { a: origin, b: end };
    let min_t = 1e-6 / ray_len.max(1.0);
    let mut best: Option<(f64, P3)> = None;
    for b in boundaries {
        for (t, _) in crossings(&ray, b) {
            if t > min_t {
                let p = ray.point(t);
                if length_of(sub(p, origin)) > 1e-6 && best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn no_extend_hit(what: &str, which: &str) -> Error {
    Error::new(
        ErrorCode::BadValue,
        format!("extending the {which} of the {what} does not reach the boundary"),
    )
}

fn extend_line(
    line: &Line,
    boundaries: &[Curve],
    end: ExtendEnd,
    ray_len: f64,
) -> Result<Object, Error> {
    let a = p3_of(line.from);
    let b = p3_of(line.to);
    let d = sub(b, a);
    let len = length_of(d);
    if len <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot extend a zero-length line".to_owned(),
        ));
    }
    let dir = [d[0] / len, d[1] / len, d[2] / len];
    let mut from = line.from;
    let mut to = line.to;
    if matches!(end, ExtendEnd::Start | ExtendEnd::Both) {
        let rev = [-dir[0], -dir[1], -dir[2]];
        let hit = nearest_ray_hit(a, rev, ray_len, boundaries)
            .ok_or_else(|| no_extend_hit("line", "start"))?;
        from = point3_of(hit);
    }
    if matches!(end, ExtendEnd::End | ExtendEnd::Both) {
        let hit = nearest_ray_hit(b, dir, ray_len, boundaries)
            .ok_or_else(|| no_extend_hit("line", "end"))?;
        to = point3_of(hit);
    }
    Ok(Object::Line(Line {
        from,
        to,
        layer: line.layer.clone(),
        properties: line.properties.clone(),
    }))
}

fn extend_arc(arc: &Arc, boundaries: &[Curve], end: ExtendEnd) -> Result<Object, Error> {
    let (start, sweep) = arc_sweep_rad(arc);
    let rem = TAU - sweep;
    if rem <= 1e-6 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "arc is already a full circle and cannot be extended".to_owned(),
        ));
    }
    let mut new_start = start;
    let mut new_end = start + sweep;
    if matches!(end, ExtendEnd::Start | ExtendEnd::Both) {
        let back = Curve::Arc {
            center: p3_of(arc.center),
            radius: arc.radius,
            start,
            sweep: -rem,
        };
        let ts = interior_crossings(&back, boundaries);
        let Some(&t) = ts.first() else {
            return Err(no_extend_hit("arc", "start"));
        };
        new_start = start - rem * t;
    }
    if matches!(end, ExtendEnd::End | ExtendEnd::Both) {
        let fwd = Curve::Arc {
            center: p3_of(arc.center),
            radius: arc.radius,
            start: start + sweep,
            sweep: rem,
        };
        let ts = interior_crossings(&fwd, boundaries);
        let Some(&t) = ts.first() else {
            return Err(no_extend_hit("arc", "end"));
        };
        new_end = start + sweep + rem * t;
    }
    Ok(Object::Arc(Arc {
        center: arc.center,
        radius: arc.radius,
        start_angle: new_start.to_degrees().rem_euclid(360.0),
        end_angle: new_end.to_degrees().rem_euclid(360.0),
        layer: arc.layer.clone(),
        properties: arc.properties.clone(),
    }))
}

fn extend_polyline(
    poly: &Polyline,
    boundaries: &[Curve],
    end: ExtendEnd,
    ray_len: f64,
) -> Result<Object, Error> {
    if poly.closed {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a closed polyline has no open ends to extend".to_owned(),
        ));
    }
    if poly.points.len() < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "polyline has too few points to extend".to_owned(),
        ));
    }
    let mut out = poly.clone();
    if out.bulges.is_empty() && (0..out.points.len() - 1).any(|i| out.bulge(i).abs() > 1e-12) {
        out.bulges = vec![0.0; out.points.len()];
    }
    let last_seg = out.points.len() - 2;

    if matches!(end, ExtendEnd::Start | ExtendEnd::Both) {
        let b = out.bulge(0);
        if out.is_flat() && b.abs() > 1e-12 {
            let p0 = out.points[0];
            let p1 = out.points[1];
            let arc = arc_of([p0.x, p0.y], [p1.x, p1.y], b).unwrap();
            let rem = TAU - arc.sweep.abs();
            let ext_sweep = if arc.sweep >= 0.0 { -rem } else { rem };
            let ext_curve = Curve::Arc {
                center: [arc.center[0], arc.center[1], p0.z],
                radius: arc.radius,
                start: arc.start,
                sweep: ext_sweep,
            };
            let ts = interior_crossings(&ext_curve, boundaries);
            let Some(&t) = ts.first() else {
                return Err(no_extend_hit("polyline", "start"));
            };
            out.points[0] = point3_of(ext_curve.point(t));
            let added = rem * t;
            let new_sweep = if arc.sweep >= 0.0 {
                arc.sweep + added
            } else {
                arc.sweep - added
            };
            if out.bulges.len() < out.points.len() {
                out.bulges.resize(out.points.len(), 0.0);
            }
            out.bulges[0] = (new_sweep / 4.0).tan();
        } else {
            let a = p3_of(out.points[0]);
            let b_pt = p3_of(out.points[1]);
            let d = sub(a, b_pt);
            let len = length_of(d);
            if len <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "cannot extend a zero-length first segment".to_owned(),
                ));
            }
            let dir = [d[0] / len, d[1] / len, d[2] / len];
            let hit = nearest_ray_hit(a, dir, ray_len, boundaries)
                .ok_or_else(|| no_extend_hit("polyline", "start"))?;
            out.points[0] = point3_of(hit);
        }
    }

    if matches!(end, ExtendEnd::End | ExtendEnd::Both) {
        let b = out.bulge(last_seg);
        if out.is_flat() && b.abs() > 1e-12 {
            let p0 = out.points[last_seg];
            let p1 = out.points[last_seg + 1];
            let arc = arc_of([p0.x, p0.y], [p1.x, p1.y], b).unwrap();
            let rem = TAU - arc.sweep.abs();
            let ext_sweep = if arc.sweep >= 0.0 { rem } else { -rem };
            let ext_curve = Curve::Arc {
                center: [arc.center[0], arc.center[1], p0.z],
                radius: arc.radius,
                start: arc.start + arc.sweep,
                sweep: ext_sweep,
            };
            let ts = interior_crossings(&ext_curve, boundaries);
            let Some(&t) = ts.first() else {
                return Err(no_extend_hit("polyline", "end"));
            };
            out.points[last_seg + 1] = point3_of(ext_curve.point(t));
            let added = rem * t;
            let new_sweep = if arc.sweep >= 0.0 {
                arc.sweep + added
            } else {
                arc.sweep - added
            };
            if out.bulges.len() < out.points.len() {
                out.bulges.resize(out.points.len(), 0.0);
            }
            out.bulges[last_seg] = (new_sweep / 4.0).tan();
        } else {
            let a = p3_of(out.points[last_seg]);
            let b_pt = p3_of(out.points[last_seg + 1]);
            let d = sub(b_pt, a);
            let len = length_of(d);
            if len <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "cannot extend a zero-length last segment".to_owned(),
                ));
            }
            let dir = [d[0] / len, d[1] / len, d[2] / len];
            let hit = nearest_ray_hit(b_pt, dir, ray_len, boundaries)
                .ok_or_else(|| no_extend_hit("polyline", "end"))?;
            out.points[last_seg + 1] = point3_of(hit);
        }
    }

    Ok(Object::Polyline(out))
}

fn extend_ellipse(e: &Ellipse, boundaries: &[Curve], end: ExtendEnd) -> Result<Object, Error> {
    if e.is_whole() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a whole ellipse is closed and cannot be extended".to_owned(),
        ));
    }
    let major = [e.major_axis.x, e.major_axis.y];
    let minor = [-major[1] * e.ratio, major[0] * e.ratio];
    let (_, start, sweep) = ellipse_curve(e);
    let rem = TAU - sweep;
    let mut new_start = start;
    let mut new_end = start + sweep;

    if matches!(end, ExtendEnd::Start | ExtendEnd::Both) {
        // Sweep the remaining complement backwards (as a forward arc from
        // `start + sweep` to `start`, taking the LAST crossing closest to `start`).
        let comp = Curve::Ellipse {
            center: p3_of(e.center),
            major,
            minor,
            start: start + sweep,
            sweep: rem,
        };
        let ts = interior_crossings(&comp, boundaries);
        let Some(&t) = ts.last() else {
            return Err(no_extend_hit("ellipse", "start"));
        };
        new_start = start + sweep + rem * t;
    }
    if matches!(end, ExtendEnd::End | ExtendEnd::Both) {
        let comp = Curve::Ellipse {
            center: p3_of(e.center),
            major,
            minor,
            start: start + sweep,
            sweep: rem,
        };
        let ts = interior_crossings(&comp, boundaries);
        let Some(&t) = ts.first() else {
            return Err(no_extend_hit("ellipse", "end"));
        };
        new_end = start + sweep + rem * t;
    }
    Ok(Object::Ellipse(Ellipse {
        center: e.center,
        major_axis: e.major_axis,
        ratio: e.ratio,
        start_angle: ellipse_degrees(new_start, e.ratio),
        end_angle: ellipse_degrees(new_end, e.ratio),
        layer: e.layer.clone(),
        properties: e.properties.clone(),
    }))
}

fn extend_spline(
    spline: &Spline,
    boundaries: &[Curve],
    end: ExtendEnd,
    ray_len: f64,
) -> Result<Object, Error> {
    let Some((degree, control, knots)) = spline_definition(spline) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "spline has too few points to extend".to_owned(),
        ));
    };
    let curve = Curve::Spline(BSpline::new(degree, control, knots));
    let (d0, d1) = curve.domain();
    let p_start = curve.point(d0);
    let p_end = curve.point(d1);
    if length_of(sub(p_end, p_start)) <= 1e-6 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a closed spline has no open ends to extend".to_owned(),
        ));
    }
    let mut pts: Vec<Point3> = if !spline.fit_points.is_empty() {
        spline.fit_points.clone()
    } else {
        (0..=24)
            .map(|i| point3_of(curve.point(d0 + (d1 - d0) * (i as f64) / 24.0)))
            .collect()
    };

    if matches!(end, ExtendEnd::Start | ExtendEnd::Both) {
        let t0 = curve.tangent(d0);
        let len = length_of(t0);
        if len <= EPS {
            return Err(Error::new(
                ErrorCode::BadValue,
                "cannot extend a spline with zero start tangent".to_owned(),
            ));
        }
        let dir = [-t0[0] / len, -t0[1] / len, -t0[2] / len];
        let hit = nearest_ray_hit(p_start, dir, ray_len, boundaries)
            .ok_or_else(|| no_extend_hit("spline", "start"))?;
        pts.insert(0, point3_of(hit));
    }
    if matches!(end, ExtendEnd::End | ExtendEnd::Both) {
        let t1 = curve.tangent(d1);
        let len = length_of(t1);
        if len <= EPS {
            return Err(Error::new(
                ErrorCode::BadValue,
                "cannot extend a spline with zero end tangent".to_owned(),
            ));
        }
        let dir = [t1[0] / len, t1[1] / len, t1[2] / len];
        let hit = nearest_ray_hit(p_end, dir, ray_len, boundaries)
            .ok_or_else(|| no_extend_hit("spline", "end"))?;
        pts.push(point3_of(hit));
    }
    Ok(Object::Spline(Spline {
        fit_points: pts,
        control_points: Vec::new(),
        degree: 3,
        layer: spline.layer.clone(),
        properties: spline.properties.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Properties;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    fn vline(x: f64) -> Curve {
        Curve::Segment {
            a: [x, -50.0, 0.0],
            b: [x, 50.0, 0.0],
        }
    }

    #[test]
    fn trim_line_start_end_between_and_ends_split() {
        let line = Object::Line(Line {
            from: Point3::new(0.0, 0.0, 0.0),
            to: Point3::new(100.0, 0.0, 0.0),
            layer: "wires".to_owned(),
            properties: Properties::default(),
        });
        let bounds = vec![vline(30.0), vline(70.0)];

        let r_start = compute_trim(&line, &bounds, TrimKeep::Start).unwrap();
        let Object::Line(l) = r_start.kept else {
            panic!()
        };
        close(l.from.x, 0.0);
        close(l.to.x, 30.0);
        assert!(r_start.split.is_none());

        let r_end = compute_trim(&line, &bounds, TrimKeep::End).unwrap();
        let Object::Line(l) = r_end.kept else {
            panic!()
        };
        close(l.from.x, 70.0);
        close(l.to.x, 100.0);

        let r_between = compute_trim(&line, &bounds, TrimKeep::Between).unwrap();
        let Object::Line(l) = r_between.kept else {
            panic!()
        };
        close(l.from.x, 30.0);
        close(l.to.x, 70.0);

        let r_split = compute_trim(&line, &bounds, TrimKeep::Ends).unwrap();
        let Object::Line(p1) = r_split.kept else {
            panic!()
        };
        let Object::Line(p2) = r_split.split.unwrap() else {
            panic!()
        };
        close(p1.from.x, 0.0);
        close(p1.to.x, 30.0);
        close(p2.from.x, 70.0);
        close(p2.to.x, 100.0);
    }

    #[test]
    fn trim_circle_and_arc() {
        let circle = Object::Circle(Circle {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 10.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        // Horizontal line y = 0 cuts circle at 0 deg and 180 deg.
        let horiz = vec![Curve::Segment {
            a: [-20.0, 0.0, 0.0],
            b: [20.0, 0.0, 0.0],
        }];
        let top = compute_trim(&circle, &horiz, TrimKeep::Start).unwrap();
        let Object::Arc(a) = top.kept else { panic!() };
        close(a.start_angle, 0.0);
        close(a.end_angle, 180.0);

        let bottom = compute_trim(&circle, &horiz, TrimKeep::End).unwrap();
        let Object::Arc(b) = bottom.kept else {
            panic!()
        };
        close(b.start_angle, 180.0);
        close(b.end_angle, 0.0);
    }

    #[test]
    fn trim_polyline_with_bulge_and_split() {
        let poly = Object::Polyline(Polyline {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(20.0, 0.0, 0.0),
                Point3::new(20.0, 20.0, 0.0),
            ],
            bulges: vec![0.0, 1.0, 0.0],
            widths: Vec::new(),
            width: 0.0,
            closed: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let r = compute_trim(&poly, &[vline(10.0)], TrimKeep::Start).unwrap();
        let Object::Polyline(p) = r.kept else {
            panic!()
        };
        assert_eq!(p.points.len(), 2);
        close(p.points[1].x, 10.0);
    }

    #[test]
    fn extend_line_arc_and_polyline() {
        let line = Object::Line(Line {
            from: Point3::new(10.0, 0.0, 0.0),
            to: Point3::new(40.0, 0.0, 0.0),
            layer: "wires".to_owned(),
            properties: Properties::default(),
        });
        let bounds = vec![vline(0.0), vline(80.0)];
        let Object::Line(ext) = compute_extend(&line, &bounds, ExtendEnd::Both).unwrap() else {
            panic!()
        };
        close(ext.from.x, 0.0);
        close(ext.to.x, 80.0);

        let arc = Object::Arc(Arc {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 10.0,
            start_angle: 30.0,
            end_angle: 60.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let axes = vec![
            Curve::Segment {
                a: [-20.0, 0.0, 0.0],
                b: [20.0, 0.0, 0.0],
            },
            Curve::Segment {
                a: [0.0, -20.0, 0.0],
                b: [0.0, 20.0, 0.0],
            },
        ];
        let Object::Arc(ea) = compute_extend(&arc, &axes, ExtendEnd::Both).unwrap() else {
            panic!()
        };
        close(ea.start_angle, 0.0);
        close(ea.end_angle, 90.0);
    }
}
