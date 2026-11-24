//! Geometric break (`break` / `BREAK` / `BR` / `BREAKATPOINT`) and join
//! (`join` / `JOIN` / `J`) operations.

use std::f64::consts::{PI, TAU};

use crate::draw::curves::{ellipse_parameter, spline_definition};
use crate::draw::polyline::arc_of;
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::{BSpline, Curve, P3};
use crate::model::{Arc, Circle, Ellipse, Line, Object, Point3, Polyline, Spline};

const EPS: f64 = 1e-7;
const Z_TOL: f64 = 1e-6;
pub(crate) const DEFAULT_JOIN_TOLERANCE: f64 = 1e-4;

/// How a `break` operation specifies where to cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BreakSpec {
    /// Split at `at` (`gap: None`, AutoCAD's `BREAKATPOINT`) or cut a centered
    /// gap of total arc length `gap` around `at` (`gap: Some(len)`).
    AtPoint { at: [f64; 3], gap: Option<f64> },
    /// Cut the span between `from` and `to` (AutoCAD's two-point `BREAK`).
    Between { from: [f64; 3], to: [f64; 3] },
}

/// Output of [`compute_break`]: `kept` carries on `target`'s ID; `split` (if
/// present) is the newly created second piece.
#[derive(Clone, Debug)]
pub(crate) struct BreakResult {
    pub(crate) kept: Object,
    pub(crate) split: Option<Object>,
}

#[derive(Clone, Copy, Debug)]
struct V2(f64, f64);

impl V2 {
    fn from_p3(p: Point3) -> Self {
        Self(p.x, p.y)
    }

    fn from_arr(p: [f64; 3]) -> Self {
        Self(p[0], p[1])
    }

    fn to_p3(self, z: f64) -> Point3 {
        Point3::new(self.0, self.1, z)
    }

    fn add(self, o: Self) -> Self {
        Self(self.0 + o.0, self.1 + o.1)
    }

    fn sub(self, o: Self) -> Self {
        Self(self.0 - o.0, self.1 - o.1)
    }

    fn scale(self, s: f64) -> Self {
        Self(self.0 * s, self.1 * s)
    }

    fn dot(self, o: Self) -> f64 {
        self.0 * o.0 + self.1 * o.1
    }

    fn cross(self, o: Self) -> f64 {
        self.0 * o.1 - self.1 * o.0
    }

    fn len(self) -> f64 {
        self.0.hypot(self.1)
    }

    fn dist(self, o: Self) -> f64 {
        self.sub(o).len()
    }

    fn unit(self) -> Option<Self> {
        let l = self.len();
        if l <= EPS {
            None
        } else {
            Some(self.scale(1.0 / l))
        }
    }
}

fn norm_deg(a: f64) -> f64 {
    let d = a.rem_euclid(360.0);
    if (d - 360.0).abs() <= 1e-12 || d.abs() <= 1e-12 {
        0.0
    } else {
        d
    }
}

fn angle_deg(c: V2, p: V2) -> f64 {
    norm_deg((p.1 - c.1).atan2(p.0 - c.0).to_degrees())
}

fn arc_point(c: V2, r: f64, deg: f64) -> V2 {
    let (s, cs) = deg.to_radians().sin_cos();
    V2(c.0 + r * cs, c.1 + r * s)
}

fn arc_sweep_deg(start: f64, end: f64) -> f64 {
    let d = (end - start).rem_euclid(360.0);
    if d <= 1e-9 {
        360.0
    } else {
        d
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_p3(a: Point3, b: Point3, t: f64) -> Point3 {
    Point3::new(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.z, b.z, t))
}

fn ellipse_degrees(param: f64, ratio: f64) -> f64 {
    let (s, c) = param.sin_cos();
    norm_deg((ratio * s).atan2(c).to_degrees())
}

// ---------------------------------------------------------------------------
// BREAK
// ---------------------------------------------------------------------------

pub(crate) fn compute_break(target: &Object, spec: BreakSpec) -> Result<BreakResult, Error> {
    if let BreakSpec::AtPoint { gap: Some(g), .. } = spec {
        if !g.is_finite() || g <= 0.0 {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`gap` must be a positive length".to_owned(),
            ));
        }
    }
    match target {
        Object::Line(line) => break_line(line, spec),
        Object::Arc(arc) => break_arc(arc, spec),
        Object::Circle(circle) => break_circle(circle, spec),
        Object::Polyline(poly) => break_polyline(poly, spec),
        Object::Ellipse(ellipse) => break_ellipse(ellipse, spec),
        Object::Spline(spline) => break_spline(spline, spec),
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`break` works on a line, polyline, arc, circle, ellipse or spline, not a {}",
                other.type_name()
            ),
        )),
    }
}

type Span = (f64, f64);

/// Resolves an open-curve cut `[s1, s2]` in `[0, total]` into 1 or 2 surviving
/// sub-intervals.
fn open_break_spans(
    total: f64,
    s1: f64,
    s2: f64,
    what: &str,
) -> Result<(Span, Option<Span>), Error> {
    let tol = (total * 1e-6).max(EPS);
    let a = s1.min(s2).clamp(0.0, total);
    let b = s1.max(s2).clamp(0.0, total);
    let has_left = a > tol;
    let has_right = total - b > tol;
    match (has_left, has_right) {
        (false, false) => Err(Error::new(
            ErrorCode::BadValue,
            format!("break consumes the entire {what}; nothing left to keep"),
        )),
        (true, false) => Ok(((0.0, a), None)),
        (false, true) => Ok(((b, total), None)),
        (true, true) => Ok(((0.0, a), Some((b, total)))),
    }
}

fn break_line(line: &Line, spec: BreakSpec) -> Result<BreakResult, Error> {
    let dx = line.to.x - line.from.x;
    let dy = line.to.y - line.from.y;
    let dz = line.to.z - line.from.z;
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a zero-length line".to_owned(),
        ));
    }
    let proj = |p: [f64; 3]| -> f64 {
        let t = ((p[0] - line.from.x) * dx + (p[1] - line.from.y) * dy + (p[2] - line.from.z) * dz)
            / (len * len);
        t.clamp(0.0, 1.0) * len
    };
    let (s1, s2) = match spec {
        BreakSpec::AtPoint { at, gap: None } => {
            let s = proj(at);
            if s <= EPS || len - s <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "break point lies at an endpoint of the line; pick an interior point"
                        .to_owned(),
                ));
            }
            (s, s)
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            let s = proj(at);
            (s - g * 0.5, s + g * 0.5)
        }
        BreakSpec::Between { from, to } => (proj(from), proj(to)),
    };
    let ((a0, a1), second) = open_break_spans(len, s1, s2, "line")?;
    let make = |u0: f64, u1: f64| {
        Object::Line(Line {
            from: lerp_p3(line.from, line.to, u0 / len),
            to: lerp_p3(line.from, line.to, u1 / len),
            layer: line.layer.clone(),
            properties: line.properties.clone(),
        })
    };
    Ok(BreakResult {
        kept: make(a0, a1),
        split: second.map(|(b0, b1)| make(b0, b1)),
    })
}

fn break_arc(arc: &Arc, spec: BreakSpec) -> Result<BreakResult, Error> {
    if arc.radius <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a zero-radius arc".to_owned(),
        ));
    }
    let c = V2::from_p3(arc.center);
    let sweep = arc_sweep_deg(arc.start_angle, arc.end_angle);
    let total = arc.radius * sweep.to_radians();
    let proj = |p: [f64; 3]| -> f64 {
        let deg = angle_deg(c, V2::from_arr(p));
        let rel = (deg - arc.start_angle).rem_euclid(360.0);
        let clamped_deg = if rel <= sweep {
            rel
        } else {
            // Closer to end or start?
            let d_end = rel - sweep;
            let d_start = 360.0 - rel;
            if d_start <= d_end {
                0.0
            } else {
                sweep
            }
        };
        arc.radius * clamped_deg.to_radians()
    };
    let (s1, s2) = match spec {
        BreakSpec::AtPoint { at, gap: None } => {
            let s = proj(at);
            if s <= EPS || total - s <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "break point lies at an endpoint of the arc; pick an interior point".to_owned(),
                ));
            }
            (s, s)
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            let s = proj(at);
            (s - g * 0.5, s + g * 0.5)
        }
        BreakSpec::Between { from, to } => (proj(from), proj(to)),
    };
    let ((a0, a1), second) = open_break_spans(total, s1, s2, "arc")?;
    let deg_at = |s: f64| norm_deg(arc.start_angle + (s / arc.radius).to_degrees());
    let make = |u0: f64, u1: f64| {
        Object::Arc(Arc {
            center: arc.center,
            radius: arc.radius,
            start_angle: deg_at(u0),
            end_angle: deg_at(u1),
            layer: arc.layer.clone(),
            properties: arc.properties.clone(),
        })
    };
    Ok(BreakResult {
        kept: make(a0, a1),
        split: second.map(|(b0, b1)| make(b0, b1)),
    })
}

fn break_circle(circle: &Circle, spec: BreakSpec) -> Result<BreakResult, Error> {
    if circle.radius <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a zero-radius circle".to_owned(),
        ));
    }
    let c = V2::from_p3(circle.center);
    let circ = TAU * circle.radius;
    let (start_angle, end_angle) = match spec {
        BreakSpec::AtPoint { gap: None, .. } => {
            return Err(Error::new(
                ErrorCode::BadValue,
                "cannot break a circle at a single point with zero gap; give `gap` or `from` and \
                 `to`"
                    .to_owned(),
            ));
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            if g >= circ - EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "gap consumes the entire circle; nothing left to keep".to_owned(),
                ));
            }
            let center_deg = angle_deg(c, V2::from_arr(at));
            let half_deg = (g / (2.0 * circle.radius)).to_degrees();
            (
                norm_deg(center_deg + half_deg),
                norm_deg(center_deg - half_deg),
            )
        }
        BreakSpec::Between { from, to } => {
            let a_from = angle_deg(c, V2::from_arr(from));
            let a_to = angle_deg(c, V2::from_arr(to));
            let cut = (a_to - a_from).rem_euclid(360.0);
            if cut <= 1e-4 || (360.0 - cut).abs() <= 1e-4 {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "`from` and `to` name the same point on the circle; give two distinct points"
                        .to_owned(),
                ));
            }
            // AutoCAD BREAK removes the anticlockwise span from `from` to `to`,
            // keeping the arc from `to` anticlockwise to `from`.
            (a_to, a_from)
        }
    };
    Ok(BreakResult {
        kept: Object::Arc(Arc {
            center: circle.center,
            radius: circle.radius,
            start_angle,
            end_angle,
            layer: circle.layer.clone(),
            properties: circle.properties.clone(),
        }),
        split: None,
    })
}

/// Segment metadata along a flat polyline for arc-length projection & slicing.
#[derive(Clone, Copy)]
struct PolySeg {
    a: V2,
    b: V2,
    bulge: f64,
    w: [f64; 2],
    len: f64,
    s0: f64,
}

fn polyline_segments(poly: &Polyline) -> Result<(Vec<PolySeg>, f64, f64), Error> {
    if !poly.is_flat() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`break` requires a flat polyline (constant Z)".to_owned(),
        ));
    }
    let n = poly.points.len();
    let seg_count = if poly.closed { n } else { n.saturating_sub(1) };
    if seg_count == 0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a degenerate polyline".to_owned(),
        ));
    }
    let z = poly.points[0].z;
    let mut segs = Vec::with_capacity(seg_count);
    let mut accum = 0.0;
    for i in 0..seg_count {
        let a = V2::from_p3(poly.points[i]);
        let b = V2::from_p3(poly.points[(i + 1) % n]);
        let bulge = poly.bulge(i);
        let len = match arc_of([a.0, a.1], [b.0, b.1], bulge) {
            Some(arc) => arc.radius * arc.sweep.abs(),
            None => a.dist(b),
        };
        segs.push(PolySeg {
            a,
            b,
            bulge,
            w: poly.widths_at(i),
            len,
            s0: accum,
        });
        accum += len;
    }
    if accum <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a zero-length polyline".to_owned(),
        ));
    }
    Ok((segs, accum, z))
}

/// Evaluates point at local fraction `t in [0, 1]` on a `PolySeg`.
fn poly_seg_point(seg: &PolySeg, t: f64) -> V2 {
    match arc_of([seg.a.0, seg.a.1], [seg.b.0, seg.b.1], seg.bulge) {
        Some(arc) => {
            let ang = arc.start + arc.sweep * t;
            let (s, c) = ang.sin_cos();
            V2(
                arc.center[0] + arc.radius * c,
                arc.center[1] + arc.radius * s,
            )
        }
        None => seg.a.add(seg.b.sub(seg.a).scale(t)),
    }
}

/// Projects `p` onto a polyline's segments and returns its global arc-length `s`.
fn project_onto_polyline(segs: &[PolySeg], p: V2) -> f64 {
    let mut best_dist = f64::INFINITY;
    let mut best_s = 0.0;
    for seg in segs {
        if seg.len <= EPS {
            continue;
        }
        let (t, d) = match arc_of([seg.a.0, seg.a.1], [seg.b.0, seg.b.1], seg.bulge) {
            Some(arc) => {
                let c = V2(arc.center[0], arc.center[1]);
                let ang = (p.1 - c.1).atan2(p.0 - c.0);
                let rel = if arc.sweep >= 0.0 {
                    (ang - arc.start).rem_euclid(TAU)
                } else {
                    -(arc.start - ang).rem_euclid(TAU)
                };
                let t_cand = (rel / arc.sweep).clamp(0.0, 1.0);
                let pt_cand = poly_seg_point(seg, t_cand);
                let d_cand = p.dist(pt_cand);
                let d0 = p.dist(seg.a);
                let d1 = p.dist(seg.b);
                if d0 <= d_cand && d0 <= d1 {
                    (0.0, d0)
                } else if d1 <= d_cand {
                    (1.0, d1)
                } else {
                    (t_cand, d_cand)
                }
            }
            None => {
                let ab = seg.b.sub(seg.a);
                let t = (p.sub(seg.a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
                let pt = seg.a.add(ab.scale(t));
                (t, p.dist(pt))
            }
        };
        if d < best_dist {
            best_dist = d;
            best_s = seg.s0 + t * seg.len;
        }
    }
    best_s
}

/// Extracts an open polyline over `[s_start, s_end]` (with `0 <= s_start < s_end <= total`).
fn slice_polyline_linear(
    poly: &Polyline,
    segs: &[PolySeg],
    z: f64,
    s_start: f64,
    s_end: f64,
) -> Polyline {
    let has_widths = !poly.widths.is_empty();
    let mut pts: Vec<Point3> = Vec::new();
    let mut bulges: Vec<f64> = Vec::new();
    let mut widths: Vec<[f64; 2]> = Vec::new();

    for seg in segs {
        if seg.len <= EPS {
            continue;
        }
        let seg_s1 = seg.s0 + seg.len;
        let lo = s_start.max(seg.s0);
        let hi = s_end.min(seg_s1);
        if hi - lo <= EPS {
            continue;
        }
        let t0 = ((lo - seg.s0) / seg.len).clamp(0.0, 1.0);
        let t1 = ((hi - seg.s0) / seg.len).clamp(0.0, 1.0);
        let p0 = poly_seg_point(seg, t0);
        let p1 = poly_seg_point(seg, t1);
        let sub_bulge = match arc_of([seg.a.0, seg.a.1], [seg.b.0, seg.b.1], seg.bulge) {
            Some(arc) => (arc.sweep * (t1 - t0) * 0.25).tan(),
            None => 0.0,
        };
        if pts.is_empty() {
            pts.push(p0.to_p3(z));
        }
        bulges.push(sub_bulge);
        if has_widths {
            widths.push([lerp(seg.w[0], seg.w[1], t0), lerp(seg.w[0], seg.w[1], t1)]);
        }
        pts.push(p1.to_p3(z));
    }
    bulges.push(0.0);
    let out_bulges = if bulges.iter().any(|b| b.abs() > 1e-12) {
        bulges
    } else {
        Vec::new()
    };
    Polyline {
        points: pts,
        bulges: out_bulges,
        widths,
        width: poly.width,
        closed: false,
        layer: poly.layer.clone(),
        properties: poly.properties.clone(),
    }
}

/// Extracts an open polyline from a closed polyline running forward from `s_start`
/// around the loop to `s_end` (wrapping past `total` when `s_end <= s_start`).
fn slice_closed_polyline(
    poly: &Polyline,
    segs: &[PolySeg],
    total: f64,
    z: f64,
    s_start: f64,
    s_end: f64,
) -> Polyline {
    if s_end > s_start + EPS {
        return slice_polyline_linear(poly, segs, z, s_start, s_end);
    }
    // Concatenate [s_start, total] followed by [0, s_end].
    let first = if total - s_start > EPS {
        Some(slice_polyline_linear(poly, segs, z, s_start, total))
    } else {
        None
    };
    let second = if s_end > EPS {
        Some(slice_polyline_linear(poly, segs, z, 0.0, s_end))
    } else {
        None
    };
    match (first, second) {
        (Some(mut a), Some(b)) => {
            let has_widths = !poly.widths.is_empty();
            let a_has_b = !a.bulges.is_empty();
            let b_has_b = !b.bulges.is_empty();
            let need_b = a_has_b || b_has_b;
            let mut bulges = if need_b {
                let mut v = if a_has_b {
                    a.bulges[..a.points.len() - 1].to_vec()
                } else {
                    vec![0.0; a.points.len() - 1]
                };
                if b_has_b {
                    v.extend_from_slice(&b.bulges[..b.points.len() - 1]);
                } else {
                    v.extend(std::iter::repeat_n(0.0, b.points.len() - 1));
                }
                v.push(0.0);
                v
            } else {
                Vec::new()
            };
            a.points.extend_from_slice(&b.points[1..]);
            if has_widths {
                a.widths.extend(b.widths);
            }
            if !bulges.iter().any(|x| x.abs() > 1e-12) {
                bulges.clear();
            }
            a.bulges = bulges;
            a
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => unreachable!("non-empty remaining span checked before"),
    }
}

fn break_polyline(poly: &Polyline, spec: BreakSpec) -> Result<BreakResult, Error> {
    let (segs, total, z) = polyline_segments(poly)?;
    if poly.closed {
        let (keep_start, keep_end) = match spec {
            BreakSpec::AtPoint { at, gap: None } => {
                let s = project_onto_polyline(&segs, V2::from_arr(at)).rem_euclid(total);
                // Open the closed polyline at `s` (runs from s around to s).
                (s, s)
            }
            BreakSpec::AtPoint { at, gap: Some(g) } => {
                if g >= total - EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "gap consumes the entire closed polyline; nothing left to keep".to_owned(),
                    ));
                }
                let s = project_onto_polyline(&segs, V2::from_arr(at));
                (
                    (s + g * 0.5).rem_euclid(total),
                    (s - g * 0.5).rem_euclid(total),
                )
            }
            BreakSpec::Between { from, to } => {
                let s_from = project_onto_polyline(&segs, V2::from_arr(from)).rem_euclid(total);
                let s_to = project_onto_polyline(&segs, V2::from_arr(to)).rem_euclid(total);
                let cut = (s_to - s_from).rem_euclid(total);
                if cut <= EPS || (total - cut).abs() <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "`from` and `to` name the same point on the closed polyline".to_owned(),
                    ));
                }
                (s_to, s_from)
            }
        };
        let opened = slice_closed_polyline(poly, &segs, total, z, keep_start, keep_end);
        return Ok(BreakResult {
            kept: Object::Polyline(opened),
            split: None,
        });
    }

    let (s1, s2) = match spec {
        BreakSpec::AtPoint { at, gap: None } => {
            let s = project_onto_polyline(&segs, V2::from_arr(at));
            if s <= EPS || total - s <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "break point lies at an endpoint of the polyline; pick an interior point"
                        .to_owned(),
                ));
            }
            (s, s)
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            let s = project_onto_polyline(&segs, V2::from_arr(at));
            (s - g * 0.5, s + g * 0.5)
        }
        BreakSpec::Between { from, to } => (
            project_onto_polyline(&segs, V2::from_arr(from)),
            project_onto_polyline(&segs, V2::from_arr(to)),
        ),
    };
    let ((a0, a1), second) = open_break_spans(total, s1, s2, "polyline")?;
    Ok(BreakResult {
        kept: Object::Polyline(slice_polyline_linear(poly, &segs, z, a0, a1)),
        split: second
            .map(|(b0, b1)| Object::Polyline(slice_polyline_linear(poly, &segs, z, b0, b1))),
    })
}

fn break_ellipse(ellipse: &Ellipse, spec: BreakSpec) -> Result<BreakResult, Error> {
    let major = V2::from_p3(ellipse.major_axis);
    let a_len = major.len();
    if a_len <= EPS || ellipse.ratio <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a degenerate ellipse".to_owned(),
        ));
    }
    let ux = V2(major.0 / a_len, major.1 / a_len);
    let uy = V2(-ux.1, ux.0);
    let b_len = a_len * ellipse.ratio;
    let c = V2::from_p3(ellipse.center);

    // Build an arc-length lookup table over the ellipse's parameter domain.
    let (p_start, p_sweep) = if ellipse.is_whole() {
        (0.0, TAU)
    } else {
        let p0 = ellipse_parameter(ellipse.start_angle, ellipse.ratio);
        let p1 = ellipse_parameter(ellipse.end_angle, ellipse.ratio);
        let mut sw = (p1 - p0).rem_euclid(TAU);
        if sw <= 1e-9 {
            sw = TAU;
        }
        (p0, sw)
    };
    const STEPS: usize = 512;
    let eval_param = |p: f64| -> V2 {
        let (s, cs) = p.sin_cos();
        c.add(ux.scale(a_len * cs)).add(uy.scale(b_len * s))
    };
    let mut params = Vec::with_capacity(STEPS + 1);
    let mut cum_len = Vec::with_capacity(STEPS + 1);
    let mut prev = eval_param(p_start);
    params.push(p_start);
    cum_len.push(0.0);
    let mut total = 0.0;
    for i in 1..=STEPS {
        let p = p_start + p_sweep * (i as f64) / (STEPS as f64);
        let pt = eval_param(p);
        total += prev.dist(pt);
        params.push(p);
        cum_len.push(total);
        prev = pt;
    }

    let proj_s = |pt_arr: [f64; 3]| -> f64 {
        let target = V2::from_arr(pt_arr);
        let mut best_i = 0;
        let mut best_d = f64::INFINITY;
        for (i, &p) in params.iter().enumerate() {
            let d = eval_param(p).dist(target);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        cum_len[best_i]
    };

    let param_at_s = |s: f64| -> f64 {
        let s = s.clamp(0.0, total);
        let idx = cum_len.partition_point(|&x| x < s);
        if idx == 0 {
            return params[0];
        }
        if idx >= cum_len.len() {
            return *params.last().unwrap();
        }
        let s0 = cum_len[idx - 1];
        let s1 = cum_len[idx];
        let frac = if (s1 - s0).abs() <= EPS {
            0.0
        } else {
            (s - s0) / (s1 - s0)
        };
        lerp(params[idx - 1], params[idx], frac)
    };

    let make = |pa: f64, pb: f64| {
        Object::Ellipse(Ellipse {
            center: ellipse.center,
            major_axis: ellipse.major_axis,
            ratio: ellipse.ratio,
            start_angle: ellipse_degrees(pa, ellipse.ratio),
            end_angle: ellipse_degrees(pb, ellipse.ratio),
            layer: ellipse.layer.clone(),
            properties: ellipse.properties.clone(),
        })
    };

    if ellipse.is_whole() {
        let (sa, sb) = match spec {
            BreakSpec::AtPoint { gap: None, .. } => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "cannot break a whole ellipse at a single point with zero gap; give `gap` or \
                     `from` and `to`"
                        .to_owned(),
                ));
            }
            BreakSpec::AtPoint { at, gap: Some(g) } => {
                if g >= total - EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "gap consumes the entire ellipse; nothing left to keep".to_owned(),
                    ));
                }
                let s = proj_s(at);
                (
                    (s + g * 0.5).rem_euclid(total),
                    (s - g * 0.5).rem_euclid(total),
                )
            }
            BreakSpec::Between { from, to } => {
                let sf = proj_s(from).rem_euclid(total);
                let st = proj_s(to).rem_euclid(total);
                let cut = (st - sf).rem_euclid(total);
                if cut <= EPS || (total - cut).abs() <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "`from` and `to` name the same point on the ellipse".to_owned(),
                    ));
                }
                (st, sf)
            }
        };
        return Ok(BreakResult {
            kept: make(param_at_s(sa), param_at_s(sb)),
            split: None,
        });
    }

    let (s1, s2) = match spec {
        BreakSpec::AtPoint { at, gap: None } => {
            let s = proj_s(at);
            if s <= EPS || total - s <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "break point lies at an endpoint of the elliptical arc".to_owned(),
                ));
            }
            (s, s)
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            let s = proj_s(at);
            (s - g * 0.5, s + g * 0.5)
        }
        BreakSpec::Between { from, to } => (proj_s(from), proj_s(to)),
    };
    let ((a0, a1), second) = open_break_spans(total, s1, s2, "elliptical arc")?;
    Ok(BreakResult {
        kept: make(param_at_s(a0), param_at_s(a1)),
        split: second.map(|(b0, b1)| make(param_at_s(b0), param_at_s(b1))),
    })
}

fn break_spline(spline: &Spline, spec: BreakSpec) -> Result<BreakResult, Error> {
    let Some((degree, control, knots)) = spline_definition(spline) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a degenerate spline".to_owned(),
        ));
    };
    let curve = Curve::Spline(BSpline::new(degree, control, knots));
    let (d0, d1) = curve.domain();
    const STEPS: usize = 512;
    let mut ts = Vec::with_capacity(STEPS + 1);
    let mut cum_len = Vec::with_capacity(STEPS + 1);
    let mut prev: P3 = curve.point(d0);
    ts.push(d0);
    cum_len.push(0.0);
    let mut total = 0.0;
    for i in 1..=STEPS {
        let t = d0 + (d1 - d0) * (i as f64) / (STEPS as f64);
        let pt = curve.point(t);
        let seg =
            ((pt[0] - prev[0]).powi(2) + (pt[1] - prev[1]).powi(2) + (pt[2] - prev[2]).powi(2))
                .sqrt();
        total += seg;
        ts.push(t);
        cum_len.push(total);
        prev = pt;
    }
    if total <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "cannot break a zero-length spline".to_owned(),
        ));
    }
    let proj_s = |p: [f64; 3]| -> f64 {
        let mut best_i = 0;
        let mut best_d = f64::INFINITY;
        for (i, &t) in ts.iter().enumerate() {
            let q = curve.point(t);
            let d = (q[0] - p[0]).hypot(q[1] - p[1]);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        cum_len[best_i]
    };
    let t_at_s = |s: f64| -> f64 {
        let s = s.clamp(0.0, total);
        let idx = cum_len.partition_point(|&x| x < s);
        if idx == 0 {
            return ts[0];
        }
        if idx >= cum_len.len() {
            return *ts.last().unwrap();
        }
        let s0 = cum_len[idx - 1];
        let s1 = cum_len[idx];
        let frac = if (s1 - s0).abs() <= EPS {
            0.0
        } else {
            (s - s0) / (s1 - s0)
        };
        lerp(ts[idx - 1], ts[idx], frac)
    };

    let (s1, s2) = match spec {
        BreakSpec::AtPoint { at, gap: None } => {
            let s = proj_s(at);
            if s <= EPS || total - s <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "break point lies at an endpoint of the spline".to_owned(),
                ));
            }
            (s, s)
        }
        BreakSpec::AtPoint { at, gap: Some(g) } => {
            let s = proj_s(at);
            (s - g * 0.5, s + g * 0.5)
        }
        BreakSpec::Between { from, to } => (proj_s(from), proj_s(to)),
    };
    let ((a0, a1), second) = open_break_spans(total, s1, s2, "spline")?;
    let sample_spline = |u0: f64, u1: f64| -> Object {
        let ta = t_at_s(u0);
        let tb = t_at_s(u1);
        let samples = 24usize;
        let fit_points: Vec<Point3> = (0..=samples)
            .map(|i| {
                let t = lerp(ta, tb, i as f64 / samples as f64);
                let q = curve.point(t);
                Point3::new(q[0], q[1], q[2])
            })
            .collect();
        Object::Spline(Spline {
            fit_points,
            control_points: Vec::new(),
            degree: 3,
            layer: spline.layer.clone(),
            properties: spline.properties.clone(),
        })
    };
    Ok(BreakResult {
        kept: sample_spline(a0, a1),
        split: second.map(|(b0, b1)| sample_spline(b0, b1)),
    })
}

// ---------------------------------------------------------------------------
// JOIN
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ChainSeg {
    a: V2,
    b: V2,
    bulge: f64,
    w: [f64; 2],
}

impl ChainSeg {
    fn reversed(self) -> Self {
        Self {
            a: self.b,
            b: self.a,
            bulge: -self.bulge,
            w: [self.w[1], self.w[0]],
        }
    }
}

#[derive(Clone)]
struct Chain {
    segs: Vec<ChainSeg>,
}

impl Chain {
    fn start(&self) -> V2 {
        self.segs[0].a
    }

    fn end(&self) -> V2 {
        self.segs.last().unwrap().b
    }

    fn reversed(&self) -> Self {
        Self {
            segs: self.segs.iter().rev().map(|s| s.reversed()).collect(),
        }
    }
}

/// Joins 2 or more objects (`line`, `arc`, open `polyline`) into a single
/// object inheriting `objects[0]`'s layer and properties.
///
/// - Collinear lines (even separated by a gap) merge into a single `Line`.
/// - Co-circular arcs merge into a single `Arc` (or `Circle` if they span 360°).
/// - Lines, arcs, and open polylines meeting end-to-end within `tolerance`
///   chain into a single `Polyline` (closing automatically when ends meet).
pub(crate) fn compute_join(objects: &[&Object], tolerance: f64) -> Result<Object, Error> {
    if objects.len() < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`join` requires at least two objects".to_owned(),
        ));
    }
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`tolerance` must be a non-negative length".to_owned(),
        ));
    }
    let tol = tolerance.max(EPS);

    // Check types and shared Z plane.
    let mut z_ref: Option<f64> = None;
    for (i, obj) in objects.iter().enumerate() {
        let z = match obj {
            Object::Line(l) => {
                if (l.from.z - l.to.z).abs() > Z_TOL {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] is not parallel to the XY plane"),
                    ));
                }
                l.from.z
            }
            Object::Arc(a) => a.center.z,
            Object::Polyline(p) => {
                if p.closed {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] is a closed polyline and cannot be joined"),
                    ));
                }
                if !p.is_flat() {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] is not a flat polyline"),
                    ));
                }
                if p.points.len() < 2 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] has fewer than two vertices"),
                    ));
                }
                p.points[0].z
            }
            other => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "`join` works on lines, arcs and open polylines, not a {} (targets[{i}])",
                        other.type_name()
                    ),
                ));
            }
        };
        if let Some(zr) = z_ref {
            if (z - zr).abs() > Z_TOL {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("targets[{i}] does not lie in the same Z plane as targets[0]"),
                ));
            }
        } else {
            z_ref = Some(z);
        }
    }
    let z = z_ref.unwrap_or(0.0);
    let layer = objects[0].layer().to_owned();
    let properties = objects[0].properties().clone();

    // Fast path 1: All lines and all collinear -> single merged Line (even across gaps).
    if objects.iter().all(|o| matches!(o, Object::Line(_))) {
        let lines: Vec<&Line> = objects
            .iter()
            .map(|o| match o {
                Object::Line(l) => l,
                _ => unreachable!(),
            })
            .collect();
        let a0 = V2::from_p3(lines[0].from);
        let b0 = V2::from_p3(lines[0].to);
        if let Some(u) = b0.sub(a0).unit() {
            let collinear = lines.iter().all(|l| {
                let p = V2::from_p3(l.from);
                let q = V2::from_p3(l.to);
                p.sub(a0).cross(u).abs() <= tol && q.sub(a0).cross(u).abs() <= tol
            });
            if collinear {
                let mut t_min = f64::INFINITY;
                let mut t_max = f64::NEG_INFINITY;
                for l in &lines {
                    for pt in [V2::from_p3(l.from), V2::from_p3(l.to)] {
                        let t = pt.sub(a0).dot(u);
                        t_min = t_min.min(t);
                        t_max = t_max.max(t);
                    }
                }
                if t_max - t_min <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "collinear lines collapse to zero length".to_owned(),
                    ));
                }
                return Ok(Object::Line(Line {
                    from: a0.add(u.scale(t_min)).to_p3(z),
                    to: a0.add(u.scale(t_max)).to_p3(z),
                    layer,
                    properties,
                }));
            }
        }
    }

    // Fast path 2: All arcs and all co-circular -> single merged Arc or Circle.
    if objects.iter().all(|o| matches!(o, Object::Arc(_))) {
        let arcs: Vec<&Arc> = objects
            .iter()
            .map(|o| match o {
                Object::Arc(a) => a,
                _ => unreachable!(),
            })
            .collect();
        let c0 = V2::from_p3(arcs[0].center);
        let r0 = arcs[0].radius;
        let cocircular = arcs
            .iter()
            .all(|a| V2::from_p3(a.center).dist(c0) <= tol && (a.radius - r0).abs() <= tol);
        if cocircular {
            return join_cocircular_arcs(&arcs, c0, r0, z, layer, properties, tol);
        }
    }

    // General path: end-to-end polyline chaining of lines, arcs, and open polylines.
    let mut chains: Vec<Chain> = Vec::with_capacity(objects.len());
    let mut any_widths = false;
    let mut uniform_width: Option<f64> = None;
    let mut widths_uniform = true;

    for (i, obj) in objects.iter().enumerate() {
        let segs = match obj {
            Object::Line(l) => {
                let a = V2::from_p3(l.from);
                let b = V2::from_p3(l.to);
                if a.dist(b) <= EPS {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] is a zero-length line"),
                    ));
                }
                vec![ChainSeg {
                    a,
                    b,
                    bulge: 0.0,
                    w: [0.0, 0.0],
                }]
            }
            Object::Arc(arc) => {
                let c = V2::from_p3(arc.center);
                let sweep = arc_sweep_deg(arc.start_angle, arc.end_angle);
                if sweep >= 360.0 - 1e-4 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("targets[{i}] is a full-circle arc and cannot be joined into a polyline"),
                    ));
                }
                let a = arc_point(c, arc.radius, arc.start_angle);
                let b = arc_point(c, arc.radius, arc.end_angle);
                let bulge = (sweep.to_radians() * 0.25).tan();
                vec![ChainSeg {
                    a,
                    b,
                    bulge,
                    w: [0.0, 0.0],
                }]
            }
            Object::Polyline(p) => {
                let mut v = Vec::with_capacity(p.points.len() - 1);
                for j in 0..p.points.len() - 1 {
                    let a = V2::from_p3(p.points[j]);
                    let b = V2::from_p3(p.points[j + 1]);
                    let w = p.widths_at(j);
                    if w[0] != 0.0 || w[1] != 0.0 {
                        any_widths = true;
                    }
                    v.push(ChainSeg {
                        a,
                        b,
                        bulge: p.bulge(j),
                        w,
                    });
                }
                v
            }
            _ => unreachable!(),
        };
        for s in &segs {
            if (s.w[0] - s.w[1]).abs() > EPS {
                widths_uniform = false;
            } else {
                match uniform_width {
                    None => uniform_width = Some(s.w[0]),
                    Some(uw) if (uw - s.w[0]).abs() > EPS => widths_uniform = false,
                    _ => {}
                }
            }
        }
        chains.push(Chain { segs });
    }

    let mut used = vec![false; chains.len()];
    let mut acc = chains[0].segs.clone();
    used[0] = true;
    let mut remaining = chains.len() - 1;

    while remaining > 0 {
        let head = acc[0].a;
        let tail = acc.last().unwrap().b;
        let mut matched: Option<(usize, bool, bool)> = None; // (idx, append_to_tail, reverse_piece)
        let mut best_dist = tol + 1.0;

        for (i, ch) in chains.iter().enumerate() {
            if used[i] {
                continue;
            }
            let candidates = [
                (ch.start().dist(tail), true, false),
                (ch.end().dist(tail), true, true),
                (ch.end().dist(head), false, false),
                (ch.start().dist(head), false, true),
            ];
            for (d, to_tail, rev) in candidates {
                if d <= tol && d < best_dist {
                    best_dist = d;
                    matched = Some((i, to_tail, rev));
                }
            }
        }

        let Some((idx, to_tail, rev)) = matched else {
            let missing: Vec<String> = (0..chains.len())
                .filter(|&i| !used[i])
                .map(|i| format!("targets[{i}]"))
                .collect();
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "cannot join: {} do{} not meet end to end within tolerance {tol}",
                    missing.join(", "),
                    if missing.len() == 1 { "es" } else { "" }
                ),
            ));
        };

        let mut piece = if rev {
            chains[idx].reversed().segs
        } else {
            chains[idx].segs.clone()
        };
        if to_tail {
            // Snap shared vertex exactly.
            let joint = acc.last().unwrap().b;
            piece[0].a = joint;
            acc.extend(piece);
        } else {
            let joint = acc[0].a;
            piece.last_mut().unwrap().b = joint;
            piece.extend(acc);
            acc = piece;
        }
        used[idx] = true;
        remaining -= 1;
    }

    let closed = acc.len() >= 2 && acc[0].a.dist(acc.last().unwrap().b) <= tol;
    let mut points: Vec<Point3> = acc.iter().map(|s| s.a.to_p3(z)).collect();
    if !closed {
        points.push(acc.last().unwrap().b.to_p3(z));
    }
    let mut bulges: Vec<f64> = acc.iter().map(|s| s.bulge).collect();
    if !closed {
        bulges.push(0.0);
    }
    let out_bulges = if bulges.iter().any(|b| b.abs() > 1e-12) {
        bulges
    } else {
        Vec::new()
    };
    let (out_width, out_widths) = if !any_widths {
        (0.0, Vec::new())
    } else if widths_uniform {
        (uniform_width.unwrap_or(0.0), Vec::new())
    } else {
        (0.0, acc.iter().map(|s| s.w).collect())
    };

    Ok(Object::Polyline(Polyline {
        points,
        bulges: out_bulges,
        widths: out_widths,
        width: out_width,
        closed,
        layer,
        properties,
    }))
}

fn join_cocircular_arcs(
    arcs: &[&Arc],
    center: V2,
    radius: f64,
    z: f64,
    layer: String,
    properties: crate::model::Properties,
    tol: f64,
) -> Result<Object, Error> {
    let ang_tol = (tol / radius.max(EPS)).to_degrees().max(1e-4);
    // Split each arc into non-wrapping intervals in [0, 360].
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for a in arcs {
        let s = norm_deg(a.start_angle);
        let sw = arc_sweep_deg(a.start_angle, a.end_angle);
        if s + sw <= 360.0 + 1e-9 {
            intervals.push((s, (s + sw).min(360.0)));
        } else {
            intervals.push((s, 360.0));
            intervals.push((0.0, (s + sw) - 360.0));
        }
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in intervals {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 + ang_tol {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    // Check wrap-around merge between last (ending at 360) and first (starting at 0).
    if merged.len() >= 2 && merged[0].0 <= ang_tol && 360.0 - merged.last().unwrap().1 <= ang_tol {
        let first = merged.remove(0);
        let last = merged.pop().unwrap();
        // Represent the wrapped merged span as (last.0, first.1 + 360).
        let span = (first.1 + 360.0) - last.0;
        if span >= 360.0 - ang_tol {
            return Ok(Object::Circle(Circle {
                center: center.to_p3(z),
                radius,
                layer,
                properties,
            }));
        }
        merged.push((last.0, first.1 + 360.0));
    }

    if merged.len() == 1 {
        let (s, e) = merged[0];
        if e - s >= 360.0 - ang_tol {
            return Ok(Object::Circle(Circle {
                center: center.to_p3(z),
                radius,
                layer,
                properties,
            }));
        }
        return Ok(Object::Arc(Arc {
            center: center.to_p3(z),
            radius,
            start_angle: norm_deg(s),
            end_angle: norm_deg(e),
            layer,
            properties,
        }));
    }

    // Multiple non-overlapping co-circular arcs (e.g., an arc broken with a gap
    // and joined back): keep the complementary arc leaving out the largest gap.
    let mut cyclic: Vec<(f64, f64)> = merged
        .into_iter()
        .map(|(s, e)| (norm_deg(s), (e - s).min(360.0)))
        .collect();
    cyclic.sort_by(|a, b| a.0.total_cmp(&b.0));
    let k = cyclic.len();
    let mut largest_gap = -1.0;
    let mut gap_after = 0usize;
    for i in 0..k {
        let end_i = (cyclic[i].0 + cyclic[i].1).rem_euclid(360.0);
        let start_next = cyclic[(i + 1) % k].0;
        let gap = (start_next - end_i).rem_euclid(360.0);
        if gap > largest_gap {
            largest_gap = gap;
            gap_after = i;
        }
    }
    let start_angle = cyclic[(gap_after + 1) % k].0;
    let end_angle = norm_deg(cyclic[gap_after].0 + cyclic[gap_after].1);
    let _ = PI;
    Ok(Object::Arc(Arc {
        center: center.to_p3(z),
        radius,
        start_angle,
        end_angle,
        layer,
        properties,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Properties;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }

    fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> Object {
        Object::Line(Line {
            from: Point3::new(x1, y1, 0.0),
            to: Point3::new(x2, y2, 0.0),
            layer: "wires".to_owned(),
            properties: Properties::default(),
        })
    }

    #[test]
    fn break_line_at_point_with_gap_and_between() {
        let w = line(0.0, 10.0, 100.0, 10.0);

        // BREAKATPOINT at x = 40 -> [0..40] and [40..100]
        let sp = compute_break(
            &w,
            BreakSpec::AtPoint {
                at: [40.0, 10.0, 0.0],
                gap: None,
            },
        )
        .unwrap();
        let Object::Line(l1) = sp.kept else { panic!() };
        let Some(Object::Line(l2)) = sp.split else {
            panic!()
        };
        close(l1.from.x, 0.0);
        close(l1.to.x, 40.0);
        close(l2.from.x, 40.0);
        close(l2.to.x, 100.0);

        // Centered gap of 6 around x = 50 -> [0..47] and [53..100]
        let gp = compute_break(
            &w,
            BreakSpec::AtPoint {
                at: [50.0, 10.0, 0.0],
                gap: Some(6.0),
            },
        )
        .unwrap();
        let Object::Line(g1) = gp.kept else { panic!() };
        let Some(Object::Line(g2)) = gp.split else {
            panic!()
        };
        close(g1.to.x, 47.0);
        close(g2.from.x, 53.0);

        // Two-point break [20, 10] to [30, 10] -> [0..20] and [30..100]
        let bw = compute_break(
            &w,
            BreakSpec::Between {
                from: [20.0, 10.0, 0.0],
                to: [30.0, 10.0, 0.0],
            },
        )
        .unwrap();
        let Object::Line(b1) = bw.kept else { panic!() };
        let Some(Object::Line(b2)) = bw.split else {
            panic!()
        };
        close(b1.to.x, 20.0);
        close(b2.from.x, 30.0);
    }

    #[test]
    fn break_circle_and_closed_polyline() {
        let c = Object::Circle(Circle {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 10.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        // Single point without gap refused on circle
        assert!(compute_break(
            &c,
            BreakSpec::AtPoint {
                at: [10.0, 0.0, 0.0],
                gap: None
            }
        )
        .is_err());

        // Between 0 deg and 90 deg removes 0..90, keeping 90..0 (270 deg arc)
        let bc = compute_break(
            &c,
            BreakSpec::Between {
                from: [10.0, 0.0, 0.0],
                to: [0.0, 10.0, 0.0],
            },
        )
        .unwrap();
        assert!(bc.split.is_none());
        let Object::Arc(arc) = bc.kept else { panic!() };
        close(arc.start_angle, 90.0);
        close(arc.end_angle, 0.0);

        // Closed square broken with gap 4 centered at (5, 0) -> open polyline from (7, 0) around to (3, 0)
        let sq = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(10.0, 10.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
            ],
            true,
            "0",
        ));
        let bsq = compute_break(
            &sq,
            BreakSpec::AtPoint {
                at: [5.0, 0.0, 0.0],
                gap: Some(4.0),
            },
        )
        .unwrap();
        let Object::Polyline(p) = bsq.kept else {
            panic!()
        };
        assert!(!p.closed);
        close(p.points[0].x, 7.0);
        close(p.points[0].y, 0.0);
        close(p.points.last().unwrap().x, 3.0);
        close(p.points.last().unwrap().y, 0.0);
    }

    #[test]
    fn join_collinear_lines_across_gap_cocircular_arcs_and_mixed_polyline() {
        // 1. Two collinear lines with a gap join back into one continuous line!
        let a = line(0.0, 10.0, 47.0, 10.0);
        let b = line(53.0, 10.0, 100.0, 10.0);
        let Object::Line(joined_line) = compute_join(&[&a, &b], DEFAULT_JOIN_TOLERANCE).unwrap()
        else {
            panic!()
        };
        close(joined_line.from.x, 0.0);
        close(joined_line.to.x, 100.0);

        // 2. Two semicircular co-circular arcs join into a Circle!
        let top = Object::Arc(Arc {
            center: Point3::new(5.0, 5.0, 0.0),
            radius: 10.0,
            start_angle: 0.0,
            end_angle: 180.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let bot = Object::Arc(Arc {
            center: Point3::new(5.0, 5.0, 0.0),
            radius: 10.0,
            start_angle: 180.0,
            end_angle: 0.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let Object::Circle(full) = compute_join(&[&top, &bot], DEFAULT_JOIN_TOLERANCE).unwrap()
        else {
            panic!()
        };
        close(full.center.x, 5.0);
        close(full.radius, 10.0);

        // 3. Out-of-order Line + Arc + Line forming an open U-bend join into one Polyline with bulge!
        let seg1 = line(0.0, 0.0, 10.0, 0.0);
        let seg3 = line(15.0, 15.0, 15.0, 5.0); // reversed direction on purpose!
        let bend = Object::Arc(Arc {
            center: Point3::new(10.0, 5.0, 0.0),
            radius: 5.0,
            start_angle: 270.0,
            end_angle: 0.0,
            layer: "wires".to_owned(),
            properties: Properties::default(),
        });
        let Object::Polyline(poly) =
            compute_join(&[&seg1, &seg3, &bend], DEFAULT_JOIN_TOLERANCE).unwrap()
        else {
            panic!()
        };
        assert!(!poly.closed);
        assert_eq!(poly.points.len(), 4);
        close(poly.points[0].x, 0.0);
        close(poly.points[1].x, 10.0);
        close(poly.points[2].x, 15.0);
        close(poly.points[2].y, 5.0);
        close(poly.points[3].x, 15.0);
        close(poly.points[3].y, 15.0);
        close(poly.bulge(1), (PI / 8.0).tan());

        // 4. Four lines forming a rectangle auto-close into a closed Polyline!
        let e1 = line(0.0, 0.0, 20.0, 0.0);
        let e2 = line(20.0, 0.0, 20.0, 10.0);
        let e3 = line(20.0, 10.0, 0.0, 10.0);
        let e4 = line(0.0, 10.0, 0.0, 0.0);
        let Object::Polyline(rect) =
            compute_join(&[&e1, &e3, &e2, &e4], DEFAULT_JOIN_TOLERANCE).unwrap()
        else {
            panic!()
        };
        assert!(rect.closed);
        assert_eq!(rect.points.len(), 4);
    }
}
